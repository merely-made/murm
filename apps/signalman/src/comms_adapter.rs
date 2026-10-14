//! LXMF correspondence over Signalman's existing message authority.
//!
//! The initial application profile is Signalman's versioned text payload, not
//! every stock LXMF body or field. The host owns durable queueing and radio
//! execution. This adapter projects that journal and never creates another one.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
pub use comms;
use comms::{
    AdapterError, Conversation, ConversationId, DeliveryStatus, Direction, Draft, Identity,
    Message, MessageBody, MessageId, ProtocolAdapter, ProtocolKind,
};

use crate::message::{MessageBook, MessageDirection, MessagePeer, MessageRecord};
use crate::message_view::MessageView;
mod diagnostics;
pub use diagnostics::{LxmfDiagnostics, LxmfFailure};

/// The application's single journal owner, shared with its existing face.
/// Queue success means the intent was persisted, never that it reached a peer.
pub trait LxmfMailbox: Send + Sync {
    fn book(&self) -> Result<Arc<MessageBook>, AdapterError>;
    fn queue_text(
        &self,
        local: MessagePeer,
        recipient: MessagePeer,
        text: &str,
    ) -> Result<crate::message::MessageId, AdapterError>;
    fn petname(&self, _peer: MessagePeer) -> Option<String> {
        None
    }
}

/// A concrete Comms backend scoped to one station's delivery destination.
pub struct LxmfAdapter<B> {
    local: MessagePeer,
    mailbox: B,
}

impl<B: LxmfMailbox> LxmfAdapter<B> {
    pub fn new(local: MessagePeer, mailbox: B) -> Self {
        Self { local, mailbox }
    }

    /// An explicitly addressed reply may start a thread before peer discovery.
    /// The local prefix prevents an old station's thread being routed as a new one.
    pub fn conversation_id(local: MessagePeer, peer: MessagePeer) -> ConversationId {
        ConversationId::new(
            ProtocolKind::Lxmf,
            format!(
                "{}/{}",
                hex::encode(local.destination),
                hex::encode(peer.destination)
            ),
        )
    }

    fn peer(&self, id: &ConversationId) -> Result<MessagePeer, AdapterError> {
        if id.protocol != ProtocolKind::Lxmf {
            return Err(AdapterError::NotFound);
        }
        let (local, peer) = id.key.split_once('/').ok_or(AdapterError::NotFound)?;
        if local != hex::encode(self.local.destination)
            || peer.len() != 32
            || peer.to_ascii_lowercase() != peer
        {
            return Err(AdapterError::NotFound);
        }
        let destination = hex::decode(peer)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(AdapterError::NotFound)?;
        Ok(MessagePeer::new(destination, None))
    }

    fn other(&self, record: &MessageRecord) -> Option<MessagePeer> {
        let (local, other) = match record.direction {
            MessageDirection::Outgoing => (record.message.sender(), record.message.recipient()),
            MessageDirection::Incoming => (record.message.recipient(), record.message.sender()),
        };
        let belongs = match record.direction {
            MessageDirection::Outgoing => local == self.local,
            MessageDirection::Incoming => {
                local.destination == self.local.destination
                    && local
                        .identity
                        .is_none_or(|key| self.local.identity == Some(key))
            }
        };
        belongs.then_some(other)
    }

    fn identity_for(&self, peer: MessagePeer) -> Identity {
        let mut identity = Identity::new(ProtocolKind::Lxmf, hex::encode(peer.destination));
        identity.display_name = self.mailbox.petname(peer);
        identity
    }
}

#[async_trait]
impl<B: LxmfMailbox> ProtocolAdapter for LxmfAdapter<B> {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Lxmf
    }
    fn identity(&self) -> Identity {
        self.identity_for(self.local)
    }

    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        let mut conversations: BTreeMap<String, Conversation> = BTreeMap::new();
        for record in self.mailbox.book()?.iter() {
            let Some(peer) = self.other(record) else {
                continue;
            };
            let id = Self::conversation_id(self.local, peer);
            let identity = self.identity_for(peer);
            let conversation =
                conversations
                    .entry(id.key.clone())
                    .or_insert_with(|| Conversation {
                        id,
                        title: identity.label().to_owned(),
                        participants: vec![self.identity(), identity],
                        last_activity_ms: None,
                        unread: 0,
                    });
            conversation.last_activity_ms = Some(
                conversation
                    .last_activity_ms
                    .unwrap_or(0)
                    .max(record.observed_unix_ms),
            );
        }
        Ok(conversations.into_values().collect())
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        let peer = self.peer(conversation)?;
        let mut messages = Vec::new();
        for record in self.mailbox.book()?.iter() {
            if self
                .other(record)
                .is_none_or(|other| other.destination != peer.destination)
            {
                continue;
            }
            let text = record.message.text().ok_or_else(|| AdapterError::Unsupported(
                "this Comms adapter presents Signalman text; voice remains in the Signalman voice face".into()))?;
            let timestamp = match &record.message {
                crate::message::Message::Text(text) => text.authored_unix_ms,
                crate::message::Message::Voice(_) => unreachable!("text checked above"),
            };
            messages.push(Message {
                id: MessageId(hex::encode(record.message.id().0)),
                author: self.identity_for(record.message.sender()),
                body: MessageBody::PlainText(text.to_owned()),
                subject: None,
                timestamp_ms: Some(timestamp),
                direction: match record.direction {
                    MessageDirection::Outgoing => Direction::Outgoing,
                    MessageDirection::Incoming => Direction::Incoming,
                },
            });
        }
        if messages.is_empty() {
            return Err(AdapterError::NotFound);
        }
        Ok(messages)
    }

    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let id = draft.conversation.as_ref().ok_or(AdapterError::NotFound)?;
        let peer = self.peer(id)?;
        if self.local.identity.is_none() {
            return Err(AdapterError::Unsupported(
                "a known station authoring identity is required".into(),
            ));
        }
        if draft.subject.is_some() {
            return Err(AdapterError::Unsupported(
                "Signalman's text profile does not carry subjects".into(),
            ));
        }
        if draft.is_empty() {
            return Err(AdapterError::Unsupported(
                "an empty radio text is not queued".into(),
            ));
        }
        Ok(MessageId(hex::encode(
            self.mailbox.queue_text(self.local, peer, &draft.body)?.0,
        )))
    }

    async fn delivery_status(
        &self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<DeliveryStatus, AdapterError> {
        let peer = self.peer(conversation)?;
        self.mailbox
            .book()?
            .iter()
            .find(|record| {
                self.other(record)
                    .is_some_and(|other| other.destination == peer.destination)
                    && hex::encode(record.message.id().0) == message.0
            })
            .map(|record| MessageView::new(record).delivery)
            .ok_or(AdapterError::NotFound)
    }
}

#[cfg(test)]
mod tests;
