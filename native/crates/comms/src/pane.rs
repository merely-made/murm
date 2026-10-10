// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The comms pane's view state — the host-neutral view-model a host renders, the
//! way a host renders `chrome`'s `ToolbarState`.
//!
//! [`CommsPane`] holds the dock geometry, the conversation-list snapshot, the
//! open thread, and the draft. The host fills the list / thread from
//! [`Comms`](crate::Comms) (async) and renders the pane; the pane owns the view
//! mutations (toggle, select, compose). It reaches no backend itself.
//!
//! [`DockState`] / [`DockSide`] are the minimal dock contract this first pane
//! needs (per the peripheral-panes architecture). They graduate to shared dock
//! infrastructure when a second pane lands; built here, scoped to what comms uses,
//! rather than as a speculative catalog.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::comms::{AdapterFailure, Inbox};
use crate::model::{Conversation, ConversationId, Draft, Message, ProtocolKind};

/// The edge a pane docks to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockSide {
    /// Docked to the left edge (vertical strip).
    Left,
    /// Docked to the right edge (vertical strip). The comms default.
    #[default]
    Right,
    /// Docked to the bottom edge (horizontal strip).
    Bottom,
}

impl DockSide {
    /// Whether this side is a vertical strip (its [`DockState::size`] is a width).
    pub fn is_vertical(self) -> bool {
        matches!(self, DockSide::Left | DockSide::Right)
    }
}

/// Minimal dock geometry shared by peripheral panes: whether the pane is open,
/// which edge it hangs on, its size along the docking axis, and whether it holds
/// input focus.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockState {
    /// Whether the pane is shown.
    pub open: bool,
    /// The edge the pane docks to.
    pub side: DockSide,
    /// Width (for a left/right dock) or height (for a bottom dock), in px.
    pub size: f32,
    /// Whether the pane currently holds input focus.
    pub focused: bool,
}

impl Default for DockState {
    fn default() -> Self {
        Self {
            open: false,
            side: DockSide::Right,
            size: 360.0,
            focused: false,
        }
    }
}

/// The compose-new form's state: which protocol to send over, the recipient
/// (a misfin `mailbox@host`; ignored for murm, which targets the cabal), and the
/// body. The host owns the live editing buffers and syncs them in on send.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewMessageForm {
    /// The protocol the new message is sent over.
    pub protocol: ProtocolKind,
    /// The recipient address (misfin `mailbox@host`). Unused for murm.
    pub to: String,
    /// The message body.
    pub body: String,
}

/// A local thread-load request. Its identity binds a completion to the pane's
/// latest request, including a reload of the same conversation. Never durable.
#[derive(Clone, Debug)]
pub struct ThreadRequest {
    conversation: ConversationId,
    identity: Arc<()>,
}

impl ThreadRequest {
    pub fn conversation(&self) -> &ConversationId {
        &self.conversation
    }
}

impl PartialEq for ThreadRequest {
    fn eq(&self, other: &Self) -> bool {
        self.conversation == other.conversation && Arc::ptr_eq(&self.identity, &other.identity)
    }
}

/// The comms pane's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommsPane {
    /// Drafts belong to this view, separately from its current attention.
    drafts: HashMap<ConversationId, Draft>,
    pending_thread: Option<ThreadRequest>,
    /// Dock geometry (open / side / size / focus).
    pub dock: DockState,
    /// The conversation-list snapshot, recency-sorted, as the host last loaded it.
    pub inbox: Vec<Conversation>,
    /// Backends that failed the last load, surfaced (not hidden) in the pane.
    pub failures: Vec<AdapterFailure>,
    /// The selected conversation, or `None` for the list-only view.
    pub selected: Option<ConversationId>,
    /// The selected conversation's messages, as the host last loaded them.
    pub thread: Vec<Message>,
    /// The reply being composed for the selected conversation.
    pub draft: Draft,
    /// A transient one-line outcome of the last send (e.g. "Delivered", "Mailbox
    /// doesn't exist", "Send failed: …"), shown under the compose box. Cleared on
    /// selecting another conversation. Useful where a send has no in-thread echo
    /// (misfin: sent mail isn't kept locally).
    pub send_status: Option<String>,
    /// The open compose-new form, or `None` when not composing a new message.
    pub new_message: Option<NewMessageForm>,
    /// This install's misfin receive address (`me@<host>`), surfaced so the user
    /// knows where peers send to reach them. `None` until the host reports it.
    pub misfin_address: Option<String>,
    /// This install's cabal join ticket, to share with a peer so they can join the
    /// cabal. `None` until the networked cabal is up.
    pub cabal_ticket: Option<String>,
}

impl CommsPane {
    /// A closed comms pane with defaults (right dock, 360px).
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the pane is open.
    pub fn is_open(&self) -> bool {
        self.dock.open
    }

    /// Toggle the pane open/closed. Opening focuses it; closing drops focus.
    pub fn toggle(&mut self) {
        if self.dock.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Open the pane and give it focus.
    pub fn open(&mut self) {
        self.dock.open = true;
        self.dock.focused = true;
    }

    /// Close the pane and drop focus. View state (selection, thread, draft) is
    /// kept so reopening returns where you were.
    pub fn close(&mut self) {
        self.dock.open = false;
        self.dock.focused = false;
    }

    /// Replace the conversation list + failures from a freshly loaded
    /// [`Inbox`](crate::Inbox). Keeps the current selection if it still exists,
    /// clearing it (and the thread) otherwise.
    pub fn set_inbox(&mut self, inbox: Inbox) {
        self.inbox = inbox.conversations;
        self.failures = inbox.failures;
        if let Some(selected) = &self.selected
            && !self.inbox.iter().any(|c| &c.id == selected)
        {
            self.clear_selection();
        }
    }

    /// Select a conversation, preserving the prior draft and restoring this
    /// conversation's draft. Selecting it again leaves its edits and thread alone.
    pub fn select(&mut self, id: ConversationId) {
        if self.selected.as_ref() == Some(&id) {
            return;
        }
        self.park_draft();
        self.draft = self
            .drafts
            .remove(&id)
            .unwrap_or_else(|| Draft::reply_to(id.clone()));
        self.selected = Some(id);
        self.thread.clear();
        self.pending_thread = None;
        self.send_status = None;
    }

    /// Return to the list-only view. Deselecting preserves edits locally.
    pub fn clear_selection(&mut self) {
        self.park_draft();
        self.selected = None;
        self.thread.clear();
        self.pending_thread = None;
        self.draft = Draft::default();
        self.send_status = None;
    }

    fn park_draft(&mut self) {
        if let Some(id) = self.draft.conversation.clone() {
            self.drafts.insert(id, self.draft.clone());
        }
    }

    /// Set the transient send-outcome line shown under the compose box.
    pub fn set_send_status(&mut self, status: impl Into<String>) {
        self.send_status = Some(status.into());
    }

    /// Open the compose-new form (a fresh form; misfin by default), dropping any
    /// open conversation so the form takes the pane.
    pub fn open_new_message(&mut self) {
        self.clear_selection();
        self.new_message = Some(NewMessageForm::default());
    }

    /// Close the compose-new form.
    pub fn close_new_message(&mut self) {
        self.new_message = None;
    }

    /// Whether the compose-new form is open.
    pub fn new_message_open(&self) -> bool {
        self.new_message.is_some()
    }

    /// Set the protocol on the open compose-new form (a no-op when closed).
    pub fn set_new_protocol(&mut self, protocol: ProtocolKind) {
        if let Some(form) = self.new_message.as_mut() {
            form.protocol = protocol;
        }
    }

    /// Record this install's connect info from the host: the misfin receive address
    /// and (when up) the cabal join ticket.
    pub fn set_identity(&mut self, misfin_address: String, cabal_ticket: Option<String>) {
        self.misfin_address = Some(misfin_address);
        self.cabal_ticket = cabal_ticket;
    }

    /// Explicitly unbind this identity and forget its transient compose state.
    /// A temporary transport outage should preserve drafts and report a failure.
    pub fn clear_identity(&mut self) {
        self.misfin_address = None;
        self.cabal_ticket = None;
        self.send_status = None;
        self.clear_selection();
        self.drafts.clear();
        self.close_new_message();
    }

    /// The currently selected conversation id, if any.
    pub fn selected(&self) -> Option<&ConversationId> {
        self.selected.as_ref()
    }

    /// Begin a load for the selected conversation. A newer request supersedes
    /// the old one, even when its conversation is the same.
    pub fn begin_thread_load(&mut self) -> Option<ThreadRequest> {
        let request = ThreadRequest {
            conversation: self.selected.clone()?,
            identity: Arc::new(()),
        };
        self.pending_thread = Some(request.clone());
        Some(request)
    }

    /// Apply only the latest requested thread. A late completion cannot attach
    /// to a different selection, a reopened selection or another pane.
    pub fn set_thread(&mut self, request: &ThreadRequest, messages: Vec<Message>) -> bool {
        if self.pending_thread.as_ref() != Some(request)
            || self.selected.as_ref() != Some(request.conversation())
        {
            return false;
        }
        self.thread = messages;
        self.pending_thread = None;
        true
    }

    /// The selected conversation's metadata from the loaded list, if present.
    pub fn selected_conversation(&self) -> Option<&Conversation> {
        let id = self.selected.as_ref()?;
        self.inbox.iter().find(|c| &c.id == id)
    }

    /// Set the committed draft body (the host syncs its live editor buffer in).
    pub fn set_draft_body(&mut self, body: impl Into<String>) {
        self.draft.body = body.into();
    }

    /// Clear the draft body after a successful send, keeping it aimed at the
    /// selected conversation.
    pub fn clear_draft(&mut self) {
        self.draft.body.clear();
        self.draft.subject = None;
    }

    /// Acknowledge an exact submitted draft after the owner confirms success.
    /// Preserve newer edits and a different selected conversation. Failures do
    /// not call this method. This records no transport or read-receipt claim.
    pub fn acknowledge_sent(&mut self, submitted: &Draft) -> bool {
        let Some(id) = submitted.conversation.as_ref() else {
            return false;
        };
        if self.draft.conversation.as_ref() == Some(id) {
            if self.draft != *submitted {
                return false;
            }
            self.clear_draft();
            return true;
        }
        if self.drafts.get(id) == Some(submitted) {
            self.drafts.remove(id);
            return true;
        }
        false
    }

    /// Whether the draft is ready to send (has a target and non-empty body).
    pub fn can_send(&self) -> bool {
        self.draft.conversation.is_some() && !self.draft.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comms::AdapterFailure;
    use crate::model::{Direction, Identity, MessageBody, MessageId, ProtocolKind};

    fn conversation(key: &str, last: u64) -> Conversation {
        Conversation {
            id: ConversationId::new(ProtocolKind::Murm, key),
            title: key.to_string(),
            participants: Vec::new(),
            last_activity_ms: Some(last),
            unread: 0,
        }
    }

    fn message(id: &str, body: &str) -> Message {
        Message {
            id: MessageId(id.to_string()),
            author: Identity::new(ProtocolKind::Murm, "peer"),
            body: MessageBody::PlainText(body.to_string()),
            subject: None,
            timestamp_ms: Some(1),
            direction: Direction::Incoming,
        }
    }

    #[test]
    fn toggle_opens_and_closes_with_focus() {
        let mut pane = CommsPane::new();
        assert!(!pane.is_open());
        pane.toggle();
        assert!(pane.is_open());
        assert!(pane.dock.focused);
        pane.toggle();
        assert!(!pane.is_open());
        assert!(!pane.dock.focused);
    }

    #[test]
    fn select_aims_the_draft_and_clears_the_prior_thread() {
        let mut pane = CommsPane::new();
        assert!(pane.begin_thread_load().is_none());
        assert!(pane.thread.is_empty());

        let id = ConversationId::new(ProtocolKind::Murm, "abc");
        pane.select(id.clone());
        assert_eq!(pane.selected(), Some(&id));
        assert_eq!(pane.draft.conversation, Some(id));
        assert!(pane.thread.is_empty());

        let request = pane.begin_thread_load().unwrap();
        assert!(pane.set_thread(&request, vec![message("m1", "hi")]));
        assert_eq!(pane.thread.len(), 1);
    }

    #[test]
    fn set_inbox_drops_a_selection_that_vanished() {
        let mut pane = CommsPane::new();
        pane.select(ConversationId::new(ProtocolKind::Murm, "gone"));
        pane.set_inbox(Inbox {
            conversations: vec![conversation("still-here", 10)],
            failures: Vec::new(),
        });
        assert!(
            pane.selected().is_none(),
            "the vanished selection is cleared"
        );
        assert!(pane.thread.is_empty());
    }

    #[test]
    fn set_inbox_keeps_a_selection_that_remains() {
        let mut pane = CommsPane::new();
        let id = ConversationId::new(ProtocolKind::Murm, "keep");
        pane.select(id.clone());
        pane.set_inbox(Inbox {
            conversations: vec![conversation("keep", 10), conversation("other", 5)],
            failures: Vec::new(),
        });
        assert_eq!(pane.selected(), Some(&id));
        assert_eq!(
            pane.selected_conversation().map(|c| c.title.as_str()),
            Some("keep")
        );
    }

    #[test]
    fn inbox_failures_are_surfaced() {
        let mut pane = CommsPane::new();
        pane.set_inbox(Inbox {
            conversations: Vec::new(),
            failures: vec![AdapterFailure {
                protocol: ProtocolKind::Misfin,
                error: crate::AdapterError::Backend("down".to_string()),
            }],
        });
        assert_eq!(pane.failures.len(), 1);
        assert_eq!(pane.failures[0].protocol, ProtocolKind::Misfin);
    }

    #[test]
    fn draft_send_readiness_and_clear() {
        let mut pane = CommsPane::new();
        assert!(!pane.can_send(), "no target, no body");
        pane.select(ConversationId::new(ProtocolKind::Murm, "abc"));
        assert!(!pane.can_send(), "target but empty body");
        pane.set_draft_body("hello");
        assert!(pane.can_send());
        pane.clear_draft();
        assert!(!pane.can_send(), "body cleared after send");
        assert_eq!(
            pane.draft.conversation,
            Some(ConversationId::new(ProtocolKind::Murm, "abc"))
        );
    }

    #[test]
    fn attention_changes_preserve_each_draft_and_identity_clear_forgets_them() {
        let mut pane = CommsPane::new();
        let a = conversation("a", 1).id;
        let b = conversation("b", 2).id;
        pane.select(a.clone());
        pane.set_draft_body("Alice's draft");
        pane.draft.subject = Some("subject".into());
        pane.select(a.clone());
        assert_eq!(pane.draft.body, "Alice's draft");
        pane.select(b.clone());
        pane.set_draft_body("Bob's draft");
        pane.clear_selection();
        pane.select(a.clone());
        assert_eq!(pane.draft.body, "Alice's draft");
        assert_eq!(pane.draft.subject.as_deref(), Some("subject"));
        pane.set_inbox(Inbox {
            conversations: vec![],
            failures: vec![],
        });
        pane.select(b);
        assert_eq!(pane.draft.body, "Bob's draft");
        pane.clear_identity();
        pane.select(a);
        assert!(pane.draft.is_empty());
    }

    #[test]
    fn delayed_thread_loads_cannot_attach_after_switch_reload_or_to_another_pane() {
        let a = conversation("a", 1).id;
        let mut pane = CommsPane::new();
        pane.select(a.clone());
        let old = pane.begin_thread_load().unwrap();
        pane.select(conversation("b", 2).id);
        pane.select(a.clone());
        assert!(!pane.set_thread(&old, vec![message("x", "stale")]));
        let first = pane.begin_thread_load().unwrap();
        let last = pane.begin_thread_load().unwrap();
        assert!(!pane.set_thread(&first, vec![message("x", "stale")]));
        let mut other = CommsPane::new();
        other.select(a);
        other.begin_thread_load();
        assert!(!other.set_thread(&last, vec![message("x", "wrong pane")]));
        assert!(pane.set_thread(&last, vec![message("y", "fresh")]));
        assert!(!pane.set_thread(&last, vec![]));
    }

    #[test]
    fn delayed_send_acknowledgment_preserves_other_and_newer_edits() {
        let mut pane = CommsPane::new();
        let a = conversation("a", 1).id;
        let b = conversation("b", 2).id;
        pane.select(a.clone());
        pane.set_draft_body("submitted");
        let sent = pane.draft.clone();
        pane.select(b);
        pane.set_draft_body("other");
        assert!(pane.acknowledge_sent(&sent));
        assert_eq!(pane.draft.body, "other");
        pane.select(a);
        assert!(pane.draft.is_empty());
        pane.set_draft_body("newer edit");
        assert!(!pane.acknowledge_sent(&sent));
        assert_eq!(pane.draft.body, "newer edit");
    }
}
