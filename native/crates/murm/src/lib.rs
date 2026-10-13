// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! # Murm
//!
//! Direct peer-to-peer conversation service for the
//! [`mere`](https://crates.io/crates/mere) browser.
//!
//! See the workspace's active
//! Murm peer-runtime plan (`mere/design_docs/mere_docs/implementation_strategy/2026-07-12_murm_peer_runtime_and_moot_domain_plan.md`)
//! for the architectural specification.
//!
//! ## What Murm owns
//!
//! - **Cabal lifecycle** — opening, joining, leaving cabals; cabal-id
//!   addressing; the `Cabal` handle returned to consumers
//! - **Bilateral identity orchestration** — uses
//!   [`identity::IdentityProvider`] for per-cabal keypair derivation
//!   (Cable spec §2.2 pattern)
//! - **Transport orchestration** — uses [`transport::Transport`] for
//!   stream-level peer connections; Murm is generic over the transport
//!   implementation
//! - **Conversation storage and causal reads** — durable signed operations,
//!   observed frontiers, and owner-authorized retention checkpoints
//!
//! ## What Murm does NOT own
//!
//! - Master keypair / OS keychain → [`identity`]
//! - iroh / QUIC / ALPN → [`transport`]
//! - Alternate protocol adapters → the comms integration boundary
//! - User-facing chat panel UI → graphshell-side Comms applet (separate)
//! - Many-to-many federation → `gemot`
//! - Shared activity lifecycle → the concrete activity's application owner
//!
//! Product surfaces call a conversation a *murmur*. The retained `Cabal*` API
//! and `mere/cable/v1` ALPN name Mere's inherited conversation grammar; they do
//! not establish interoperability with cabal-club Cable.
//!
//! ## Status
//!
//! Pre-1.0. Phase 2B: the Cable-shaped Murm dialect is in place, and a cabal has a
//! real **send** ([`CabalHandle::send_text`] and siblings), **history**
//! ([`CabalHandle::history`]), and live **subscribe** ([`CabalHandle::subscribe`])
//! API. With a [`transport::P2pandaTransport`], [`SyncedCabal`] adds the two sync lanes
//! (live gossip + LogSync catch-up) on top of the same surface.

#![doc(html_root_url = "https://docs.rs/murm/0.0.1")]
#![warn(missing_docs)]

mod cabal;
mod conversation_backend;
mod conversation_engine;
mod conversation_store;
mod drop_export;
mod error;
mod gossip_sync;
mod key_epoch;
mod post;
mod post_hash;
mod post_sign;
mod post_wire;
#[cfg(feature = "session-lane")]
mod session_lane;

pub use crate::cabal::{CabalHandle, CabalId, CabalKey, CabalMembership};
pub use crate::conversation_backend::{ConversationBackend, ConversationStorage};
pub use crate::conversation_engine::{CabalHistory, ConversationEngine, ConversationRefresh};
pub use crate::conversation_store::{
    CabalCheckpointAuthority, ConversationStore, ConversationStoreError,
};
pub use crate::drop_export::{
    ConversationDropPriorities, ConversationDropPrivacy, ConversationDropProfile,
    ConversationDropSelector, ConversationFrontier,
};
pub use crate::error::MurmError;
pub use crate::gossip_sync::SyncedCabal;
pub use crate::key_epoch::{CABAL_DROP_SUITE, CabalKeyEpoch, CabalKeyring, CabalKeyringError};
pub use crate::post::{ChannelName, InfoEntry, Post, PostId, PostKind};
pub use crate::post_hash::hash_cabal_id;
pub use crate::post_sign::{sign_post, verify_post};
pub use crate::post_wire::{
    CabalExt, decode_post, encode_post, operation_id, operation_to_post, post_to_operation,
};
#[cfg(feature = "session-lane")]
pub use crate::session_lane::{
    Admission, MAX_POST_FRAME, SessionOutcome, lane_binding, push_posts, serve_accepted_session,
    serve_session,
};

// Re-export key types from the layers we sit on, so consumers don't all
// need direct dependencies on the lower crates.
pub use identity::{Ed25519PublicKey, IdentityProvider};
pub use transport::{Alpn, PeerID, Transport};

/// Compute a post's signed-header identity.
pub fn hash_post(post: &Post) -> PostId {
    operation_id(post)
}

use std::sync::Arc;

/// The Mere bilateral-comms supercrate entry point.
///
/// `Murm` orchestrates identity, transport, conversation admission, storage,
/// materialization, and sync.
///
/// ## Generic over transport
///
/// `Murm<T: Transport>` takes the transport as a generic parameter, not a
/// `Box<dyn Transport>`. This avoids object-safety issues with
/// `Transport::Stream` and lets the same code work against an iroh-backed
/// transport in production and an in-memory transport in tests.
///
/// ## Identity is `Arc<dyn IdentityProvider>`
///
/// `IdentityProvider` is object-safe (sync methods only at this stage), so
/// using `Arc<dyn ...>` here is fine and gives flexibility — the same
/// identity backend can be shared with `transport` (for `PeerID`
/// derivation) and other consumers without a generic parameter explosion.
///
/// ## Status
///
/// Phase 2B. [`open_cabal`](Murm::open_cabal) returns a working
/// [`CabalHandle`] (send / history / subscribe); over a [`transport::P2pandaTransport`],
/// [`subscribe_cabal`](Murm::subscribe_cabal) returns a [`SyncedCabal`] that
/// also runs the gossip + LogSync lanes. Shared activity lifecycle belongs to
/// the concrete application consuming the conversation and admission seams.
pub struct Murm<T: Transport> {
    identity: Arc<dyn IdentityProvider>,
    transport: T,
    conversation: Arc<ConversationEngine>,
}

impl<T: Transport> Murm<T> {
    /// Construct a `Murm` with a given identity provider and transport.
    pub fn new(identity: Arc<dyn IdentityProvider>, transport: T) -> Self {
        Self::with_storage(identity, transport, ConversationStorage::Memory)
    }

    /// Construct Murm with host-selected conversation persistence.
    pub fn with_storage(
        identity: Arc<dyn IdentityProvider>,
        transport: T,
        storage: ConversationStorage,
    ) -> Self {
        let conversation = Arc::new(ConversationEngine::with_storage(identity.clone(), storage));
        Self {
            identity,
            transport,
            conversation,
        }
    }

    /// The local carrier's configured peer identity.
    ///
    /// A production carrier uses a scoped Personae-derived key, which can
    /// differ from both the persona root and the conversation author key.
    pub fn local_peer_id(&self) -> PeerID {
        self.transport.local_peer_id()
    }

    /// Access the underlying identity provider.
    pub fn identity(&self) -> &Arc<dyn IdentityProvider> {
        &self.identity
    }

    /// Access the underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Access the underlying native conversation runtime.
    ///
    /// Useful for advanced operations not yet exposed on
    /// [`CabalHandle`] (e.g. inspecting all open cabals, custom post
    /// composition).
    pub fn conversation_engine(&self) -> &Arc<ConversationEngine> {
        &self.conversation
    }

    /// Open or join a cabal in Mere's Cable-shaped dialect by its secret key.
    ///
    /// Derives the per-cabal Ed25519 keypair (Cable spec §2.2),
    /// computes the public cabal id (BLAKE3 of the key), and opens the
    /// host-selected store for the cabal. Idempotent: opening the same key
    /// twice returns equivalent handles backed by the same underlying
    /// session.
    ///
    /// Returns a [`CabalHandle`] for sending and querying posts.
    pub async fn open_cabal(&self, cabal_key: &CabalKey) -> Result<CabalHandle, MurmError> {
        let id_bytes = self.conversation.open(*cabal_key.as_bytes()).await?;
        let cabal_id = CabalId::new(id_bytes);
        Ok(CabalHandle::new(cabal_id, Arc::clone(&self.conversation)))
    }

    /// Compute the per-cabal keypair for a given cabal key.
    ///
    /// The identity provider derives the key with the cabal secret as scope.
    ///
    /// The returned keypair signs cabal posts authored by this user. The
    /// master secret never leaves the identity provider.
    pub fn derive_cabal_keypair(
        &self,
        cabal_key: &CabalKey,
    ) -> Result<identity::Ed25519Keypair, MurmError> {
        Ok(self.identity.derive_keypair(cabal_key.as_bytes())?)
    }
}

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lifecycle stage marker.
pub const STAGE: &str = "pre-alpha";

#[cfg(test)]
mod tests {
    use super::*;
    use identity::InMemoryProvider;
    use p2panda_core::Topic;
    use std::io::Cursor;
    use std::sync::Arc;
    use stickleback::{
        DropExportProfile, DropLimits, export_topic_operations, write_plain_drop,
        write_protected_drop,
    };
    use tempfile::tempdir;
    use tokio::io::DuplexStream;

    /// A no-op transport for foundation tests. Connect/accept always
    /// refuse; only `local_peer_id` returns a meaningful value. Phase 2B
    /// will replace this with an in-process loopback transport for full
    /// protocol-roundtrip tests.
    struct StubTransport {
        peer_id: PeerID,
    }

    impl StubTransport {
        fn new(peer_id: PeerID) -> Self {
            Self { peer_id }
        }
    }

    impl Transport for StubTransport {
        type Stream = DuplexStream;

        fn local_peer_id(&self) -> PeerID {
            self.peer_id
        }

        async fn connect(
            &self,
            _peer: PeerID,
            _alpn: Alpn,
        ) -> Result<Self::Stream, transport::TransportError> {
            Err(transport::TransportError::ConnectionRefused)
        }

        async fn accept(
            &self,
            _alpn: Alpn,
        ) -> Result<transport::AcceptedSession<Self::Stream>, transport::TransportError> {
            Err(transport::TransportError::ConnectionRefused)
        }
    }

    fn make_murm() -> Murm<StubTransport> {
        let identity: Arc<dyn IdentityProvider> = Arc::new(InMemoryProvider::from_seed([42; 32]));
        let peer_id = PeerID::from_public_key(identity.master_public_key());
        let transport = StubTransport::new(peer_id);
        Murm::new(identity, transport)
    }

    #[test]
    fn murm_constructs_with_identity_and_transport() {
        let _murm = make_murm();
    }

    #[test]
    fn local_peer_id_matches_master_public_key() {
        let murm = make_murm();
        let expected = PeerID::from_public_key(murm.identity().master_public_key());
        assert_eq!(murm.local_peer_id(), expected);
    }

    #[test]
    fn local_peer_id_matches_transport_peer_id_when_consistent() {
        let murm = make_murm();
        assert_eq!(murm.local_peer_id(), murm.transport().local_peer_id());
    }

    #[tokio::test]
    async fn scoped_carrier_identity_routes_without_replacing_the_conversation_author() {
        let identity: Arc<dyn IdentityProvider> = Arc::new(InMemoryProvider::from_seed([0x61; 32]));
        let carrier_key = identity.derived_keypair(b"murm-carrier").unwrap();
        let carrier_peer = PeerID::from_public_key(carrier_key.public_key());
        let root_peer = PeerID::from_public_key(identity.master_public_key());
        assert_ne!(carrier_peer, root_peer);
        let other_peer =
            PeerID::from_public_key(InMemoryProvider::from_seed([0x62; 32]).master_public_key());
        let (carrier, remote) = transport::memory::MemoryTransport::pair(carrier_peer, other_peer);
        let murm = Murm::new(identity.clone(), carrier);
        assert_eq!(murm.local_peer_id(), carrier_peer);
        let protocol = Alpn::new("mere/cable/v1");
        let (connected, accepted) = tokio::join!(
            remote.connect(murm.local_peer_id(), protocol.clone()),
            murm.transport().accept(protocol),
        );
        assert!(connected.is_ok());
        assert_eq!(accepted.unwrap().peer, Some(other_peer));
        let key = CabalKey::new([0x63; 32]);
        let conversation = murm.open_cabal(&key).await.unwrap();
        let post_id = conversation
            .send_text_at("session", "scoped carrier", 1)
            .await
            .unwrap();
        let post = conversation.get_post(&post_id).unwrap();
        let author = identity
            .derive_keypair(key.as_bytes())
            .unwrap()
            .public_key();
        assert_eq!(post.author, author);
        assert_ne!(post.author, carrier_key.public_key());
    }

    #[tokio::test]
    async fn open_cabal_derives_id_from_key() {
        let murm = make_murm();
        let key = CabalKey::new([42; 32]);
        let cabal = murm.open_cabal(&key).await.unwrap();
        // The id is BLAKE3 of the key — independent of which Murm
        // instance opened it. Two different Murms with the same cabal_key
        // see the same id.
        let murm2 = make_murm();
        let cabal2 = murm2.open_cabal(&key).await.unwrap();
        assert_eq!(cabal.id(), cabal2.id());
    }

    #[tokio::test]
    async fn open_cabal_send_text_history_round_trip() {
        let murm = make_murm();
        let cabal = murm.open_cabal(&CabalKey::new([7; 32])).await.unwrap();

        let id1 = cabal.send_text_at("session", "first", 1).await.unwrap();
        let id2 = cabal.send_text_at("session", "second", 2).await.unwrap();
        assert_ne!(id1, id2);

        let history = cabal.history("session");
        assert_eq!(history.len(), 2);

        // Author of stored posts matches what `author_public_key` reports.
        let expected_author = cabal.author_public_key().unwrap();
        for post in &history {
            assert_eq!(post.author.to_bytes(), expected_author.to_bytes());
        }

        // Channel isolation.
        assert!(cabal.history("links").is_empty());
    }

    #[tokio::test]
    async fn causal_history_keeps_pruned_cross_channel_witnesses_and_reports_missing_headers() {
        let murm = make_murm();
        let cabal = murm.open_cabal(&CabalKey::new([0x34; 32])).await.unwrap();
        let first = cabal.send_text_at("session", "first", 9_000).await.unwrap();
        let bridge = cabal
            .send_text_at("links", "between channels", 8_000)
            .await
            .unwrap();
        let last = cabal.send_text_at("session", "last", 1).await.unwrap();
        let store = murm
            .conversation_engine()
            .sync_store(cabal.id().as_bytes())
            .unwrap();
        assert!(
            store
                .delete_operation_payload(&bridge.0.into())
                .await
                .unwrap()
        );
        let read = cabal.causal_history("session").await.unwrap();
        assert_eq!(
            read.posts.iter().map(hash_post).collect::<Vec<_>>(),
            [first, last]
        );
        assert!(read.pending.is_empty());
        assert_eq!(read.unavailable_payloads, 0);
        assert_eq!(store.operation_count().await.unwrap(), 3);

        // A payload can disappear without breaking its header's causal witness.
        assert!(
            store
                .delete_operation_payload(&first.0.into())
                .await
                .unwrap()
        );
        let read = cabal.causal_history("session").await.unwrap();
        assert_eq!(read.posts.iter().map(hash_post).collect::<Vec<_>>(), [last]);
        assert_eq!(read.unavailable_payloads, 1);
        assert!(read.pending.is_empty());
        assert_eq!(store.operation_count().await.unwrap(), 3);

        // Header withdrawal is different: no valid complete tail can be claimed.
        assert!(store.delete_operation(&bridge.0.into()).await.unwrap());
        let read = cabal.causal_history("session").await.unwrap();
        assert!(read.posts.is_empty());
        assert_eq!(read.pending.len(), 1);
        assert_eq!(read.pending[0].operation, last.0);
        assert_eq!(read.pending[0].missing, [bridge.0]);
        assert_eq!(read.unavailable_payloads, 1);
        assert_eq!(store.operation_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn causal_history_withholds_cross_author_child_until_its_parent_arrives() {
        let murm = make_murm();
        let key = CabalKey::new([0x35; 32]);
        let cabal = murm.open_cabal(&key).await.unwrap();
        let alice = identity::InMemoryProvider::from_seed([0x36; 32]);
        let bob = identity::InMemoryProvider::from_seed([0x37; 32]);
        let parent = sign_post(
            &alice.derive_keypair(key.as_bytes()).unwrap(),
            *cabal.id().as_bytes(),
            0,
            None,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "parent".into(),
                timestamp_ms: 9_000,
            },
        );
        let child = sign_post(
            &bob.derive_keypair(key.as_bytes()).unwrap(),
            *cabal.id().as_bytes(),
            0,
            None,
            vec![hash_post(&parent)],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "child".into(),
                timestamp_ms: 1,
            },
        );
        cabal.ingest_post(child.clone()).await.unwrap();
        let unrelated = cabal.send_text_at("session", "unrelated", 2).await.unwrap();
        let before = cabal.causal_history("session").await.unwrap();
        assert_eq!(
            before.posts.iter().map(hash_post).collect::<Vec<_>>(),
            [unrelated]
        );
        assert_eq!(before.pending[0].operation, hash_post(&child).0);
        assert_eq!(before.pending[0].missing, [hash_post(&parent).0]);
        cabal.ingest_post(parent.clone()).await.unwrap();
        let after = cabal.causal_history("session").await.unwrap();
        let ids: Vec<_> = after.posts.iter().map(hash_post).collect();
        assert_eq!(ids.len(), 3);
        assert!(
            ids.iter().position(|id| *id == hash_post(&parent)).unwrap()
                < ids.iter().position(|id| *id == hash_post(&child)).unwrap()
        );
        assert!(after.pending.is_empty());
        assert!(cabal.causal_history("").await.is_err());
    }

    #[tokio::test]
    async fn durable_cabal_reopens_history_and_author_head() {
        let directory = tempdir().unwrap();
        let storage = ConversationStorage::redb(directory.path());
        let key = CabalKey::new([0x73; 32]);

        let provider: Arc<dyn IdentityProvider> = Arc::new(InMemoryProvider::from_seed([0x31; 32]));
        let peer_id = PeerID::from_public_key(provider.master_public_key());
        let murm = Murm::with_storage(provider, StubTransport::new(peer_id), storage.clone());
        let cabal = murm.open_cabal(&key).await.unwrap();
        cabal
            .send_text_at("session", "before restart", 1)
            .await
            .unwrap();
        drop(cabal);
        drop(murm);

        let provider: Arc<dyn IdentityProvider> = Arc::new(InMemoryProvider::from_seed([0x31; 32]));
        let peer_id = PeerID::from_public_key(provider.master_public_key());
        let reopened = Murm::with_storage(provider, StubTransport::new(peer_id), storage);
        let cabal = reopened.open_cabal(&key).await.unwrap();
        assert_eq!(cabal.history("session").len(), 1);
        cabal
            .send_text_at("session", "after restart", 2)
            .await
            .unwrap();
        let history = cabal.history("session");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seq_num, 0);
        assert_eq!(history[1].seq_num, 1);
    }

    #[tokio::test]
    async fn imported_drop_refreshes_history_and_subscription_once() {
        let key = CabalKey::new([0x74; 32]);
        let source = make_murm();
        let source_cabal = source.open_cabal(&key).await.unwrap();
        let post_id = source_cabal
            .send_text_at("session", "carried by a drop", 1)
            .await
            .unwrap();
        let source_store = source
            .conversation_engine()
            .sync_store(source_cabal.id().as_bytes())
            .unwrap();
        let records = export_topic_operations::<_, _, u64>(
            &source_store,
            &Topic::from(*source_cabal.id().as_bytes()),
            DropExportProfile::default(),
        )
        .await
        .unwrap();
        let mut bytes = Vec::new();
        write_plain_drop(&mut bytes, &records, DropLimits::default()).unwrap();

        let target_provider: Arc<dyn IdentityProvider> =
            Arc::new(InMemoryProvider::from_seed([0x75; 32]));
        let target_id = PeerID::from_public_key(target_provider.master_public_key());
        let target = Murm::new(target_provider, StubTransport::new(target_id));
        let target_cabal = target.open_cabal(&key).await.unwrap();
        let mut events = target_cabal.subscribe().unwrap();
        let (import, refresh) = target_cabal
            .import_plain_drop(Cursor::new(bytes.clone()), DropLimits::default())
            .await
            .unwrap();
        assert_eq!(import.accepted, 1);
        assert_eq!(refresh.added, 1);
        assert_eq!(target_cabal.history("session").len(), 1);
        assert_eq!(hash_post(&events.try_recv().unwrap()), post_id);

        let (repeat, refresh) = target_cabal
            .import_plain_drop(Cursor::new(bytes), DropLimits::default())
            .await
            .unwrap();
        assert!(repeat.receipt_hit);
        assert_eq!(refresh.added, 0);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn protected_drop_refresh_survives_key_rotation() {
        let key = CabalKey::new([0x76; 32]);
        let source = make_murm();
        let source_cabal = source.open_cabal(&key).await.unwrap();
        source_cabal
            .send_text_at("session", "old epoch", 1)
            .await
            .unwrap();

        let source_store = source
            .conversation_engine()
            .sync_store(source_cabal.id().as_bytes())
            .unwrap();
        let topic = Topic::from(*source_cabal.id().as_bytes());
        let old_records = export_topic_operations::<_, _, u64>(
            &source_store,
            &topic,
            DropExportProfile::default(),
        )
        .await
        .unwrap();
        let mut old_drop = Vec::new();
        write_protected_drop(
            &mut old_drop,
            &old_records,
            DropLimits::default(),
            &source_cabal.keyring().unwrap(),
        )
        .unwrap();

        let rotated_key = [0x77; 32];
        source_cabal
            .install_key_epoch(CabalKeyEpoch(1), rotated_key)
            .unwrap();
        source_cabal
            .send_text_at("session", "new epoch", 2)
            .await
            .unwrap();
        let new_records = export_topic_operations::<_, _, u64>(
            &source_store,
            &topic,
            DropExportProfile::default(),
        )
        .await
        .unwrap();
        let mut new_drop = Vec::new();
        write_protected_drop(
            &mut new_drop,
            &new_records,
            DropLimits::default(),
            &source_cabal.keyring().unwrap(),
        )
        .unwrap();

        let target = make_murm();
        let target_cabal = target.open_cabal(&key).await.unwrap();
        target_cabal
            .install_key_epoch(CabalKeyEpoch(1), rotated_key)
            .unwrap();
        let (_, old_refresh) = target_cabal
            .import_protected_drop(Cursor::new(old_drop.clone()), DropLimits::default())
            .await
            .unwrap();
        let (_, new_refresh) = target_cabal
            .import_protected_drop(Cursor::new(new_drop), DropLimits::default())
            .await
            .unwrap();
        assert_eq!(old_refresh.added, 1);
        assert_eq!(new_refresh.added, 1);
        assert_eq!(target_cabal.history("session").len(), 2);

        assert_eq!(
            target_cabal
                .keyring()
                .unwrap()
                .forget_before(CabalKeyEpoch(1)),
            1
        );
        assert!(
            target_cabal
                .import_protected_drop(Cursor::new(old_drop), DropLimits::default())
                .await
                .is_err()
        );
    }

    #[test]
    fn open_cabal_handle_is_clone_and_send() {
        // Compile-check: CabalHandle should be Clone + Send + Sync, so it
        // can be passed across tasks/threads.
        fn assert_send_sync<T: Send + Sync + Clone + 'static>() {}
        assert_send_sync::<CabalHandle>();
    }

    #[tokio::test]
    async fn two_murms_on_same_cabal_key_can_exchange_posts_via_ingest() {
        // Alice's murm posts; bob's murm ingests via the post object
        // (simulating transport delivery without yet wiring the actual
        // transport-level sync code).
        let alice_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([10; 32]));
        let alice_id = PeerID::from_public_key(alice_provider.master_public_key());
        let alice = Murm::new(alice_provider, StubTransport::new(alice_id));

        let bob_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([20; 32]));
        let bob_id = PeerID::from_public_key(bob_provider.master_public_key());
        let bob = Murm::new(bob_provider, StubTransport::new(bob_id));

        let cabal_key = CabalKey::new([0xab; 32]);
        let alice_cabal = alice.open_cabal(&cabal_key).await.unwrap();
        let bob_cabal = bob.open_cabal(&cabal_key).await.unwrap();
        // Same cabal_key → same cabal id (public).
        assert_eq!(alice_cabal.id(), bob_cabal.id());

        // Alice posts. Bob's history is still empty (no sync yet).
        let post_id = alice_cabal
            .send_text_at("session", "hi bob", 1)
            .await
            .unwrap();
        assert_eq!(alice_cabal.history("session").len(), 1);
        assert!(bob_cabal.history("session").is_empty());

        // Simulate transport delivery: bob receives the post object.
        let post = alice_cabal.get_post(&post_id).unwrap();
        let bob_post_id = bob_cabal.ingest_post(post).await.unwrap();
        assert_eq!(post_id, bob_post_id);

        // Bob's history now contains alice's post; signatures verify
        // (ingest_post checks that).
        assert_eq!(bob_cabal.history("session").len(), 1);
    }

    #[tokio::test]
    async fn cabal_subscribe_emits_authored_and_ingested_posts() {
        // Two peers on the same cabal. Bob subscribes, then sees both a post he
        // authors locally and a post he ingests from alice — each once.
        let alice_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([60; 32]));
        let alice_id = PeerID::from_public_key(alice_provider.master_public_key());
        let alice = Murm::new(alice_provider, StubTransport::new(alice_id));

        let bob_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([61; 32]));
        let bob_id = PeerID::from_public_key(bob_provider.master_public_key());
        let bob = Murm::new(bob_provider, StubTransport::new(bob_id));

        let key = CabalKey::new([0xcd; 32]);
        let alice_cabal = alice.open_cabal(&key).await.unwrap();
        let bob_cabal = bob.open_cabal(&key).await.unwrap();

        let mut bob_rx = bob_cabal.subscribe().unwrap();

        // Bob authors locally → his subscriber sees it.
        let local_id = bob_cabal
            .send_text_at("session", "bob local", 1)
            .await
            .unwrap();
        let got_local = bob_rx.try_recv().expect("local authored post emitted");
        assert_eq!(hash_post(&got_local), local_id);

        // Alice authors; bob ingests the post object → subscriber sees it.
        let remote_id = alice_cabal
            .send_text_at("session", "from alice", 2)
            .await
            .unwrap();
        let post = alice_cabal.get_post(&remote_id).unwrap();
        bob_cabal.ingest_post(post).await.unwrap();
        let got_remote = bob_rx.try_recv().expect("ingested post emitted");
        assert_eq!(hash_post(&got_remote), remote_id);

        assert!(bob_rx.try_recv().is_err(), "nothing else is pending");
    }

    #[tokio::test]
    async fn cabal_membership_folds_signed_join_leave_by_author_sequence() {
        let alice_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([70; 32]));
        let alice_id = PeerID::from_public_key(alice_provider.master_public_key());
        let alice = Murm::new(alice_provider, StubTransport::new(alice_id));

        let bob_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([71; 32]));
        let bob_id = PeerID::from_public_key(bob_provider.master_public_key());
        let bob = Murm::new(bob_provider, StubTransport::new(bob_id));

        let key = CabalKey::new([0xce; 32]);
        let alice_cabal = alice.open_cabal(&key).await.unwrap();
        let bob_cabal = bob.open_cabal(&key).await.unwrap();
        let alice_author = alice_cabal.author_public_key().unwrap().to_bytes();
        let bob_author = bob_cabal.author_public_key().unwrap().to_bytes();

        let alice_join = alice_cabal.send_join_at("secrets", 30).await.unwrap();
        let bob_join = bob_cabal.send_join_at("secrets", 10).await.unwrap();
        bob_cabal
            .ingest_post(alice_cabal.get_post(&alice_join).unwrap())
            .await
            .unwrap();
        alice_cabal
            .ingest_post(bob_cabal.get_post(&bob_join).unwrap())
            .await
            .unwrap();

        let alice_view = alice_cabal.membership("secrets");
        let bob_view = bob_cabal.membership("secrets");
        assert_eq!(alice_view, bob_view);
        assert!(alice_view.contains(&alice_author));
        assert!(alice_view.contains(&bob_author));
        let joined_revision = alice_view.revision;

        // Alice's later per-author operation wins even with an older asserted
        // timestamp; membership never uses wall-clock last-writer-wins.
        let alice_leave = alice_cabal.send_leave_at("secrets", 1).await.unwrap();
        bob_cabal
            .ingest_post(alice_cabal.get_post(&alice_leave).unwrap())
            .await
            .unwrap();
        let left = bob_cabal.membership("secrets");
        assert!(!left.contains(&alice_author));
        assert!(left.contains(&bob_author));
        assert_ne!(left.revision, joined_revision);
    }

    #[test]
    fn derive_cabal_keypair_uses_identity_provider() {
        let murm = make_murm();
        let cabal_key = CabalKey::new([1; 32]);

        // Two derivations from the same key produce equal public keys
        // (deterministic).
        let kp1 = murm.derive_cabal_keypair(&cabal_key).unwrap();
        let kp2 = murm.derive_cabal_keypair(&cabal_key).unwrap();
        assert_eq!(kp1.public_key().to_bytes(), kp2.public_key().to_bytes());

        // Different cabal key → different derived keypair.
        let other = CabalKey::new([2; 32]);
        let kp3 = murm.derive_cabal_keypair(&other).unwrap();
        assert_ne!(kp1.public_key().to_bytes(), kp3.public_key().to_bytes());
    }

    #[test]
    fn cabal_key_debug_redacts_bytes() {
        let key = CabalKey::new([0xaa; 32]);
        let debug = format!("{:?}", key);
        assert!(debug.contains("redacted"));
        assert!(!debug.contains("aa"));
    }

    // ─────────────────────────────────────────────────────────────────
    // End-to-end integration: Cable post roundtrip between two peers
    // ─────────────────────────────────────────────────────────────────

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use transport::memory::MemoryTransport;

    /// Read a LEB128 varint from an async reader.
    async fn read_varint_async(reader: &mut (impl AsyncReadExt + Unpin)) -> u64 {
        let mut value = 0u64;
        let mut shift = 0;
        let mut buf = [0u8; 1];
        loop {
            reader.read_exact(&mut buf).await.unwrap();
            let byte = buf[0];
            value |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        value
    }

    /// Encode a varint into a Vec.
    fn encode_varint_to_vec(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// Full pipeline test: Alice signs a Text post and sends it to Bob over
    /// the in-memory paired transport on the Cable ALPN. Bob reads, decodes,
    /// and verifies the signature. Validates: identity derivation, post
    /// signing, wire encoding, transport delivery, decoding, and signature
    /// verification — all the moving parts of Phase 2B working together.
    #[tokio::test]
    async fn cable_text_post_roundtrips_between_peers() {
        // Setup
        let alice_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([11; 32]));
        let bob_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([22; 32]));

        let alice_id = PeerID::from_public_key(alice_provider.master_public_key());
        let bob_id = PeerID::from_public_key(bob_provider.master_public_key());

        let (alice_t, bob_t) = MemoryTransport::pair(alice_id, bob_id);

        // Alice signs a post addressed to a (made-up for this test) cabal.
        let cabal_key = CabalKey::new([0x42; 32]);
        let cabal_id = [0x42; 32]; // opaque cabal id; this test exercises the wire
        let alice_kp = alice_provider.derive_keypair(cabal_key.as_bytes()).unwrap();

        let post = sign_post(
            &alice_kp,
            cabal_id,
            0,
            None,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "hello, bob, this is alice from a real signed post".to_string(),
                timestamp_ms: 1_700_000_000_000,
            },
        );
        let post_bytes = encode_post(&post);

        // Both peers open the Cable ALPN concurrently — Alice connects, Bob
        // accepts.
        let alpn = Alpn::new("mere/cable/v1");
        let (alice_stream_res, bob_stream_res) =
            tokio::join!(alice_t.connect(bob_id, alpn.clone()), bob_t.accept(alpn));
        let mut alice_stream = alice_stream_res.expect("alice connect failed");
        let mut bob_stream = bob_stream_res.expect("bob accept failed").into_stream();

        // Wire framing: varint length prefix, then post bytes.
        let len_prefix = encode_varint_to_vec(post_bytes.len() as u64);
        alice_stream.write_all(&len_prefix).await.unwrap();
        alice_stream.write_all(&post_bytes).await.unwrap();
        alice_stream.flush().await.unwrap();

        // Bob reads varint length, then the post bytes.
        let post_len = read_varint_async(&mut bob_stream).await as usize;
        assert_eq!(post_len, post_bytes.len());

        let mut received = vec![0u8; post_len];
        bob_stream.read_exact(&mut received).await.unwrap();
        assert_eq!(received, post_bytes, "transport delivered exact bytes");

        // Bob decodes the post.
        let decoded = decode_post(&received).expect("decode failed");

        // Bob verifies the signature.
        assert!(
            verify_post(&decoded),
            "signature should verify — Alice's keypair signed this"
        );

        // Bob can also confirm the author claim matches Alice's *cabal-derived*
        // public key (via Bob's own identity-provider derivation, since both
        // sides know the cabal key out-of-band).
        let bob_view_of_alice_cabal_pubkey = alice_provider
            .derive_keypair(cabal_key.as_bytes())
            .unwrap()
            .public_key();
        assert_eq!(
            decoded.author.to_bytes(),
            bob_view_of_alice_cabal_pubkey.to_bytes(),
            "author should match the cabal-derived public key"
        );

        // And the content.
        match &decoded.kind {
            PostKind::Text {
                channel,
                text,
                timestamp_ms,
            } => {
                assert_eq!(channel.as_str(), "session");
                assert_eq!(text, "hello, bob, this is alice from a real signed post");
                assert_eq!(*timestamp_ms, 1_700_000_000_000);
            },
            _ => panic!("expected Text post"),
        }
    }

    /// Same shape but tampering the bytes mid-transit should make Bob's
    /// verification fail. Demonstrates the integrity guarantee end-to-end.
    #[tokio::test]
    async fn tampered_post_in_transit_fails_verification() {
        let alice_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([33; 32]));
        let bob_provider: Arc<dyn IdentityProvider> =
            Arc::new(identity::InMemoryProvider::from_seed([44; 32]));

        let alice_id = PeerID::from_public_key(alice_provider.master_public_key());
        let bob_id = PeerID::from_public_key(bob_provider.master_public_key());

        let (alice_t, bob_t) = MemoryTransport::pair(alice_id, bob_id);

        let alice_kp = alice_provider
            .derive_keypair(b"some-cabal-salt-32-bytes-please.")
            .unwrap();
        let post = sign_post(
            &alice_kp,
            [0x42; 32],
            0,
            None,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "original message".to_string(),
                timestamp_ms: 1,
            },
        );
        let mut post_bytes = encode_post(&post);

        // Tamper: flip a byte partway through the encoded operation. Bob must
        // reject it, whether that surfaces as a decode failure or a bad
        // signature.
        let mid = post_bytes.len() / 2;
        post_bytes[mid] ^= 0x01;

        let alpn = Alpn::new("mere/cable/v1");
        let (alice_stream_res, bob_stream_res) =
            tokio::join!(alice_t.connect(bob_id, alpn.clone()), bob_t.accept(alpn));
        let mut alice_stream = alice_stream_res.unwrap();
        let mut bob_stream = bob_stream_res.unwrap().into_stream();

        let len_prefix = encode_varint_to_vec(post_bytes.len() as u64);
        alice_stream.write_all(&len_prefix).await.unwrap();
        alice_stream.write_all(&post_bytes).await.unwrap();
        alice_stream.flush().await.unwrap();

        let post_len = read_varint_async(&mut bob_stream).await as usize;
        let mut received = vec![0u8; post_len];
        bob_stream.read_exact(&mut received).await.unwrap();

        let detected = match decode_post(&received) {
            Ok(post) => !verify_post(&post),
            Err(_) => true,
        };
        assert!(
            detected,
            "tampered post must be rejected (decode failure or bad signature)"
        );
    }
}
