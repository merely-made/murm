// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The host-neutral comms model: the shapes the pane renders, independent of any
//! backend. A [`ProtocolAdapter`](crate::ProtocolAdapter) turns a concrete
//! backend (a misfin mailbox, a murm cabal) into these.
//!
//! The split mirrors mail: a [`Conversation`] is lightweight metadata for the
//! list, and [`Message`]s are loaded on demand when a conversation is opened.

use serde::{Deserialize, Serialize};

/// Which protocol a conversation, identity, or message belongs to. Conversation
/// routing also uses the local account qualifier carried by [`ConversationId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtocolKind {
    /// Misfin (gemini-style mail). The default for a fresh new-message form.
    #[default]
    Misfin,
    /// Murm's invited conversations (murmurs).
    Murm,
    /// Commons' retained encrypted chat, projected under Moot authority.
    /// This names the record grammar, independently of its carriage.
    CommonsChat,
    /// LXMF correspondence, with application-owned history and delivery facts.
    Lxmf,
    /// Meshtastic channel text, addressed by node number within a local realm.
    Meshtastic,
    /// MeshCore private text, addressed by full public key within a local realm.
    MeshCore,
}

/// Stable identity for one local account/store served by a protocol adapter.
/// This is routing metadata; it never replaces a backend's native conversation key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub String);

impl AccountId {
    /// Construct a stable account identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<String> for AccountId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AccountId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
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

/// A stable, protocol and optionally account-qualified conversation identifier.
/// `key` is the backend's own handle for the thread: a misfin correspondent
/// addr-spec or an exact murm cabal/channel address. Account scope is routing
/// metadata and never changes that backend-native key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId {
    pub protocol: ProtocolKind,
    pub key: String,
    /// The local adapter account that owns this conversation. Missing IDs are
    /// legacy/unqualified IDs and may only route when exactly one adapter serves
    /// `protocol`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
}

impl ConversationId {
    /// Construct a conversation id.
    pub fn new(protocol: ProtocolKind, key: impl Into<String>) -> Self {
        Self {
            protocol,
            key: key.into(),
            account_id: None,
        }
    }

    /// Construct a conversation ID qualified for one local account.
    pub fn for_account(
        protocol: ProtocolKind,
        account_id: impl Into<AccountId>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            protocol,
            key: key.into(),
            account_id: Some(account_id.into()),
        }
    }

    /// Attach the owning account while preserving the backend-native key.
    pub fn with_account_id(mut self, account_id: impl Into<AccountId>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    /// Remove routing metadata before passing the native ID to an adapter.
    pub(crate) fn unqualified(&self) -> Self {
        Self {
            protocol: self.protocol,
            key: self.key.clone(),
            account_id: None,
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

/// Why an outgoing message remains queued before it reaches a radio or other
/// carriage boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryQueueReason {
    /// No currently usable carrier is available.
    Offline,
    /// The message is ready for a station or other carrier to accept it.
    ReadyForCarriage,
    /// Carriage requires a peer that is not presently available.
    WaitingForPeer,
}

/// The latest delivery fact known for one message.
///
/// This is a projection value, not a read receipt. In particular, [`Unknown`](Self::Unknown)
/// means the available records do not establish delivery state; it says nothing about whether a
/// recipient has read the message. Transport-specific records retain any identifiers and modes
/// needed to substantiate these facts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    /// Available records do not establish a delivery state.
    #[default]
    Unknown,
    /// The message remains local pending carriage.
    Queued(DeliveryQueueReason),
    /// A radio carrier accepted the message for transmission.
    HandedToRadio,
    /// A propagation node accepted the message.
    AcceptedByPropagationNode,
    /// The message was fetched from a propagation node.
    FetchedFromPropagationNode,
    /// The message arrived through a direct exchange.
    ReceivedDirect,
    /// The sender cancelled the outstanding message.
    Cancelled,
    /// Delivery failed with the preserved backend detail.
    Failed { detail: String },
    /// A text packet was received by the local radio protocol owner. This does
    /// not establish a direct route or authenticate the displayed sender.
    ReceivedByRadio,
    /// A backend matched a protocol acknowledgment. Its owner retains the
    /// exact ACK and integrity limitations; this is not a read receipt.
    ProtocolAcknowledged,
}

impl DeliveryStatus {
    /// A short, stable presentation label for the latest known delivery fact.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "delivery unknown",
            Self::Queued(DeliveryQueueReason::Offline) => "offline, queued",
            Self::Queued(DeliveryQueueReason::ReadyForCarriage) => "queued for station",
            Self::Queued(DeliveryQueueReason::WaitingForPeer) => "queued, waiting for peer",
            Self::HandedToRadio => "handed to radio",
            Self::AcceptedByPropagationNode => "accepted by propagation node",
            Self::FetchedFromPropagationNode => "fetched from propagation node",
            Self::ReceivedDirect => "received directly",
            Self::Cancelled => "cancelled",
            Self::Failed { .. } => "failed",
            Self::ReceivedByRadio => "received by radio",
            Self::ProtocolAcknowledged => "protocol acknowledgment observed",
        }
    }
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
