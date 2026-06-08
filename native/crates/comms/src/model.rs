/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The host-neutral comms model: the shapes the pane renders, independent of any
//! backend. A [`ProtocolAdapter`](crate::ProtocolAdapter) turns a concrete
//! backend (a misfin mailbox, a murm cabal) into these.
//!
//! The split mirrors mail: a [`Conversation`] is lightweight metadata for the
//! list, and [`Message`]s are loaded on demand when a conversation is opened.

use serde::{Deserialize, Serialize};

/// Which backend a conversation, identity, or message belongs to. Routing keys
/// off this; new protocols (the mooting family) add variants here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtocolKind {
    /// Misfin (gemini-style mail). The default for a fresh new-message form.
    #[default]
    Misfin,
    /// Murm (Cable bilateral / small-group cabals).
    Murm,
}

/// A party in a conversation — you or a peer. `address` is the backend-native
/// identifier (a misfin `mailbox@host`, a murm author key as hex); `display_name`
/// is the human label when one is known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub protocol: ProtocolKind,
    pub address: String,
    pub display_name: Option<String>,
}

impl Identity {
    /// Construct an identity.
    pub fn new(protocol: ProtocolKind, address: impl Into<String>) -> Self {
        Self {
            protocol,
            address: address.into(),
            display_name: None,
        }
    }

    /// Set the display name.
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// The best label to show: the display name if known, else the address.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.address)
    }
}

/// A stable, protocol-qualified conversation identifier. `key` is the backend's
/// own handle for the thread: a misfin correspondent addr-spec, a murm cabal id
/// (hex). Routing uses `protocol`; the owning adapter interprets `key`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId {
    pub protocol: ProtocolKind,
    pub key: String,
}

impl ConversationId {
    /// Construct a conversation id.
    pub fn new(protocol: ProtocolKind, key: impl Into<String>) -> Self {
        Self {
            protocol,
            key: key.into(),
        }
    }
}

/// A message identifier, unique within its conversation. Opaque and
/// adapter-defined (a misfin sequence, a murm post id as hex) so consumers can
/// dedup and address messages without knowing the backend.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

/// Whether a message was received or sent by this user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Received from a peer.
    Incoming,
    /// Authored and sent by this user.
    Outgoing,
}

/// A message body plus the hint a renderer needs. Gemtext rides the same nematic
/// engine the content card uses; plain text is shown as-is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageBody {
    /// Gemtext (`text/gemini`) — misfin gemmail bodies; render with nematic.
    Gemtext(String),
    /// Plain text — murm post text.
    PlainText(String),
}

impl MessageBody {
    /// The raw body text, whatever the format.
    pub fn text(&self) -> &str {
        match self {
            MessageBody::Gemtext(text) | MessageBody::PlainText(text) => text,
        }
    }
}

/// One message in a conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub author: Identity,
    pub body: MessageBody,
    /// A subject line when the backend carries one (misfin gemmail); `None` for
    /// backends without subjects (murm).
    pub subject: Option<String>,
    /// Delivery / authoring time, **unix milliseconds**. Adapters normalize their
    /// native units (misfin seconds, murm milliseconds) to ms. `None` when the
    /// backend gives no timestamp.
    pub timestamp_ms: Option<u64>,
    pub direction: Direction,
}

/// Lightweight metadata for the conversation list. Messages are loaded on demand
/// via [`ProtocolAdapter::messages`](crate::ProtocolAdapter::messages), so
/// building the list stays cheap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    /// The label for the list row: a correspondent address, a cabal name.
    pub title: String,
    /// Known participants (excluding or including self is the adapter's call;
    /// document per adapter). May be empty when the backend doesn't enumerate
    /// membership.
    pub participants: Vec<Identity>,
    /// Most recent activity, unix milliseconds, for recency sorting. `None` sorts
    /// last.
    pub last_activity_ms: Option<u64>,
    /// Unread message count, when the backend tracks read state; `0` otherwise.
    pub unread: usize,
}

/// A message being composed for `conversation`. The host owns edit state; the
/// adapter consumes a finished draft in [`ProtocolAdapter::send`](crate::ProtocolAdapter::send).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// The target conversation. (New-conversation compose, where the target is an
    /// address rather than an existing thread, is a later addition.)
    pub conversation: Option<ConversationId>,
    /// The body text being written.
    pub body: String,
    /// An optional subject, for backends that carry one (misfin).
    pub subject: Option<String>,
}

impl Draft {
    /// Start a draft replying into an existing conversation.
    pub fn reply_to(conversation: ConversationId) -> Self {
        Self {
            conversation: Some(conversation),
            body: String::new(),
            subject: None,
        }
    }

    /// Whether the draft has any body to send.
    pub fn is_empty(&self) -> bool {
        self.body.trim().is_empty()
    }
}
