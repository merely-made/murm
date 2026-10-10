// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The murm adapter: cabals as conversations, posts as messages.
//!
//! A [`MurmAdapter`] presents a set of murm cabals as comms conversations. Each
//! cabal/channel is one conversation (`"session"` by default); each
//! [`Text`](murm::PostKind::Text) post is a message. Control posts (join, leave,
//! topic, info, delete) are not shown as messages in this pass.
//! The owner read exposes missing-history diagnostics separately. This adapter
//! preserves causal order but does not fold deletion or moderation effects.
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
    CabalHandle, CabalHistory, CabalId, ChannelName, Ed25519PublicKey, MurmError, Post, PostId,
    PostKind, SyncedCabal, hash_post,
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
    /// A current causally ordered read, with missing-history diagnostics.
    async fn history(&self, channel: &str) -> Result<CabalHistory, MurmError>;
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
    async fn history(&self, channel: &str) -> Result<CabalHistory, MurmError> {
        self.causal_history(channel).await
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
    async fn history(&self, channel: &str) -> Result<CabalHistory, MurmError> {
        self.handle().causal_history(channel).await
    }
    async fn send_text(&self, channel: &str, text: &str) -> Result<PostId, MurmError> {
        SyncedCabal::send_text(self, channel, text).await
    }
}

/// One cabal surfaced as a conversation: a display `label` and the backend sink.
pub struct MurmCabal {
    label: String,
    sink: Box<dyn CabalSink>,
    channel: Option<String>,
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

    /// Read and write a non-default channel for entries added with `with_cabal`.
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = channel.into();
        self
    }

    /// Surface the adapter's default channel as a conversation labelled `label`.
    pub fn with_cabal(mut self, label: impl Into<String>, sink: Box<dyn CabalSink>) -> Self {
        self.cabals.push(MurmCabal {
            label: label.into(),
            sink,
            channel: None,
        });
        self
    }

    /// Surface an explicit channel alongside other channels of the same cabal.
    /// This channel is independent of the adapter's default `with_channel`.
    pub fn with_cabal_in_channel(
        mut self,
        label: impl Into<String>,
        channel: impl Into<String>,
        sink: Box<dyn CabalSink>,
    ) -> Self {
        self.cabals.push(MurmCabal {
            label: label.into(),
            sink,
            channel: Some(channel.into()),
        });
        self
    }

    fn channel_of<'a>(&'a self, cabal: &'a MurmCabal) -> &'a str {
        cabal.channel.as_deref().unwrap_or(&self.channel)
    }

    /// Exact protocol, cabal and channel lookup; the identifier grants nothing.
    fn cabal_for(&self, conversation: &ConversationId) -> Option<&MurmCabal> {
        self.cabals.iter().find(|cabal| {
            conversation_id(cabal.sink.cabal_id(), self.channel_of(cabal)) == *conversation
        })
    }

    /// Observe the owner read without hiding its missing-history diagnostics.
    pub async fn history(
        &self,
        conversation: &ConversationId,
    ) -> Result<CabalHistory, AdapterError> {
        let cabal = self.cabal_for(conversation).ok_or(AdapterError::NotFound)?;
        let channel = self.channel_of(cabal);
        if !ChannelName::is_valid_name(channel) {
            return Err(AdapterError::Unsupported("invalid Murm channel".into()));
        }
        cabal
            .sink
            .history(channel)
            .await
            .map_err(|error| AdapterError::Backend(error.to_string()))
    }

    /// Map channel posts without changing the owner's causal order.
    async fn messages_of(&self, cabal: &MurmCabal) -> Result<Vec<Message>, AdapterError> {
        let me = cabal
            .sink
            .author_key()
            .map_err(|error| AdapterError::Backend(error.to_string()))?
            .to_bytes();
        let history = self
            .history(&conversation_id(
                cabal.sink.cabal_id(),
                self.channel_of(cabal),
            ))
            .await?;
        let messages: Vec<Message> = history
            .posts
            .iter()
            .filter_map(|post| post_to_message(post, &me))
            .collect();
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
            let messages = self.messages_of(cabal).await?;
            let last_activity_ms = messages.iter().filter_map(|m| m.timestamp_ms).max();
            let mut participants: Vec<Identity> = Vec::new();
            // Observed authors only, not a membership roster or live presence.
            for message in &messages {
                if !participants
                    .iter()
                    .any(|p| p.address == message.author.address)
                {
                    participants.push(message.author.clone());
                }
            }
            conversations.push(Conversation {
                id: conversation_id(cabal.sink.cabal_id(), self.channel_of(cabal)),
                title: cabal.label.clone(),
                participants,
                last_activity_ms,
                unread: 0,
            });
        }
        Ok(conversations)
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        let cabal = self.cabal_for(conversation).ok_or(AdapterError::NotFound)?;
        self.messages_of(cabal).await
    }

    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let conversation = draft.conversation.as_ref().ok_or(AdapterError::NotFound)?;
        let cabal = self.cabal_for(conversation).ok_or(AdapterError::NotFound)?;
        if draft.is_empty() {
            return Err(AdapterError::Unsupported("message body is empty".into()));
        }
        if draft.subject.is_some() {
            return Err(AdapterError::Unsupported(
                "Murm text carries no subject line".into(),
            ));
        }
        let channel = self.channel_of(cabal);
        if !ChannelName::is_valid_name(channel) {
            return Err(AdapterError::Unsupported("invalid Murm channel".into()));
        }
        let post_id = cabal
            .sink
            .send_text(channel, &draft.body)
            .await
            .map_err(|error| AdapterError::Backend(error.to_string()))?;
        Ok(MessageId(hex(&post_id.0)))
    }
}

/// A private view address for the exact cabal/channel. The key is opaque to
/// hosts, not a URL or a capability; use the adapter's listed identities.
pub fn conversation_id(cabal: CabalId, channel: &str) -> ConversationId {
    ConversationId::new(
        ProtocolKind::Murm,
        format!("{}/{channel}", hex(cabal.as_bytes())),
    )
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
        async fn history(&self, channel: &str) -> Result<CabalHistory, MurmError> {
            Ok(CabalHistory {
                posts: self
                    .posts
                    .iter()
                    .filter(|post| post.kind.channel().map(|c| c.as_str()) == Some(channel))
                    .cloned()
                    .collect(),
                ..Default::default()
            })
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
        (
            adapter,
            conversation_id(CabalId::new(cabal_id), DEFAULT_CHANNEL).key,
        )
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
        // Join post is filtered; the owner's ordering survives opposing clocks.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].body.text(), "mine");
        assert_eq!(messages[0].direction, Direction::Outgoing);
        assert_eq!(messages[1].body.text(), "theirs");
        assert_eq!(messages[1].direction, Direction::Incoming);
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

    #[cfg(not(target_arch = "wasm32"))]
    async fn real_cabal() -> (murm::Murm<transport::memory::MemoryTransport>, CabalHandle) {
        use identity::{IdentityProvider as _, InMemoryProvider};
        let identity = InMemoryProvider::from_seed([0x61; 32]);
        let local = murm::PeerID::from_public_key(identity.master_public_key());
        let peer = murm::PeerID::from_public_key(
            InMemoryProvider::from_seed([0x62; 32]).master_public_key(),
        );
        let (transport, _) = transport::memory::MemoryTransport::pair(local, peer);
        let runtime = murm::Murm::new(std::sync::Arc::new(identity), transport);
        let cabal = runtime
            .open_cabal(&murm::CabalKey::new([0x63; 32]))
            .await
            .unwrap();
        (runtime, cabal)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn real_channels_share_a_cabal_without_aliasing_order_drafts_or_dispatch() {
        let (_runtime, cabal) = real_cabal().await;
        let first = cabal.send_text_at("hall", "first", 9_000).await.unwrap();
        cabal
            .send_text_at("music", "different channel", 8_000)
            .await
            .unwrap();
        let second = cabal.send_text_at("hall", "second", 1).await.unwrap();
        let adapter = MurmAdapter::new(Identity::new(ProtocolKind::Murm, "local"))
            .with_cabal_in_channel("Hall", "hall", Box::new(cabal.clone()))
            .with_cabal_in_channel("Music", "music", Box::new(cabal.clone()));
        let hall = conversation_id(*cabal.id(), "hall");
        let music = conversation_id(*cabal.id(), "music");
        assert_ne!(hall, music);
        let messages = adapter.messages(&hall).await.unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.id.clone())
                .collect::<Vec<_>>(),
            [MessageId(hex(&first.0)), MessageId(hex(&second.0))]
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.timestamp_ms)
                .collect::<Vec<_>>(),
            [Some(9_000), Some(1)]
        );
        assert!(
            messages
                .iter()
                .all(|message| message.direction == Direction::Outgoing)
        );
        assert!(adapter.history(&hall).await.unwrap().pending.is_empty());

        let comms = crate::Comms::new().with_adapter(Box::new(adapter));
        let mut pane = crate::CommsPane::new();
        pane.set_inbox(comms.inbox().await);
        assert_eq!(pane.inbox.len(), 2);
        pane.select(hall.clone());
        pane.draft.body = "hall draft".into();
        pane.select(music.clone());
        pane.draft.body = "music draft".into();
        pane.select(hall.clone());
        assert_eq!(pane.draft.body, "hall draft");
        let submitted = pane.draft.clone();
        let sent = comms.send(&submitted).await.unwrap();
        let read = cabal.causal_history("hall").await.unwrap();
        assert_eq!(read.posts.len(), 3);
        let stored = read.posts.last().unwrap();
        assert_eq!(sent, MessageId(hex(&hash_post(stored).0)));
        assert_eq!(stored.kind.channel().unwrap().as_str(), "hall");
        assert!(matches!(&stored.kind, PostKind::Text { text, .. } if text == &submitted.body));
        assert_eq!(cabal.causal_history("music").await.unwrap().posts.len(), 1);
        pane.draft.body.push_str(" with newer edits");
        assert!(!pane.acknowledge_sent(&submitted));
        assert_eq!(pane.draft.body, "hall draft with newer edits");
        pane.select(music);
        assert_eq!(pane.draft.body, "music draft");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn real_owner_is_untouched_by_wrong_addresses_or_unsupported_drafts() {
        let (runtime, cabal) = real_cabal().await;
        let adapter = MurmAdapter::new(Identity::new(ProtocolKind::Murm, "local"))
            .with_cabal("Session", Box::new(cabal.clone()));
        let target = conversation_id(*cabal.id(), DEFAULT_CHANNEL);
        let foreign = ConversationId::new(ProtocolKind::CommonsChat, target.key.clone());
        assert_eq!(
            adapter.messages(&foreign).await,
            Err(AdapterError::NotFound)
        );
        for address in [
            foreign,
            conversation_id(*cabal.id(), "other"),
            ConversationId::new(ProtocolKind::Murm, hex(cabal.id().as_bytes())),
        ] {
            let mut draft = Draft::reply_to(address);
            draft.body = "must not be authored".into();
            assert_eq!(adapter.send(&draft).await, Err(AdapterError::NotFound));
        }
        let mut empty = Draft::reply_to(target.clone());
        empty.body = " \n ".into();
        assert!(matches!(
            adapter.send(&empty).await,
            Err(AdapterError::Unsupported(_))
        ));
        let mut subject = Draft::reply_to(target);
        subject.body = "preserve this body".into();
        subject.subject = Some("must not be discarded".into());
        let original = subject.clone();
        assert!(matches!(
            adapter.send(&subject).await,
            Err(AdapterError::Unsupported(_))
        ));
        assert_eq!(subject, original);
        let invalid = MurmAdapter::new(Identity::new(ProtocolKind::Murm, "local"))
            .with_channel("\n")
            .with_cabal("Invalid", Box::new(cabal.clone()));
        let invalid_id = conversation_id(*cabal.id(), "\n");
        assert!(matches!(
            invalid.messages(&invalid_id).await,
            Err(AdapterError::Unsupported(_))
        ));
        let store = runtime
            .conversation_engine()
            .sync_store(cabal.id().as_bytes())
            .unwrap();
        assert_eq!(store.operation_count().await.unwrap(), 0);
        assert!(
            cabal
                .causal_history(DEFAULT_CHANNEL)
                .await
                .unwrap()
                .posts
                .is_empty()
        );
        // The owner can disappear after selection/preparation. A valid address
        // still cannot turn that stale view into successful authoring.
        let mut pending = Draft::reply_to(conversation_id(*cabal.id(), DEFAULT_CHANNEL));
        pending.body = "retry when the owner returns".into();
        let original = pending.clone();
        assert!(runtime.conversation_engine().close(cabal.id().as_bytes()));
        assert!(matches!(
            adapter.send(&pending).await,
            Err(AdapterError::Backend(_))
        ));
        assert_eq!(pending, original);
        assert_eq!(store.operation_count().await.unwrap(), 0);
    }
}
