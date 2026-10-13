// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cabal post signing and verification — p2panda-core operation semantics.
//!
//! A post's signature is the Ed25519 signature its author's per-cabal key
//! places over the canonical operation header (see [`crate::post_wire`]).
//! Signing goes through p2panda-core's header builder and verification through
//! `Header::decode`, which re-derives the signing bytes and checks the
//! signature before it will return a header — so murm posts are authenticated
//! exactly like any other operation on the substrate.
//!
//! [`sign_post`] constructs a signed [`Post`] in a cabal from a keypair, its
//! per-author log position (`seq_num`/`backlink`), links, and payload kind.
//! [`verify_post`] checks an existing `Post`'s signature against its `author`.

use identity::Ed25519Keypair;
use p2panda_core::{Hash, Header, Signature, SigningKey, VerifyingKey};

use crate::post_wire::CabalExt;

use crate::post_wire::{build_header, decompose, from_p2_sig};
use crate::{Post, PostId, PostKind};

/// Sign a new [`Post`] in the given cabal with the given per-cabal keypair.
///
/// `cabal_id` (= `BLAKE3(cabal_key)`) is written into the signed header, so the
/// resulting post is self-describing. `seq_num`/`backlink` place the operation
/// in this author's per-cabal log (`0`/`None` for the author's first operation;
/// thereafter `seq_num` increments and `backlink` is the previous operation's
/// id). The keypair's secret seed builds the p2panda [`SigningKey`] (both are
/// ed25519-dalek underneath); `Post.signature` is the operation header's Ed25519
/// signature, and `Post.author` is the keypair's public key.
pub fn sign_post(
    keypair: &Ed25519Keypair,
    cabal_id: [u8; 32],
    seq_num: u32,
    backlink: Option<PostId>,
    links: Vec<PostId>,
    kind: PostKind,
) -> Post {
    let signing_key = SigningKey::from_bytes(&keypair.to_seed());
    let (header, _body) = build_header(&signing_key, cabal_id, seq_num, backlink, &links, &kind);
    let signature: Signature = header.signature;
    Post {
        author: keypair.public_key(),
        cabal_id,
        seq_num,
        backlink,
        links,
        kind,
        signature: from_p2_sig(&signature),
        header: header.encode(),
    }
}

/// Verify a post's signature.
///
/// Returns `true` iff `post.signature` is a valid Ed25519 signature by
/// `post.author` over the canonical operation header reconstructed from the
/// post's `cabal_id`, `seq_num`, `backlink`, `links`, and `kind`. A `false`
/// return covers signature invalidity, a mismatched author, a different cabal, a
/// tampered log position, or a tampered body — anything that changes the header
/// preimage.
pub fn verify_post(post: &Post) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(&post.author.to_bytes()) else {
        return false;
    };
    // Decoding IS the verification: `Header::decode` re-derives the signing
    // bytes and checks the signature before returning. What it cannot know is
    // whether the header agrees with the rest of the `Post`, so the fields the
    // header signs are compared against it here.
    let Ok(header) = Header::<CabalExt>::decode(&post.header) else {
        return false;
    };
    // Everything the header signs, recomputed from the post and compared. The
    // extension covers cabal_id, links, channel, post_type and timestamp; the
    // payload commitment covers the text or topic body.
    let (extensions, body) = decompose(post.cabal_id, &post.links, &post.kind);
    header.verifying_key == verifying_key
        && header.signature.to_bytes() == post.signature.to_bytes()
        && header.extensions == extensions
        && header.seq_num == post.seq_num
        && header.backlink.map(|h| PostId::new(*h.as_bytes())) == post.backlink
        && header.payload_size == body.len() as u32
        && header.payload_hash == (!body.is_empty()).then(|| Hash::digest(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_wire::{decode_post, encode_post, operation_id};
    use crate::{ChannelName, InfoEntry};
    use identity::{IdentityProvider, InMemoryProvider};

    /// A fixed cabal id for the signing tests.
    const CABAL: [u8; 32] = [0xca; 32];

    fn keypair() -> Ed25519Keypair {
        InMemoryProvider::from_seed([99; 32])
            .derive_keypair(b"signing-test")
            .unwrap()
    }

    /// Sign a root operation (first in the log: seq 0, no backlink).
    fn root(kp: &Ed25519Keypair, links: Vec<PostId>, kind: PostKind) -> Post {
        sign_post(kp, CABAL, 0, None, links, kind)
    }

    fn text(t: &str) -> PostKind {
        PostKind::Text {
            channel: ChannelName::new("c"),
            text: t.to_string(),
            timestamp_ms: 1,
        }
    }

    #[test]
    fn sign_then_verify_text() {
        let kp = keypair();
        let post = root(
            &kp,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "hello signed cabal".to_string(),
                timestamp_ms: 1_700_000_000_000,
            },
        );
        assert!(verify_post(&post));
        assert_eq!(post.author.to_bytes(), kp.public_key().to_bytes());
        assert_eq!(post.cabal_id, CABAL);
        assert_eq!(post.seq_num, 0);
        assert_eq!(post.backlink, None);
    }

    #[test]
    fn sign_then_verify_all_post_types() {
        let kp = keypair();
        let cases: Vec<PostKind> = vec![
            text("x"),
            PostKind::Delete {
                posts: vec![PostId::new([1; 32]), PostId::new([2; 32])],
                timestamp_ms: 2,
            },
            PostKind::Info {
                entries: vec![InfoEntry::name("alice")],
                timestamp_ms: 3,
            },
            PostKind::Topic {
                channel: ChannelName::new("c"),
                topic: "t".to_string(),
                timestamp_ms: 4,
            },
            PostKind::Join {
                channel: ChannelName::new("c"),
                timestamp_ms: 5,
            },
            PostKind::Leave {
                channel: ChannelName::new("c"),
                timestamp_ms: 6,
            },
        ];
        for kind in cases {
            let kind_name = kind.kind_name();
            let post = root(&kp, vec![], kind);
            assert!(verify_post(&post), "verify failed for {kind_name}");
        }
    }

    #[test]
    fn per_author_log_chain_verifies_and_round_trips() {
        // op0 is the log root; op1 chains onto it via seq_num + backlink.
        let kp = keypair();
        let op0 = root(&kp, vec![], text("a"));
        assert!(verify_post(&op0));
        let id0 = operation_id(&op0);

        let op1 = sign_post(&kp, CABAL, 1, Some(id0), vec![], text("b"));
        assert!(verify_post(&op1));
        assert_eq!(op1.seq_num, 1);
        assert_eq!(op1.backlink, Some(id0));

        // The log position survives the wire round trip.
        let decoded = decode_post(&encode_post(&op1)).expect("decode");
        assert_eq!(decoded.seq_num, 1);
        assert_eq!(decoded.backlink, Some(id0));
        assert!(verify_post(&decoded));
    }

    #[test]
    fn verify_rejects_tampered_log_position() {
        // seq_num and backlink are signed; tampering either breaks verification.
        let kp = keypair();
        let id0 = operation_id(&root(&kp, vec![], text("a")));
        let op1 = sign_post(&kp, CABAL, 1, Some(id0), vec![], text("b"));

        let mut bad_seq = op1.clone();
        bad_seq.seq_num = 2;
        assert!(!verify_post(&bad_seq));

        let mut bad_backlink = op1.clone();
        bad_backlink.backlink = Some(PostId::new([0xee; 32]));
        assert!(!verify_post(&bad_backlink));
    }

    #[test]
    fn verify_rejects_tampered_text() {
        let kp = keypair();
        let mut post = root(
            &kp,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "original".to_string(),
                timestamp_ms: 1,
            },
        );
        if let PostKind::Text { ref mut text, .. } = post.kind {
            *text = "tampered".to_string();
        }
        assert!(!verify_post(&post));
    }

    #[test]
    fn verify_rejects_tampered_cabal_id() {
        // The cabal id is signed, so swapping it invalidates the signature —
        // a post can't be replayed into a different cabal.
        let kp = keypair();
        let mut post = root(&kp, vec![], text("x"));
        post.cabal_id = [0xff; 32];
        assert!(!verify_post(&post));
    }

    #[test]
    fn verify_rejects_swapped_author() {
        let kp_alice = keypair();
        let kp_bob = InMemoryProvider::from_seed([0; 32])
            .derive_keypair(b"signing-test")
            .unwrap();
        let mut post = root(
            &kp_alice,
            vec![],
            PostKind::Text {
                channel: ChannelName::new("c"),
                text: "from alice".to_string(),
                timestamp_ms: 1,
            },
        );
        // Replace the author claim with bob's key — alice's signature no longer
        // matches the (now bob-authored) header preimage.
        post.author = kp_bob.public_key();
        assert!(!verify_post(&post));
    }

    #[test]
    fn signed_post_round_trips_through_wire_encoding() {
        let kp = keypair();
        let post = root(
            &kp,
            vec![PostId::new([7; 32])],
            PostKind::Text {
                channel: ChannelName::new("session"),
                text: "round trip me".to_string(),
                timestamp_ms: 1_700_000_000_000,
            },
        );
        let encoded = encode_post(&post);
        let decoded = decode_post(&encoded).expect("decode failed");
        assert!(
            verify_post(&decoded),
            "decoded post failed signature verification"
        );
        assert_eq!(encoded, encode_post(&decoded));
    }

    #[test]
    fn deterministic_signing_for_same_input() {
        // Ed25519 signing is deterministic (RFC 8032): same input → same
        // signature. We rely on this for content-addressing stability.
        let kp = keypair();
        let mk = || root(&kp, vec![], text("x"));
        assert_eq!(mk().signature.to_bytes(), mk().signature.to_bytes());
    }

    #[test]
    fn different_links_yield_different_signatures() {
        let kp = keypair();
        let no_links = root(&kp, vec![], text("x"));
        let with_links = root(&kp, vec![PostId::new([1; 32])], text("x"));
        // The signed header carries `parents`, so the signatures differ.
        assert_ne!(
            no_links.signature.to_bytes(),
            with_links.signature.to_bytes()
        );
        assert!(verify_post(&no_links));
        assert!(verify_post(&with_links));
    }

    #[test]
    fn different_cabals_yield_different_signatures() {
        // Same author, same content, different cabal → different signature,
        // because the cabal id is part of the signed header.
        let kp = keypair();
        let a = sign_post(&kp, [1; 32], 0, None, vec![], text("x"));
        let b = sign_post(&kp, [2; 32], 0, None, vec![], text("x"));
        assert_ne!(a.signature.to_bytes(), b.signature.to_bytes());
    }
}
