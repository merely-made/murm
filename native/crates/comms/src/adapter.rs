// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The seam between the comms model and a backend.
//!
//! A [`ProtocolAdapter`] presents one backend (a misfin mailbox, a murm cabal
//! set) as conversations, messages, and an identity, and accepts a [`Draft`] to
//! send. [`Comms`](crate::Comms) holds a set of adapters and merges them into one
//! unified view, routing per-conversation operations to the owning adapter by
//! [`ProtocolKind`].
//!
//! The trait is `async` and object-safe (via `async_trait`) so the host can hold
//! a heterogeneous `Vec<Box<dyn ProtocolAdapter>>` and light each protocol up as
//! its backend matures.

use async_trait::async_trait;

use crate::model::{
    Conversation, ConversationId, Draft, Identity, Message, MessageId, ProtocolKind,
};

/// Why an adapter operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterError {
    /// The backend (network, store, protocol) failed. Carries its message.
    Backend(String),
    /// The addressed conversation or message does not exist.
    NotFound,
    /// The operation is not supported by this backend (e.g. sending on a
    /// receive-only protocol surface).
    Unsupported(String),
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::Backend(message) => write!(formatter, "comms backend error: {message}"),
            AdapterError::NotFound => write!(formatter, "comms target not found"),
            AdapterError::Unsupported(message) => {
                write!(formatter, "comms operation unsupported: {message}")
            }
        }
    }
}

impl std::error::Error for AdapterError {}

/// One backend, presented as comms conversations.
///
/// Implementors own a backend handle (a `MailboxStore`, a `CabalHandle`) and
/// shape its data into the host-neutral model. Methods that touch the network or
/// disk are `async`.
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Which protocol this adapter serves. [`Comms`](crate::Comms) routes by it.
    fn protocol(&self) -> ProtocolKind;

    /// This user's identity on this protocol (the "me" the pane shows).
    fn identity(&self) -> Identity;

    /// The conversations this backend currently holds, as list metadata. Order is
    /// the adapter's; [`Comms`](crate::Comms) re-sorts the merged set by recency.
    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError>;

    /// The messages in one conversation, in delivery order. `NotFound` if the id
    /// is unknown to this adapter.
    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError>;

    /// Send `draft`, returning the new message's id. `Unsupported` if this
    /// backend can't send (a receive-only surface), `NotFound` if the draft's
    /// target conversation is unknown.
    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError>;
}
