// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cable-shaped post types, post IDs, and channel names.
//!
//! These types retain the chat shapes from the upstream
//! [Cable spec](https://github.com/cabal-club/cable), with an explicit native
//! retention checkpoint. Murm's wire is signed p2panda operations; these shapes
//! do not assert literal Cable wire compatibility.

use identity::{Ed25519PublicKey, Ed25519Signature};

use crate::MurmError;

/// Maximum channel name length in bytes. Picked to fit in a single varint
/// byte and to match common chat-protocol limits. The Cable spec is silent
/// on this but Cable implementations conventionally enforce a limit; 255
/// is more than generous for channel-naming use.
pub const MAX_CHANNEL_NAME_BYTES: usize = 255;

/// A post identifier — its signed p2panda header hash, binding the payload
/// commitment as well as the author's causal metadata (see [`crate::hash_post`]).
///
/// Per Cable's content-addressing model, posts are identified by their
/// hash; references to posts (in subsequent posts' `links` field, or in
/// `post/delete` payloads) carry these 32-byte hashes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PostId(pub [u8; 32]);

impl PostId {
    /// Construct from raw bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// View as bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A channel name within a cabal.
///
/// Per Cable spec §2.5, posts are organized into named channels. Co-op
/// session minichat uses `"session"`; named cabals may use multiple
/// channels (`"links"`, `"notes"`, etc.).
///
/// The wire encoding is a varint length prefix followed by UTF-8 bytes.
///
/// ## Validation
///
/// - Non-empty
/// - At most [`MAX_CHANNEL_NAME_BYTES`] bytes (UTF-8 length, not
///   codepoint count)
/// - No ASCII control characters (`< 0x20` or `0x7F`) — these are
///   typically problematic in chat-style names and the Cable spec doesn't
///   require their support
///
/// [`ChannelName::new`] is permissive (always succeeds; the channel name
/// may be invalid but tests of older Murm versions still compile).
/// [`ChannelName::try_new`] validates and returns
/// [`MurmError::MissingField`] on failure.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ChannelName(String);

impl ChannelName {
    /// Construct a channel name without validation.
    ///
    /// Useful for tests and for round-tripping channel names from peers
    /// (we accept what we receive even if it doesn't pass our local
    /// validation — the peer might use slightly different rules). For
    /// user-facing input, prefer [`ChannelName::try_new`].
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Construct a channel name, validating against the rules in the
    /// type-level docs.
    ///
    /// Returns [`MurmError::MissingField`] (with a description of
    /// the failing rule) if the name is invalid.
    pub fn try_new(name: impl Into<String>) -> Result<Self, MurmError> {
        let name = name.into();
        if name.is_empty() {
            return Err(MurmError::MissingField("channel name (empty)"));
        }
        if name.len() > MAX_CHANNEL_NAME_BYTES {
            return Err(MurmError::MissingField("channel name (too long)"));
        }
        if name
            .chars()
            .any(|c| (c as u32) < 0x20 || (c as u32) == 0x7F)
        {
            return Err(MurmError::MissingField("channel name (control character)"));
        }
        Ok(Self(name))
    }

    /// Check whether a string is a valid channel name without
    /// constructing.
    pub fn is_valid_name(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_CHANNEL_NAME_BYTES
            && !s.chars().any(|c| (c as u32) < 0x20 || (c as u32) == 0x7F)
    }

    /// View as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A single key-value entry in a `post/info` payload.
///
/// Per Cable spec §post/info, info posts carry a *list* of key-value pairs
/// rather than a fixed set of fields. Standard keys include:
///
/// - `"name"` — UTF-8 display name (defaults to hex-encoded public key)
/// - `"accept-role"` — varint (`0` or `1`; default `1`)
///
/// Custom keys are permitted; receivers ignore unknown keys.
///
/// **Validation note**: per the spec, keys are UTF-8 strings (1–128
/// codepoints), values are bytes (max 4096). The types here are
/// permissive; encode-time validation lands in a later phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InfoEntry {
    /// Key name (UTF-8).
    pub key: String,
    /// Value bytes (interpretation key-specific).
    pub value: Vec<u8>,
}

impl InfoEntry {
    /// Construct an entry from a key and value.
    pub fn new(key: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Construct a standard `name` entry from a display-name string.
    pub fn name(name: impl AsRef<str>) -> Self {
        Self {
            key: "name".to_string(),
            value: name.as_ref().as_bytes().to_vec(),
        }
    }
}

/// A signed Cable post.
///
/// On the wire a post is a p2panda-core operation (see [`crate::post_wire`]):
/// content-addressed via its `PostId` (the operation's **signed-header hash** =
/// `Header::hash()`, which transitively binds the body via `payload_hash`),
/// belonging to a cabal (`cabal_id`), positioned in its author's per-cabal log
/// (`seq_num` / `backlink`), and forming a cross-author causal DAG via `links`.
/// The author's signature covers the whole operation header, including
/// `cabal_id`, `seq_num`, `backlink`, `links`, the kind metadata, and a hash of
/// the body.
///
/// ## Two notions of order
///
/// - `seq_num` / `backlink` are the **per-author append-only log** (one chain
///   per `(author, cabal)`): the unit p2panda-net LogSync reconciles. Each
///   author's operations form a single hash-linked chain.
/// - `links` are **cross-author causal references** (the DAG view), layered on
///   top of the per-author logs.
#[derive(Clone, Debug)]
pub struct Post {
    /// The author's public key.
    pub author: Ed25519PublicKey,
    /// The cabal (space) this post belongs to: `BLAKE3(cabal_key)`. Carried in
    /// the signed operation header, so a post is self-describing and cannot be
    /// replayed into a different cabal.
    pub cabal_id: [u8; 32],
    /// Position in this author's per-cabal log: `0` for the author's first
    /// operation in the cabal, incrementing by exactly 1 thereafter. Signed.
    pub seq_num: u32,
    /// The author's previous operation in this cabal (its `PostId`), or `None`
    /// for the first. When set it must equal that operation's signed-header
    /// hash; per the p2panda log rule, `backlink.is_some()` iff `seq_num > 0`.
    pub backlink: Option<PostId>,
    /// Cross-author causal-DAG predecessors (operation ids of preceding posts).
    pub links: Vec<PostId>,
    /// The post's payload kind (also implicitly carries `post_type` and
    /// `timestamp_ms`).
    pub kind: PostKind,
    /// The author's Ed25519 signature over the canonical operation header.
    ///
    /// A cached view of the signature inside [`Post::header`]; the header bytes
    /// are authoritative.
    pub signature: Ed25519Signature,
    /// The canonical encoded operation header this post was signed as.
    ///
    /// p2panda 0.7.1 made `Header`'s CBOR cache, size and digest private, so a
    /// signed header can no longer be rebuilt from its parts — it can only be
    /// decoded. Carrying the bytes keeps `operation_id`, `post_to_operation`
    /// and `encode_post` exact, and decoding them re-verifies the signature as
    /// a side effect, which is what `verify_post` now rests on.
    pub header: Vec<u8>,
}

/// The payload variant of a [`Post`].
///
/// Carries the `post_type` discriminator and `timestamp_ms` together with
/// the type-specific body, mirroring the Cable wire format. The
/// integer-valued `post_type` per Cable spec is:
///
/// - `0` = [`PostKind::Text`]
/// - `1` = [`PostKind::Delete`]
/// - `2` = [`PostKind::Info`]
/// - `3` = [`PostKind::Topic`]
/// - `4` = [`PostKind::Join`]
/// - `5` = [`PostKind::Leave`]
/// - `6` = [`PostKind::RetentionCheckpoint`] (Murm extension)
///
/// (See [`PostKind::post_type`] for programmatic access.)
#[derive(Clone, Debug)]
pub enum PostKind {
    /// `post_type = 0`. A chat text message.
    Text {
        /// Channel this post belongs to.
        channel: ChannelName,
        /// UTF-8 text body. Cable spec caps at 4096 bytes.
        text: String,
        /// Author-asserted millisecond Unix timestamp.
        timestamp_ms: u64,
    },
    /// `post_type = 1`. Author-initiated deletion of one or more earlier
    /// posts. The author can only delete posts they themselves authored
    /// (receivers enforce).
    Delete {
        /// The posts to delete (referenced by hash).
        posts: Vec<PostId>,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
    /// `post_type = 2`. Sets or replaces the author's metadata. Each new
    /// `Info` post completely replaces prior metadata for that author.
    Info {
        /// Key-value entries (e.g. `name`, `accept-role`).
        entries: Vec<InfoEntry>,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
    /// `post_type = 3`. Sets a channel topic. Empty `topic` clears.
    Topic {
        /// Channel whose topic is being set.
        channel: ChannelName,
        /// New topic text (UTF-8, 0–512 codepoints per spec).
        topic: String,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
    /// `post_type = 4`. Author entering a channel.
    Join {
        /// Channel being joined.
        channel: ChannelName,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
    /// `post_type = 5`. Author leaving a channel.
    Leave {
        /// Channel being left.
        channel: ChannelName,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
    /// Murm extension `post_type = 6`: an explicitly authorized root of this
    /// author's retained log. It removes the author's earlier headers and
    /// bodies, not other authors' records or a materialized community snapshot.
    RetentionCheckpoint {
        /// Exact authority revision supplied by the conversation owner.
        authority_revision: proofs::Digest,
        /// Author-asserted timestamp.
        timestamp_ms: u64,
    },
}

impl PostKind {
    /// The author-asserted timestamp for this post kind.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            PostKind::Text { timestamp_ms, .. }
            | PostKind::Delete { timestamp_ms, .. }
            | PostKind::Info { timestamp_ms, .. }
            | PostKind::Topic { timestamp_ms, .. }
            | PostKind::Join { timestamp_ms, .. }
            | PostKind::Leave { timestamp_ms, .. }
            | PostKind::RetentionCheckpoint { timestamp_ms, .. } => *timestamp_ms,
        }
    }

    /// The channel this post belongs to, if any. Channel-less variants
    /// ([`PostKind::Info`], [`PostKind::Delete`]) return `None`.
    pub fn channel(&self) -> Option<&ChannelName> {
        match self {
            PostKind::Text { channel, .. }
            | PostKind::Topic { channel, .. }
            | PostKind::Join { channel, .. }
            | PostKind::Leave { channel, .. } => Some(channel),
            PostKind::Info { .. }
            | PostKind::Delete { .. }
            | PostKind::RetentionCheckpoint { .. } => None,
        }
    }

    /// The Cable-shaped `post_type` discriminator (0–5) or Murm extension (6).
    pub fn post_type(&self) -> u64 {
        match self {
            PostKind::Text { .. } => 0,
            PostKind::Delete { .. } => 1,
            PostKind::Info { .. } => 2,
            PostKind::Topic { .. } => 3,
            PostKind::Join { .. } => 4,
            PostKind::Leave { .. } => 5,
            PostKind::RetentionCheckpoint { .. } => 6,
        }
    }

    /// A short discriminant string for logging / diagnostics.
    pub fn kind_name(&self) -> &'static str {
        match self {
            PostKind::Text { .. } => "text",
            PostKind::Delete { .. } => "delete",
            PostKind::Info { .. } => "info",
            PostKind::Topic { .. } => "topic",
            PostKind::Join { .. } => "join",
            PostKind::Leave { .. } => "leave",
            PostKind::RetentionCheckpoint { .. } => "retention-checkpoint",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_id_round_trips_through_bytes() {
        let bytes = [42u8; 32];
        let id = PostId::new(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn channel_name_validation_covers_limits_and_controls() {
        assert_eq!(ChannelName::try_new("session").unwrap().as_str(), "session");
        assert!(ChannelName::try_new("会議").is_ok());
        assert!(matches!(
            ChannelName::try_new(""),
            Err(MurmError::MissingField(_))
        ));
        assert!(matches!(
            ChannelName::try_new("a\nline"),
            Err(MurmError::MissingField(_))
        ));
        assert!(matches!(
            ChannelName::try_new("a\x7fb"),
            Err(MurmError::MissingField(_))
        ));
        assert!(ChannelName::try_new("a".repeat(255)).is_ok());
        assert!(matches!(
            ChannelName::try_new("a".repeat(256)),
            Err(MurmError::MissingField(_))
        ));
    }

    #[test]
    fn channel_name_predicate_matches_constructor() {
        assert!(ChannelName::is_valid_name("session"));
        assert!(ChannelName::is_valid_name("会議"));
        assert!(!ChannelName::is_valid_name(""));
        assert!(!ChannelName::is_valid_name("a\nb"));
        assert!(!ChannelName::is_valid_name(&"a".repeat(256)));
    }

    #[test]
    fn post_kind_accessors_cover_channel_and_channelless_posts() {
        let text = PostKind::Text {
            channel: ChannelName::new("session"),
            text: "hello".to_string(),
            timestamp_ms: 1_700_000_000_000,
        };
        assert_eq!(text.timestamp_ms(), 1_700_000_000_000);
        assert_eq!(text.channel().unwrap().as_str(), "session");
        assert_eq!(text.kind_name(), "text");

        let info = PostKind::Info {
            entries: vec![InfoEntry::name("alice")],
            timestamp_ms: 0,
        };
        assert!(info.channel().is_none());

        let delete = PostKind::Delete {
            posts: vec![PostId::new([0; 32])],
            timestamp_ms: 0,
        };
        assert!(delete.channel().is_none());
    }

    #[test]
    fn all_post_kinds_have_distinct_names_and_wire_discriminants() {
        let kinds = [
            PostKind::Text {
                channel: ChannelName::new("c"),
                text: String::new(),
                timestamp_ms: 0,
            },
            PostKind::Delete {
                posts: vec![PostId::new([0; 32])],
                timestamp_ms: 0,
            },
            PostKind::Info {
                entries: vec![],
                timestamp_ms: 0,
            },
            PostKind::Topic {
                channel: ChannelName::new("c"),
                topic: String::new(),
                timestamp_ms: 0,
            },
            PostKind::Join {
                channel: ChannelName::new("c"),
                timestamp_ms: 0,
            },
            PostKind::Leave {
                channel: ChannelName::new("c"),
                timestamp_ms: 0,
            },
            PostKind::RetentionCheckpoint {
                authority_revision: proofs::Digest::blake3(b"policy"),
                timestamp_ms: 0,
            },
        ];

        let names: std::collections::HashSet<_> = kinds.iter().map(PostKind::kind_name).collect();
        let discriminants: std::collections::HashSet<_> =
            kinds.iter().map(PostKind::post_type).collect();
        assert_eq!(names.len(), 7);
        assert_eq!(discriminants.len(), 7);
    }
}
