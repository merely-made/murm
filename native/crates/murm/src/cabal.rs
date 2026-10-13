// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cabal-level types for bilateral comms.
//!
//! A *cabal* in Murm terminology is a long-lived bilateral or small-group
//! conversation in Mere's Cable-shaped dialect. These words name the domain
//! grammar; they do not assert cabal-club wire compatibility. A cabal has:
//!
//! - A 32-byte symmetric **cabal key** ([`CabalKey`], shared secret,
//!   distributed out-of-band among members)
//! - A **cabal id** ([`CabalId`], public identifier, derived from the key
//!   via BLAKE3 — used for addressing and discovery without revealing the
//!   secret)
//! - One or more **named channels** (per Cable spec §2.5: `"session"`,
//!   `"links"`, etc.)
//! - A persistent or ephemeral post store (Phase 2B uses in-memory)
//!
//! Per Cable spec §2.2, the user's per-cabal Ed25519 keypair is derived
//! from `BLAKE3(master_secret || cabal_key)` — that derivation lives in
//! [`identity::IdentityProvider::derive_keypair`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::MurmError;
use stickleback::{DropImportReport, DropLimits};

use crate::{
    CabalKeyEpoch, CabalKeyring, ConversationEngine, ConversationRefresh, InfoEntry, Post, PostId,
    PostKind, hash_post,
};

/// Current wall-clock time as milliseconds since UNIX epoch. Returns 0 if
/// the system clock is before the epoch (effectively unreachable on
/// real systems).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A cabal's symmetric secret key (32 bytes).
///
/// Distributed out-of-band to members (QR code, invite link, mDNS, etc.).
/// Knowledge of the key is what makes you a cabal member at the protocol
/// level.
///
/// **Security note**: cabal keys are secrets in transit and at rest. Any
/// local persistence should be keychain-protected.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct CabalKey(pub [u8; 32]);

impl CabalKey {
    /// Construct from raw bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// View as bytes.
    ///
    /// **Use cautiously**: this exposes the secret cabal key. Prefer
    /// derived values ([`CabalId`], the per-cabal keypair via
    /// `derive_keypair`) over raw key bytes wherever possible.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// Don't auto-derive Debug — it would print the secret bytes. A redacted
// manual impl is safer.
impl std::fmt::Debug for CabalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CabalKey(<redacted>)")
    }
}

/// A cabal's public identifier (`BLAKE3(cabal_key)`).
///
/// Suitable as a routing identifier without revealing the secret. Two
/// peers who hold the same cabal key compute the same `CabalId`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CabalId(pub [u8; 32]);

impl CabalId {
    /// Construct from raw bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// View as bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Deterministic current membership of one cabal channel.
///
/// Members are per-cabal author public keys, derived by each persona from the
/// cabal key. The revision commits to the latest signed Join/Leave operation
/// for every author, so consumers can bind private state to a frozen audience.
/// This is an audience projection, not key revocation: removing a member must
/// be followed by a new cabal key before future posts are confidential from a
/// former key holder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CabalMembership {
    /// Channel whose signed Join/Leave records were folded.
    pub channel: String,
    /// Active per-cabal author public keys.
    pub members: BTreeSet<[u8; 32]>,
    /// BLAKE3 commitment to the current per-author membership states.
    pub revision: [u8; 32],
}

impl CabalMembership {
    /// Whether a per-cabal author public key is currently joined.
    pub fn contains(&self, author: &[u8; 32]) -> bool {
        self.members.contains(author)
    }
}

/// Handle to an open cabal.
///
/// Returned by [`crate::Murm::open_cabal`]. Holds the cabal's public id and
/// a shared reference to the underlying [`ConversationEngine`] that backs the
/// cabal's storage and signing. `CabalHandle` is `Send + Sync + 'static`
/// (no lifetime tied to `Murm`), so you can clone it across tasks.
#[derive(Clone)]
pub struct CabalHandle {
    cabal_id: CabalId,
    engine: Arc<ConversationEngine>,
}

impl CabalHandle {
    /// Construct a cabal handle. Internal — `Murm::open_cabal` creates these.
    pub(crate) fn new(cabal_id: CabalId, engine: Arc<ConversationEngine>) -> Self {
        Self { cabal_id, engine }
    }

    /// The cabal's public identifier.
    pub fn id(&self) -> &CabalId {
        &self.cabal_id
    }

    /// The author public key this user posts under in this cabal —
    /// derived from `(master_secret, cabal_key)` per Cable spec §2.2.
    pub fn author_public_key(&self) -> Result<identity::Ed25519PublicKey, MurmError> {
        self.engine.author_public_key(self.cabal_id.as_bytes())
    }

    /// The cabal's epoch-aware native-drop protector.
    pub fn keyring(&self) -> Result<CabalKeyring, MurmError> {
        self.engine.keyring(self.cabal_id.as_bytes())
    }

    /// Install or withdraw retention authority resolved by this cabal's owner.
    pub async fn set_checkpoint_authority(
        &self,
        authority: Option<crate::CabalCheckpointAuthority>,
    ) -> Result<(), MurmError> {
        self.engine
            .set_checkpoint_authority(self.cabal_id.as_bytes(), authority)
            .await
    }

    /// Remove this author's earlier log through an explicitly authorized root.
    pub async fn retention_checkpoint_at(
        &self,
        authority_revision: proofs::Digest,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_retention_checkpoint(self.cabal_id.as_bytes(), authority_revision, timestamp_ms)
            .await
    }

    /// Install the next key produced by the cabal's group-state owner.
    ///
    /// This is deliberately an installation hook, not an authorization API:
    /// p2panda DCGKA / Moot policy decides whether a rotation is valid.
    pub fn install_key_epoch(&self, epoch: CabalKeyEpoch, key: [u8; 32]) -> Result<(), MurmError> {
        self.engine
            .install_key_epoch(self.cabal_id.as_bytes(), epoch, key)
    }

    /// Compose, sign, and store a `post/text` message at the current wall
    /// clock.
    pub async fn send_text(&self, channel: &str, text: &str) -> Result<PostId, MurmError> {
        self.send_text_at(channel, text, now_ms()).await
    }

    /// Compose, sign, and store a `post/text` message with an explicit
    /// timestamp (useful for tests and replays).
    pub async fn send_text_at(
        &self,
        channel: &str,
        text: &str,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_text(self.cabal_id.as_bytes(), channel, text, timestamp_ms)
            .await
    }

    /// Compose, sign, and store a `post/topic` at the current wall clock.
    pub async fn send_topic(&self, channel: &str, topic: &str) -> Result<PostId, MurmError> {
        self.send_topic_at(channel, topic, now_ms()).await
    }

    /// Compose, sign, and store a `post/topic` with an explicit timestamp.
    pub async fn send_topic_at(
        &self,
        channel: &str,
        topic: &str,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_topic(self.cabal_id.as_bytes(), channel, topic, timestamp_ms)
            .await
    }

    /// Compose, sign, and store a `post/join` at the current wall clock.
    pub async fn send_join(&self, channel: &str) -> Result<PostId, MurmError> {
        self.send_join_at(channel, now_ms()).await
    }

    /// Compose, sign, and store a `post/join` with an explicit timestamp.
    pub async fn send_join_at(
        &self,
        channel: &str,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_join(self.cabal_id.as_bytes(), channel, timestamp_ms)
            .await
    }

    /// Compose, sign, and store a `post/leave` at the current wall clock.
    pub async fn send_leave(&self, channel: &str) -> Result<PostId, MurmError> {
        self.send_leave_at(channel, now_ms()).await
    }

    /// Compose, sign, and store a `post/leave` with an explicit timestamp.
    pub async fn send_leave_at(
        &self,
        channel: &str,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_leave(self.cabal_id.as_bytes(), channel, timestamp_ms)
            .await
    }

    /// Compose, sign, and store a `post/info` at the current wall clock.
    pub async fn send_info(&self, entries: Vec<InfoEntry>) -> Result<PostId, MurmError> {
        self.send_info_at(entries, now_ms()).await
    }

    /// Compose, sign, and store a `post/info` with an explicit timestamp.
    pub async fn send_info_at(
        &self,
        entries: Vec<InfoEntry>,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_info(self.cabal_id.as_bytes(), entries, timestamp_ms)
            .await
    }

    /// Compose, sign, and store a `post/delete` at the current wall clock.
    pub async fn send_delete(&self, posts: Vec<PostId>) -> Result<PostId, MurmError> {
        self.send_delete_at(posts, now_ms()).await
    }

    /// Compose, sign, and store a `post/delete` with an explicit timestamp.
    pub async fn send_delete_at(
        &self,
        posts: Vec<PostId>,
        timestamp_ms: u64,
    ) -> Result<PostId, MurmError> {
        self.engine
            .post_delete(self.cabal_id.as_bytes(), posts, timestamp_ms)
            .await
    }

    /// Look up a post by id.
    pub fn get_post(&self, post_id: &PostId) -> Option<Post> {
        self.engine.get_post(self.cabal_id.as_bytes(), post_id)
    }

    /// Legacy materialized channel history, ordered by asserted time and post id.
    /// Use [`Self::causal_history`] for a causally ordered application read.
    pub fn history(&self, channel: &str) -> Vec<Post> {
        self.engine.history(self.cabal_id.as_bytes(), channel)
    }

    /// A causally complete read over retained headers and available bodies.
    /// Reports missing history separately; never grants access or removes data.
    pub async fn causal_history(&self, channel: &str) -> Result<crate::CabalHistory, MurmError> {
        self.engine
            .causal_history(self.cabal_id.as_bytes(), channel)
            .await
    }

    /// Fold signed Join/Leave posts into the channel's current audience.
    ///
    /// Each author's highest per-cabal sequence number wins for that author;
    /// cross-author wall-clock timestamps never decide membership. The result
    /// therefore converges independent of arrival order.
    pub fn membership(&self, channel: &str) -> CabalMembership {
        let mut latest: BTreeMap<[u8; 32], (u64, bool, PostId)> = BTreeMap::new();
        for post in self.history(channel) {
            let joined = match &post.kind {
                PostKind::Join { .. } => true,
                PostKind::Leave { .. } => false,
                _ => continue,
            };
            let author = post.author.to_bytes();
            let post_id = hash_post(&post);
            let post_seq_num = u64::from(post.seq_num);
            let replace = match latest.get(&author) {
                Some((seq_num, _, prior_id)) => {
                    post_seq_num > *seq_num
                        || (post_seq_num == *seq_num && post_id.as_bytes() > prior_id.as_bytes())
                },
                None => true,
            };
            if replace {
                latest.insert(author, (post_seq_num, joined, post_id));
            }
        }

        let mut hasher = blake3::Hasher::new_derive_key("mere.murm.cabal-membership.v1");
        hasher.update(&(channel.len() as u64).to_le_bytes());
        hasher.update(channel.as_bytes());
        for (author, (seq_num, joined, post_id)) in &latest {
            hasher.update(author);
            hasher.update(&seq_num.to_le_bytes());
            hasher.update(&[u8::from(*joined)]);
            hasher.update(post_id.as_bytes());
        }
        CabalMembership {
            channel: channel.to_owned(),
            members: latest
                .into_iter()
                .filter_map(|(author, (_, joined, _))| joined.then_some(author))
                .collect(),
            revision: *hasher.finalize().as_bytes(),
        }
    }

    /// Subscribe to posts as they land in this cabal.
    ///
    /// The receiver yields each post stored *after* it subscribes — authored
    /// locally (here or via a [`SyncedCabal`](crate::SyncedCabal)), gossiped by a
    /// peer, or caught up via LogSync — once each. It does not replay the
    /// backlog; load it with [`history`](Self::history) first, then drive the live
    /// view from this stream (dedup by [`PostId`] is cheap, posts being
    /// content-addressed). On a `Lagged` error the consumer should re-read
    /// `history` to reconcile.
    pub fn subscribe(&self) -> Result<tokio::sync::broadcast::Receiver<Post>, MurmError> {
        self.engine.subscribe(self.cabal_id.as_bytes())
    }

    /// Inject a post arrived from a peer (e.g. via transport sync).
    ///
    /// Verifies the signature, computes the post id, and stores it.
    /// Returns the computed post id on success; rejects with
    /// an admission error if the signature is invalid.
    pub async fn ingest_post(&self, post: Post) -> Result<PostId, MurmError> {
        self.engine
            .ingest_post(self.cabal_id.as_bytes(), post)
            .await
    }

    /// Import a verified native plaintext/public drop and refresh live views.
    ///
    /// Newly materialized posts are emitted to subscribers exactly once. The
    /// returned refresh report also exposes removals if retention changed the
    /// underlying store before this reconciliation.
    pub async fn import_plain_drop<R: Read>(
        &self,
        reader: R,
        limits: DropLimits,
    ) -> Result<(DropImportReport, ConversationRefresh), MurmError> {
        self.engine
            .import_plain_drop(self.cabal_id.as_bytes(), reader, limits)
            .await
    }

    /// Import a native drop protected by this cabal's retained key epochs.
    ///
    /// The envelope selects its epoch. Old drops remain readable until the
    /// corresponding key is explicitly forgotten. View refresh and subscriber
    /// delivery match [`import_plain_drop`](Self::import_plain_drop).
    pub async fn import_protected_drop<R: Read>(
        &self,
        reader: R,
        limits: DropLimits,
    ) -> Result<(DropImportReport, ConversationRefresh), MurmError> {
        self.engine
            .import_protected_drop(self.cabal_id.as_bytes(), reader, limits)
            .await
    }

    /// Reconcile materialized history after advanced out-of-band store work.
    pub async fn refresh(&self) -> Result<ConversationRefresh, MurmError> {
        self.engine.refresh(self.cabal_id.as_bytes()).await
    }
}
