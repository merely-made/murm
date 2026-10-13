// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cabal post wire encoding — p2panda-core Operations (BLAKE3, CBOR, Ed25519).
//!
//! A murm "post" is the logical chat event; on the wire it is a p2panda-core
//! operation (a signed [`Header<CabalExt>`] plus an optional body), the same
//! event type the rest of the substrate (p2panda-net sync, the cabal-space
//! model) is built on. This is the §12 "Cable on our stack" promotion: the
//! shared-secret bilateral trust model, but BLAKE3 / CBOR / p2panda operations
//! throughout, not the literal Cable varint wire.
//!
//! ## Mapping
//!
//! A [`Post`] maps to an operation as:
//!
//! - `author`    ↔ `Header::verifying_key`
//! - `signature` ↔ `Header::signature` (Ed25519 over the canonical header)
//! - `seq_num` / `backlink` ↔ the per-author p2panda log
//! - `links`     ↔ `CabalExt::parents` (complete observed causal frontier)
//! - `kind`      ↔ `CabalExt::{channel, post_type, timestamp_ms, info, deletes}`
//!   plus the operation `Body` (text / topic bytes; empty for the others)
//!
//! The post id ([`crate::post_hash`]) stays the plain BLAKE3-256 of these
//! canonical CBOR bytes.
//!
//! The construction is deterministic in `(author, links, kind)`, so signing
//! ([`crate::post_sign`]), verification, and encoding all reconstruct
//! byte-identical headers — content addressing is stable across peers.

use identity::{Ed25519PublicKey, Ed25519Signature};
use p2panda_core::cbor::{decode_cbor_strict, encode_cbor};
use p2panda_core::prune::PruneFlag;
use p2panda_core::{Body, Hash, Header, Operation, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{ChannelName, InfoEntry, MurmError, Post, PostId, PostKind};

/// Cabal-event metadata carried in the signed p2panda `Header` extension.
///
/// Everything needed to reconstruct a [`PostKind`] except the free content
/// (text / topic), which rides in the operation `Body` and is bound into the
/// signature via the header's `payload_hash`.
///
/// Public because a cabal post *is* an `Operation<CabalExt>`: the sync wiring
/// (in `murm`) receives these operations from p2panda-net's LogSync and converts
/// them back to [`Post`]s via [`operation_to_post`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CabalExt {
    /// The cabal (space) this event belongs to: `BLAKE3(cabal_key)`. Signed, so
    /// the event is self-describing and can't be replayed into another cabal.
    pub cabal_id: [u8; 32],
    /// Channel name (`""` for channel-less kinds: info, delete).
    pub channel: String,
    /// Cable-shaped discriminator (0..=5); 6 is Murm's retention extension.
    pub post_type: u64,
    /// Author-asserted millisecond timestamp.
    pub timestamp_ms: u64,
    /// Causal-DAG predecessors — the post's `links`.
    pub parents: Vec<[u8; 32]>,
    /// `post/info` entries (only populated for `post_type == 2`).
    pub info: Vec<(String, Vec<u8>)>,
    /// `post/delete` targets (only populated for `post_type == 1`).
    pub deletes: Vec<[u8; 32]>,
    /// Upstream p2panda prune marker, admitted only on an authorized checkpoint.
    /// Omitted when false, preserving ordinary posts' existing wire bytes.
    #[serde(default, skip_serializing_if = "PruneFlag::is_not_set")]
    pub prune_flag: PruneFlag,
    /// Signed authority revision for a retention checkpoint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_revision: Option<proofs::Digest>,
}

/// The canonical wire form: a signed header plus its body bytes, CBOR-encoded
/// as one blob.
///
/// The header travels as its own encoded bytes. p2panda 0.7.1 dropped the serde
/// impls on `Header`, and decoding the bytes back through `Header::decode`
/// verifies the signature, which serde never did.
#[derive(Serialize, Deserialize)]
struct WirePost {
    // `serde_bytes` so the header travels as a CBOR byte string; a bare
    // `Vec<u8>` would serialise as an array of integers, roughly doubling it.
    #[serde(with = "serde_bytes")]
    header: Vec<u8>,
    body: Vec<u8>,
}

// ── identity ed25519 ↔ p2panda-core ed25519 ──────────────────────────────────
// Both are ed25519-dalek underneath; convert through canonical bytes.

fn from_p2_vk(vk: &VerifyingKey) -> Result<Ed25519PublicKey, MurmError> {
    Ed25519PublicKey::from_bytes(vk.as_bytes()).map_err(|_| MurmError::MalformedPost)
}

pub(crate) fn from_p2_sig(sig: &Signature) -> Ed25519Signature {
    Ed25519Signature::from_bytes(&sig.to_bytes())
}

/// Split a [`PostKind`] (+ cabal id, links) into the header extension and body.
pub(crate) fn decompose(
    cabal_id: [u8; 32],
    links: &[PostId],
    kind: &PostKind,
) -> (CabalExt, Vec<u8>) {
    let parents: Vec<[u8; 32]> = links.iter().map(|l| *l.as_bytes()).collect();
    let mut ext = CabalExt {
        cabal_id,
        channel: String::new(),
        post_type: kind.post_type(),
        timestamp_ms: kind.timestamp_ms(),
        parents,
        info: Vec::new(),
        deletes: Vec::new(),
        prune_flag: PruneFlag::default(),
        authority_revision: None,
    };
    let body = match kind {
        PostKind::Text { channel, text, .. } => {
            ext.channel = channel.as_str().to_string();
            text.as_bytes().to_vec()
        },
        PostKind::Topic { channel, topic, .. } => {
            ext.channel = channel.as_str().to_string();
            topic.as_bytes().to_vec()
        },
        PostKind::Join { channel, .. } | PostKind::Leave { channel, .. } => {
            ext.channel = channel.as_str().to_string();
            Vec::new()
        },
        PostKind::Info { entries, .. } => {
            ext.info = entries
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect();
            Vec::new()
        },
        PostKind::Delete { posts, .. } => {
            ext.deletes = posts.iter().map(|p| *p.as_bytes()).collect();
            Vec::new()
        },
        PostKind::RetentionCheckpoint {
            authority_revision, ..
        } => {
            ext.prune_flag = PruneFlag::new(true);
            ext.authority_revision = Some(authority_revision.clone());
            Vec::new()
        },
    };
    (ext, body)
}

/// Rebuild a [`PostKind`] from a header extension and body bytes.
fn recompose(ext: &CabalExt, body: &[u8]) -> Result<PostKind, MurmError> {
    let timestamp_ms = ext.timestamp_ms;
    let text_body = || String::from_utf8(body.to_vec()).map_err(|_| MurmError::MalformedPost);
    match ext.post_type {
        0 => Ok(PostKind::Text {
            channel: ChannelName::new(ext.channel.clone()),
            text: text_body()?,
            timestamp_ms,
        }),
        1 => Ok(PostKind::Delete {
            posts: ext.deletes.iter().map(|d| PostId::new(*d)).collect(),
            timestamp_ms,
        }),
        2 => Ok(PostKind::Info {
            entries: ext
                .info
                .iter()
                .map(|(k, v)| InfoEntry {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect(),
            timestamp_ms,
        }),
        3 => Ok(PostKind::Topic {
            channel: ChannelName::new(ext.channel.clone()),
            topic: text_body()?,
            timestamp_ms,
        }),
        4 => Ok(PostKind::Join {
            channel: ChannelName::new(ext.channel.clone()),
            timestamp_ms,
        }),
        5 => Ok(PostKind::Leave {
            channel: ChannelName::new(ext.channel.clone()),
            timestamp_ms,
        }),
        6 if ext.prune_flag.is_set() => Ok(PostKind::RetentionCheckpoint {
            authority_revision: ext
                .authority_revision
                .clone()
                .ok_or(MurmError::MalformedPost)?,
            timestamp_ms,
        }),
        _ => Err(MurmError::MalformedPost),
    }
}

/// Build the unsigned p2panda header (+ body bytes) for a post's logical fields.
///
/// Deterministic in `(author, cabal_id, seq_num, backlink, links, kind)`. Shared
/// by signing, verification, and encoding so all three reconstruct
/// byte-identical headers. `seq_num`/`backlink` are the per-author log position
/// (the p2panda log rule: `backlink.is_some()` iff `seq_num > 0`).
pub(crate) fn build_header(
    signing_key: &SigningKey,
    cabal_id: [u8; 32],
    seq_num: u32,
    backlink: Option<PostId>,
    links: &[PostId],
    kind: &PostKind,
) -> (Header<CabalExt>, Vec<u8>) {
    let (extensions, body_bytes) = decompose(cabal_id, links, kind);
    // p2panda 0.7.1 has no unsigned `Header`: the builder encodes and signs in
    // one step, and `body` sets payload_size/payload_hash from the bytes —
    // including the empty-body case, where both stay zero/None as the operation
    // format requires (channel-less and empty-text posts carry no body).
    let header = Header::builder()
        .body(&body_bytes)
        .seq_num(seq_num)
        .backlink(backlink.map(|id| Hash::from(*id.as_bytes())))
        .build(signing_key, extensions);
    (header, body_bytes)
}

/// Recover the *signed* header (+ body bytes) for an existing [`Post`].
///
/// Decodes the bytes the post carries rather than rebuilding them: since
/// p2panda 0.7.1 a signed header cannot be reconstructed from its parts, and
/// `Header::decode` re-verifies the signature on the way through.
fn signed_header(post: &Post) -> Result<(Header<CabalExt>, Vec<u8>), MurmError> {
    let header = Header::<CabalExt>::decode(&post.header).map_err(|_| MurmError::MalformedPost)?;
    let (_ext, body) = decompose(post.cabal_id, &post.links, &post.kind);
    Ok((header, body))
}

/// The operation id of a post: its signed-header hash (`Header::hash()`), the
/// p2panda operation identity used for content addressing, backlinks, and sync.
/// Equal to what `Header::hash()` returned at authoring time (the header is
/// rebuilt deterministically from the post's fields).
pub fn operation_id(post: &Post) -> PostId {
    match signed_header(post) {
        Ok((header, _body)) => PostId::new(*header.hash().as_bytes()),
        // Unreachable for a well-formed post: `author` is always a valid key.
        Err(_) => PostId::new([0u8; 32]),
    }
}

/// Reconstruct the full p2panda [`Operation`] for a stored [`Post`].
///
/// The inverse of [`decode_post`] at the operation layer: it rebuilds the signed
/// `Header<CabalExt>` and body so a stored post can be served to p2panda-net's
/// LogSync, which reconciles `Operation`s, not `Post`s. `body` is `Some` iff the
/// post carries a payload (text/topic), matching the header's `payload_hash`.
pub fn post_to_operation(post: &Post) -> Result<Operation<CabalExt>, MurmError> {
    let (header, body_bytes) = signed_header(post)?;
    let hash = header.hash();
    let body = if body_bytes.is_empty() {
        None
    } else {
        Some(Body::from_bytes(&body_bytes))
    };
    Ok(Operation { hash, header, body })
}

/// Reconstruct a [`Post`] from a received p2panda [`Operation`].
///
/// The inverse of [`post_to_operation`]: rebuilds the logical post from the
/// operation's signed header (`author`, `signature`, `seq_num`, `backlink`,
/// `links`) and its `CabalExt` extension + body (the `PostKind`). Used by the
/// sync drain to land operations LogSync delivers. Returns
/// [`MurmError::MalformedPost`] for a missing signature, an unknown
/// `post_type`, invalid key bytes, or non-UTF-8 text/topic content.
///
/// This does **not** verify the signature. Runtime admission in `murm` does so
/// alongside the conversation-id and log-position checks.
pub fn operation_to_post(op: &Operation<CabalExt>) -> Result<Post, MurmError> {
    let header = &op.header;
    let author = from_p2_vk(&header.verifying_key)?;
    let signature = from_p2_sig(&header.signature);
    let links = header
        .extensions
        .parents
        .iter()
        .map(|p| PostId::new(*p))
        .collect();
    let body_bytes = op.body.as_ref().map(|b| b.to_bytes()).unwrap_or_default();
    let kind = recompose(&header.extensions, &body_bytes)?;
    let backlink = header.backlink.map(|h| PostId::new(*h.as_bytes()));
    Ok(Post {
        author,
        cabal_id: header.extensions.cabal_id,
        seq_num: header.seq_num,
        backlink,
        links,
        kind,
        signature,
        header: header.encode(),
    })
}

/// Encode a [`Post`] to its canonical wire bytes (a CBOR p2panda operation).
pub fn encode_post(post: &Post) -> Vec<u8> {
    // A `Post` whose `author` is not a valid ed25519 point can never have been
    // produced by signing or decoding; fall back to empty rather than panic.
    let Ok((_header, body)) = signed_header(post) else {
        return Vec::new();
    };
    encode_cbor(&WirePost {
        header: post.header.clone(),
        body,
    })
    .expect("CBOR encoding of a post never fails")
}

/// Decode a [`Post`] from canonical wire bytes.
///
/// Returns [`MurmError::MalformedPost`] for any structural problem
/// (CBOR error, missing signature, unknown `post_type`, invalid public-key
/// bytes, non-UTF-8 in a text/topic field).
pub fn decode_post(bytes: &[u8]) -> Result<Post, MurmError> {
    let wire: WirePost = decode_cbor_strict(bytes).map_err(|_| MurmError::MalformedPost)?;
    // `Header::decode` verifies the signature before it will return a header,
    // so a malformed or forged post is rejected right here.
    let header = Header::<CabalExt>::decode(&wire.header).map_err(|_| MurmError::MalformedPost)?;
    let author = from_p2_vk(&header.verifying_key)?;
    let signature = from_p2_sig(&header.signature);
    let links = header
        .extensions
        .parents
        .iter()
        .map(|p| PostId::new(*p))
        .collect();
    let kind = recompose(&header.extensions, &wire.body)?;
    let backlink = header.backlink.map(|h| PostId::new(*h.as_bytes()));
    Ok(Post {
        author,
        cabal_id: header.extensions.cabal_id,
        seq_num: header.seq_num,
        backlink,
        links,
        kind,
        signature,
        header: wire.header,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
