/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! [`Comms`] — the unified comms model over a set of backend adapters.
//!
//! Holds a `Vec<Box<dyn ProtocolAdapter>>` and presents them as one inbox:
//! [`inbox`](Comms::inbox) merges every adapter's conversations (recency-sorted)
//! and surfaces, rather than hides, any backend that failed;
//! [`messages`](Comms::messages) and [`send`](Comms::send) route to the owning
//! adapter by [`ProtocolKind`].

use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::model::{Conversation, ConversationId, Draft, Identity, Message, MessageId, ProtocolKind};

/// A backend that failed during [`Comms::inbox`], paired with its protocol so the
/// pane can show which one is down without dropping the conversations that loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterFailure {
    pub protocol: ProtocolKind,
    pub error: AdapterError,
}

/// The merged conversation list plus any per-backend failures. Failures are
/// reported, not swallowed: one backend being down degrades to the others rather
/// than blanking the inbox or hiding the problem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inbox {
    pub conversations: Vec<Conversation>,
    pub failures: Vec<AdapterFailure>,
}

/// The unified comms model. Construct with [`Comms::new`] and add backends with
/// [`with_adapter`](Comms::with_adapter) / [`add_adapter`](Comms::add_adapter).
#[derive(Default)]
pub struct Comms {
    adapters: Vec<Box<dyn ProtocolAdapter>>,
}

impl Comms {
    /// An empty model with no backends.
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    /// Add a backend adapter (builder style).
    pub fn with_adapter(mut self, adapter: Box<dyn ProtocolAdapter>) -> Self {
        self.adapters.push(adapter);
        self
    }

    /// Add a backend adapter.
    pub fn add_adapter(&mut self, adapter: Box<dyn ProtocolAdapter>) {
        self.adapters.push(adapter);
    }

    /// This user's identity on each backend (one per adapter, in add order) — the
    /// "who am I" the pane shows per protocol.
    pub fn identities(&self) -> Vec<Identity> {
        self.adapters
            .iter()
            .map(|adapter| adapter.identity())
            .collect()
    }

    /// The merged inbox: every backend's conversations, sorted most-recent first
    /// (conversations with no timestamp sort last). A backend that errors
    /// contributes an [`AdapterFailure`] instead of its conversations.
    pub async fn inbox(&self) -> Inbox {
        let mut conversations = Vec::new();
        let mut failures = Vec::new();
        for adapter in &self.adapters {
            match adapter.conversations().await {
                Ok(mut found) => conversations.append(&mut found),
                Err(error) => failures.push(AdapterFailure {
                    protocol: adapter.protocol(),
                    error,
                }),
            }
        }
        // Most recent first; `None` (no activity yet) sorts to the end.
        conversations.sort_by(|a, b| b.last_activity_ms.cmp(&a.last_activity_ms));
        Inbox {
            conversations,
            failures,
        }
    }

    /// The messages in one conversation, routed to the backend that owns it.
    /// `Unsupported` if no adapter serves the conversation's protocol.
    pub async fn messages(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<Message>, AdapterError> {
        self.adapter_for(conversation.protocol)?
            .messages(conversation)
            .await
    }

    /// Send a draft, routed to the backend that owns its target conversation.
    /// Errors if the draft has no target, or no adapter serves its protocol.
    pub async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let conversation = draft.conversation.as_ref().ok_or_else(|| {
            AdapterError::Unsupported("draft has no target conversation".to_string())
        })?;
        self.adapter_for(conversation.protocol)?.send(draft).await
    }

    /// The adapter serving `protocol`, or `Unsupported` if none is registered.
    fn adapter_for(&self, protocol: ProtocolKind) -> Result<&dyn ProtocolAdapter, AdapterError> {
        self.adapters
            .iter()
            .find(|adapter| adapter.protocol() == protocol)
            .map(|adapter| adapter.as_ref())
            .ok_or_else(|| {
                AdapterError::Unsupported(format!("no adapter registered for {protocol:?}"))
            })
    }
}
