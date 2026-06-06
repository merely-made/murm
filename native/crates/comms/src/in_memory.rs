/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! An in-memory [`ProtocolAdapter`] for tests and prototyping.
//!
//! Backed by plain `Vec`s, no network or disk. Doubles as the executable
//! reference for the trait. Enabled for this crate's tests automatically and for
//! downstream crates via the `test-support` feature.

use async_trait::async_trait;

use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::model::{
    Conversation, ConversationId, Direction, Draft, Identity, Message, MessageBody, MessageId,
    ProtocolKind,
};

/// A fake backend: a fixed identity, a set of conversations, and their messages.
pub struct InMemoryAdapter {
    protocol: ProtocolKind,
    identity: Identity,
    conversations: Vec<Conversation>,
    messages: Vec<(ConversationId, Vec<Message>)>,
    /// When false, `send` reports `Unsupported` (to model receive-only backends).
    can_send: bool,
}

impl InMemoryAdapter {
    /// A sending-capable adapter for `protocol` with the given `identity`.
    pub fn new(protocol: ProtocolKind, identity: Identity) -> Self {
        Self {
            protocol,
            identity,
            conversations: Vec::new(),
            messages: Vec::new(),
            can_send: true,
        }
    }

    /// Make this adapter receive-only (`send` returns `Unsupported`).
    pub fn receive_only(mut self) -> Self {
        self.can_send = false;
        self
    }

    /// Add a conversation and its messages.
    pub fn with_conversation(mut self, conversation: Conversation, messages: Vec<Message>) -> Self {
        self.messages
            .push((conversation.id.clone(), messages));
        self.conversations.push(conversation);
        self
    }
}

#[async_trait]
impl ProtocolAdapter for InMemoryAdapter {
    fn protocol(&self) -> ProtocolKind {
        self.protocol
    }

    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        Ok(self.conversations.clone())
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        self.messages
            .iter()
            .find(|(id, _)| id == conversation)
            .map(|(_, messages)| messages.clone())
            .ok_or(AdapterError::NotFound)
    }

    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        if !self.can_send {
            return Err(AdapterError::Unsupported("receive-only backend".to_string()));
        }
        let conversation = draft
            .conversation
            .as_ref()
            .ok_or(AdapterError::NotFound)?;
        if !self.conversations.iter().any(|c| &c.id == conversation) {
            return Err(AdapterError::NotFound);
        }
        // A deterministic stand-in id; a real backend returns its content hash.
        Ok(MessageId(format!("{}:{}", conversation.key, self.identity.address)))
    }
}

/// A trivial incoming text message, for building fixtures.
pub fn sample_message(
    id: &str,
    author: Identity,
    body: &str,
    timestamp_ms: u64,
) -> Message {
    Message {
        id: MessageId(id.to_string()),
        author,
        body: MessageBody::PlainText(body.to_string()),
        subject: None,
        timestamp_ms: Some(timestamp_ms),
        direction: Direction::Incoming,
    }
}
