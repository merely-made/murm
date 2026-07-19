// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The murm adapter: cabals as conversations, posts as messages.
//!
//! A [`MurmAdapter`] presents a set of murm cabals as comms conversations. Each
//! cabal is one conversation (its `"session"` channel by default); each
//! [`Text`](murm::PostKind::Text) post is a message. Control posts (join, leave,
//! topic, info, delete) are not shown as messages in this pass.
//!
//! ## The cabal seam
//!
//! The adapter reads and writes through [`CabalSink`], implemented for both a
//! plain [`murm::CabalHandle`] (local authoring) and a [`murm::SyncedCabal`]
//! (authoring that gossips to peers). The host hands the adapter synced cabals so
//! sends propagate; tests use lighter handles. Which cabals to surface is the
//! host's call (from the user's joined-cabals set); the adapter just maps what it
//! is given.

use async_trait::async_trait;

use murm::{
    CabalHandle, CabalId, Ed25519PublicKey, MurmError, Post, PostId, PostKind, SyncedCabal,
    hash_post,
};

use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::model::{
    Conversation, ConversationId, Direction, Draft, Identity, Message, MessageBody, MessageId,
    ProtocolKind,
};

/// The default channel a cabal conversation reads and writes.
const DEFAULT_CHANNEL: &str = "session";

/// A murm cabal the adapter can read and write. Implemented for
/// [`murm::CabalHandle`] (local) and [`murm::SyncedCabal`] (gossiping), so the
/// same adapter serves tests and the live host.
#[async_trait]
pub trait CabalSink: Send + Sync {
    /// The cabal's public id.
    fn cabal_id(&self) -> CabalId;
    /// This user's author key in the cabal (used to label a message Outgoing).
    fn author_key(&self) -> Result<Ed25519PublicKey, MurmError>;
    /// The posts in `channel`, in delivery order.
    fn history(&self, channel: &str) -> Vec<Post>;
    /// Author and store a text post in `channel`, returning its id.
    async fn send_text(&self, channel: &str, text: &str) -> Result<PostId, MurmError>;
}

#[async_trait]
impl CabalSink for CabalHandle {
    fn cabal_id(&self) -> CabalId {
        *self.id()
    }
    fn author_key(&self) -> Result<Ed25519PublicKey, MurmError> {
        self.author_public_key()
    }
    fn history(&self, channel: &str) -> Vec<Post> {
        CabalHandle::history(self, channel)
    }
    async fn send_text(&self, channel: &str, text: &str) -> Result<PostId, MurmError> {
        CabalHandle::send_text(self, channel, text).await
    }
}

#[async_trait]
impl CabalSink for SyncedCabal {
    fn cabal_id(&self) -> CabalId {
        *self.handle().id()
    }
    fn author_key(&self) -> Result<Ed25519PublicKey, MurmError> {
        self.handle().author_public_key()
    }
    fn history(&self, channel: &str) -> Vec<Post> {
        SyncedCabal::history(self, channel)
    }
    async fn send_text(&self, channel: &str, text: &str) -> Result<PostId, MurmError> {
        SyncedCabal::send_text(self, channel, text).await
    }
}

/// One cabal surfaced as a conversation: a display `label` and the backend sink.
pub struct MurmCabal {
    label: String,
    sink: Box<dyn CabalSink>,
}

/// Presents murm cabals as comms conversations.
pub struct MurmAdapter {
    identity: Identity,
    channel: String,
    cabals: Vec<MurmCabal>,
}

impl MurmAdapter {
    /// A murm adapter for the given local `identity`, reading the default
    /// (`"session"`) channel.
    pub fn new(identity: Identity) -> Self {
        Self {
            identity,
            channel: DEFAULT_CHANNEL.to_string(),
            cabals: Vec::new(),
        }
    }

    /// Read and write a non-default channel.
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = channel.into();
        self
    }

    /// Surface a cabal as a conversation labelled `label`.
    pub fn with_cabal(mut self, label: impl Into<String>, sink: Box<dyn CabalSink>) -> Self {
        self.cabals.push(MurmCabal {
            label: label.into(),
            sink,
        });
        self
    }

    /// The cabal whose id (hex) matches `key`.
    fn cabal_for(&self, key: &str) -> Option<&MurmCabal> {
        self.cabals
            .iter()
            .find(|cabal| hex(cabal.sink.cabal_id().as_bytes()) == key)
    }

    /// Map a cabal's channel history into messages, sorted by timestamp.
    fn messages_of(&self, cabal: &MurmCabal) -> Result<Vec<Message>, AdapterError> {
        let me = cabal
            .sink
            .author_key()
            .map_err(|error| AdapterError::Backend(error.to_string()))?
            .to_bytes();
        let mut messages: Vec<Message> = cabal
            .sink
            .history(&self.channel)
            .iter()
            .filter_map(|post| post_to_message(post, &me))
            .collect();
        messages.sort_by_key(|message| message.timestamp_ms);
        Ok(messages)
    }
}

#[async_trait]
impl ProtocolAdapter for MurmAdapter {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Murm
    }

    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        let mut conversations = Vec::with_capacity(self.cabals.len());
        for cabal in &self.cabals {
            let messages = self.messages_of(cabal)?;
            let last_activity_ms = messages.iter().filter_map(|m| m.timestamp_ms).max();
            let mut participants: Vec<Identity> = Vec::new();
            for message in &messages {
                if !participants
                    .iter()
                    .any(|p| p.address == message.author.address)
                {
                    participants.push(message.author.clone());
                }
            }
            conversations.push(Conversation {
                id: ConversationId::new(ProtocolKind::Murm, hex(cabal.sink.cabal_id().as_bytes())),
                title: cabal.label.clone(),
                participants,
                last_activity_ms,
                unread: 0,
            });
        }
        Ok(conversations)
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        let cabal = self
            .cabal_for(&conversation.key)
            .ok_or(AdapterError::NotFound)?;
        self.messages_of(cabal)
    }

    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let conversation = draft.conversation.as_ref().ok_or(AdapterError::NotFound)?;
        let cabal = self
            .cabal_for(&conversation.key)
            .ok_or(AdapterError::NotFound)?;
        let post_id = cabal
            .sink
            .send_text(&self.channel, &draft.body)
            .await
            .map_err(|error| AdapterError::Backend(error.to_string()))?;
        Ok(MessageId(hex(&post_id.0)))
    }
}

/// Map a single post to a message, or `None` for control posts (only `Text` posts
/// are shown). Direction is `Outgoing` when the post's author is this user's
/// cabal key.
fn post_to_message(post: &Post, my_author: &[u8; 32]) -> Option<Message> {
    let PostKind::Text {
        text, timestamp_ms, ..
    } = &post.kind
    else {
        return None;
    };
    let direction = if post.author.to_bytes() == *my_author {
        Direction::Outgoing
    } else {
        Direction::Incoming
    };
    Some(Message {
        id: MessageId(hex(&hash_post(post).0)),
        author: Identity::new(ProtocolKind::Murm, hex(&post.author.to_bytes())),
        body: MessageBody::PlainText(text.clone()),
        subject: None,
        timestamp_ms: Some(*timestamp_ms),
        direction,
    })
}

/// Lowercase hex, no separators.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use identity::Ed25519Keypair;
    use murm::{ChannelName, sign_post};

    /// A canned cabal: fixed id, fixed "me" key, and a list of posts.
    struct FakeSink {
        cabal_id: CabalId,
        author: Ed25519PublicKey,
        posts: Vec<Post>,
    }

    #[async_trait]
    impl CabalSink for FakeSink {
        fn cabal_id(&self) -> CabalId {
            self.cabal_id
        }
        fn author_key(&self) -> Result<Ed25519PublicKey, MurmError> {
            Ok(self.author)
        }
        fn history(&self, channel: &str) -> Vec<Post> {
            self.posts
                .iter()
                .filter(|post| post.kind.channel().map(|c| c.as_str()) == Some(channel))
                .cloned()
                .collect()
        }
        async fn send_text(&self, _channel: &str, _text: &str) -> Result<PostId, MurmError> {
            Ok(PostId([7u8; 32]))
        }
    }

    fn text(keypair: &Ed25519Keypair, cabal_id: [u8; 32], body: &str, ts: u64) -> Post {
        sign_post(
            keypair,
            cabal_id,
            0,
            None,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: body.to_string(),
                timestamp_ms: ts,
            },
        )
    }

    fn adapter_with_one_cabal() -> (MurmAdapter, String) {
        let me = Ed25519Keypair::from_seed([1u8; 32]);
        let peer = Ed25519Keypair::from_seed([2u8; 32]);
        let cabal_id = [0x42u8; 32];

        let mine = text(&me, cabal_id, "mine", 2_000);
        let theirs = text(&peer, cabal_id, "theirs", 1_000);
        // A control post that must not become a message.
        let join = sign_post(
            &peer,
            cabal_id,
            0,
            None,
            vec![],
            PostKind::Join {
                channel: ChannelName::new("session"),
                timestamp_ms: 1_500,
            },
        );

        let sink = FakeSink {
            cabal_id: CabalId::new(cabal_id),
            author: me.public_key(),
            posts: vec![mine, theirs, join],
        };
        let adapter = MurmAdapter::new(Identity::new(ProtocolKind::Murm, "me"))
            .with_cabal("Project cabal", Box::new(sink));
        (adapter, hex(&cabal_id))
    }

    #[tokio::test]
    async fn conversations_map_a_cabal_to_a_conversation() {
        let (adapter, cabal_hex) = adapter_with_one_cabal();
        let conversations = adapter.conversations().await.unwrap();
        assert_eq!(conversations.len(), 1);
        let conversation = &conversations[0];
        assert_eq!(conversation.title, "Project cabal");
        assert_eq!(conversation.id.key, cabal_hex);
        // Latest text timestamp.
        assert_eq!(conversation.last_activity_ms, Some(2_000));
        // Two distinct authors (me + peer); the join post adds no new participant.
        assert_eq!(conversation.participants.len(), 2);
    }

    #[tokio::test]
    async fn messages_filter_to_text_and_carry_direction() {
        let (adapter, cabal_hex) = adapter_with_one_cabal();
        let messages = adapter
            .messages(&ConversationId::new(ProtocolKind::Murm, cabal_hex))
            .await
            .unwrap();
        // Join post is filtered; two text posts remain, time-sorted.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].body.text(), "theirs");
        assert_eq!(messages[0].direction, Direction::Incoming);
        assert_eq!(messages[1].body.text(), "mine");
        assert_eq!(messages[1].direction, Direction::Outgoing);
    }

    #[tokio::test]
    async fn send_returns_the_post_id_as_hex() {
        let (adapter, cabal_hex) = adapter_with_one_cabal();
        let mut draft = Draft::reply_to(ConversationId::new(ProtocolKind::Murm, cabal_hex));
        draft.body = "hello".to_string();
        let id = adapter.send(&draft).await.unwrap();
        assert_eq!(id, MessageId(hex(&[7u8; 32])));
    }

    #[tokio::test]
    async fn unknown_cabal_is_not_found() {
        let (adapter, _) = adapter_with_one_cabal();
        let result = adapter
            .messages(&ConversationId::new(ProtocolKind::Murm, "deadbeef"))
            .await;
        assert_eq!(result, Err(AdapterError::NotFound));
    }
}
