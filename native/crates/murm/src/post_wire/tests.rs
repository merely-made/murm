// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::post_sign::sign_post;
use identity::{Ed25519Keypair, IdentityProvider, InMemoryProvider};

fn keypair() -> Ed25519Keypair {
    InMemoryProvider::from_seed([5; 32])
        .derive_keypair(b"wire-test")
        .unwrap()
}

/// The cabal id used by the round-trip helper.
const TEST_CABAL: [u8; 32] = [0xca; 32];

/// Round-trip a signed post: encode → decode → re-encode yields identical
/// bytes, and the decoded post still verifies. (`Post` doesn't implement
/// `PartialEq` — the canonical bytes are the equality test.)
fn assert_round_trip(kind: PostKind, links: Vec<PostId>) {
    let post = sign_post(&keypair(), TEST_CABAL, 0, None, links, kind);
    let encoded = encode_post(&post);
    let decoded = decode_post(&encoded).expect("decode failed");
    assert_eq!(
        encoded,
        encode_post(&decoded),
        "post round-trip changed bytes"
    );
    assert!(
        crate::post_sign::verify_post(&decoded),
        "decoded post failed verification"
    );
}

#[test]
fn round_trip_text() {
    assert_round_trip(
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "hello, cabal".to_string(),
            timestamp_ms: 1_700_000_000_000,
        },
        vec![],
    );
}

#[test]
fn round_trip_text_with_links() {
    assert_round_trip(
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "follow-up".to_string(),
            timestamp_ms: 1,
        },
        vec![PostId::new([7; 32]), PostId::new([8; 32])],
    );
}

#[test]
fn round_trip_delete_multiple() {
    assert_round_trip(
        PostKind::Delete {
            posts: vec![
                PostId::new([3; 32]),
                PostId::new([4; 32]),
                PostId::new([5; 32]),
            ],
            timestamp_ms: 42,
        },
        vec![PostId::new([1; 32])],
    );
}

#[test]
fn round_trip_info_multiple_entries() {
    assert_round_trip(
        PostKind::Info {
            entries: vec![
                InfoEntry::name("alice"),
                InfoEntry::new("accept-role", vec![1u8]),
                InfoEntry::new("custom-key", vec![0xde, 0xad, 0xbe, 0xef]),
            ],
            timestamp_ms: 0,
        },
        vec![],
    );
}

#[test]
fn round_trip_info_empty() {
    assert_round_trip(
        PostKind::Info {
            entries: vec![],
            timestamp_ms: 0,
        },
        vec![],
    );
}

#[test]
fn round_trip_topic_empty_clears() {
    assert_round_trip(
        PostKind::Topic {
            channel: ChannelName::new("session"),
            topic: String::new(),
            timestamp_ms: 0,
        },
        vec![],
    );
}

#[test]
fn round_trip_join_and_leave() {
    assert_round_trip(
        PostKind::Join {
            channel: ChannelName::new("session"),
            timestamp_ms: 1,
        },
        vec![],
    );
    assert_round_trip(
        PostKind::Leave {
            channel: ChannelName::new("session"),
            timestamp_ms: 1,
        },
        vec![],
    );
}

#[test]
fn round_trip_text_with_unicode() {
    assert_round_trip(
        PostKind::Text {
            channel: ChannelName::new("国際"),
            text: "こんにちは 🌐".to_string(),
            timestamp_ms: 1,
        },
        vec![],
    );
}

#[test]
fn garbage_bytes_are_malformed() {
    assert!(matches!(
        decode_post(&[0xff, 0x00, 0x13, 0x37]),
        Err(MurmError::MalformedPost)
    ));
    assert!(matches!(decode_post(&[]), Err(MurmError::MalformedPost)));
}

#[test]
fn decoded_post_carries_the_signing_author_and_cabal() {
    let kp = keypair();
    let post = sign_post(
        &kp,
        [0x42; 32],
        0,
        None,
        vec![],
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "x".to_string(),
            timestamp_ms: 1,
        },
    );
    let decoded = decode_post(&encode_post(&post)).unwrap();
    assert_eq!(decoded.author.to_bytes(), kp.public_key().to_bytes());
    assert_eq!(
        decoded.cabal_id, [0x42; 32],
        "cabal id survives the round trip"
    );
}

#[test]
fn operations_pass_p2panda_log_validation() {
    // The strongest check: our per-author chain satisfies p2panda's *own*
    // validators, so p2panda-net LogSync will accept it as a real log.
    use p2panda_core::operation::{validate_backlink, validate_header};

    let kp = keypair();
    let op0 = sign_post(
        &kp,
        TEST_CABAL,
        0,
        None,
        vec![],
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "first".to_string(),
            timestamp_ms: 1,
        },
    );
    let op1 = sign_post(
        &kp,
        TEST_CABAL,
        1,
        Some(operation_id(&op0)),
        vec![],
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "second".to_string(),
            timestamp_ms: 2,
        },
    );

    let (h0, _) = signed_header(&op0).expect("rebuild op0 header");
    let (h1, _) = signed_header(&op1).expect("rebuild op1 header");

    assert!(validate_header(&h0).is_ok(), "root op is a valid header");
    assert!(validate_header(&h1).is_ok(), "second op is a valid header");
    assert!(
        validate_backlink(&h0, &h1).is_ok(),
        "op1 chains onto op0 per p2panda's backlink rule (seq+1, backlink == op0 header hash)"
    );
}

#[test]
fn operation_to_post_inverts_post_to_operation() {
    // A post → operation → post round-trip is byte-identical and still
    // verifies. This is the path the sync drain takes when LogSync delivers
    // an `OperationReceived` (operation back into a post to ingest).
    let post = sign_post(
        &keypair(),
        TEST_CABAL,
        1,
        Some(PostId::new([9; 32])),
        vec![PostId::new([7; 32])],
        PostKind::Text {
            channel: ChannelName::new("session"),
            text: "round trip via operation".to_string(),
            timestamp_ms: 5,
        },
    );
    let op = post_to_operation(&post).expect("post -> operation");
    // The operation id is the post's operation id (signed-header hash).
    assert_eq!(op.hash.as_bytes(), operation_id(&post).as_bytes());

    let back = operation_to_post(&op).expect("operation -> post");
    assert_eq!(
        encode_post(&post),
        encode_post(&back),
        "post survives the operation round-trip"
    );
    assert!(
        crate::post_sign::verify_post(&back),
        "reconstructed post verifies"
    );
}

#[test]
fn operation_to_post_handles_bodyless_posts() {
    // Channel-less, payload-less kinds (info/delete/join/leave) carry no
    // operation body; the round-trip must still reconstruct them.
    let post = sign_post(
        &keypair(),
        TEST_CABAL,
        0,
        None,
        vec![],
        PostKind::Info {
            entries: vec![InfoEntry::name("alice")],
            timestamp_ms: 0,
        },
    );
    let op = post_to_operation(&post).unwrap();
    assert!(
        op.body.is_none(),
        "a bodyless post yields an op with no body"
    );
    let back = operation_to_post(&op).unwrap();
    assert_eq!(encode_post(&post), encode_post(&back));
}

#[test]
fn retention_checkpoint_round_trip_and_ordinary_wire_compatibility() {
    assert_round_trip(
        PostKind::RetentionCheckpoint {
            authority_revision: proofs::Digest::blake3(b"policy"),
            timestamp_ms: 0,
        },
        Vec::new(),
    );

    // Model the previous extension grammar: absent new fields must decode as
    // defaults, and ordinary posts must retain exactly the old signed bytes.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct LegacyExt {
        cabal_id: [u8; 32],
        channel: String,
        post_type: u64,
        timestamp_ms: u64,
        parents: Vec<[u8; 32]>,
        info: Vec<(String, Vec<u8>)>,
        deletes: Vec<[u8; 32]>,
    }
    let kind = PostKind::Text {
        channel: ChannelName::new("session"),
        text: "unchanged".into(),
        timestamp_ms: 1,
    };
    let post = sign_post(&keypair(), TEST_CABAL, 0, None, Vec::new(), kind);
    let legacy = Header::builder().body(b"unchanged").build(
        &SigningKey::from_bytes(&keypair().to_seed()),
        LegacyExt {
            cabal_id: TEST_CABAL,
            channel: "session".into(),
            post_type: 0,
            timestamp_ms: 1,
            parents: Vec::new(),
            info: Vec::new(),
            deletes: Vec::new(),
        },
    );
    assert_eq!(post.header, legacy.encode());
    let decoded = Header::<CabalExt>::decode(&legacy.encode()).unwrap();
    assert!(decoded.extensions.prune_flag.is_not_set());
    assert!(decoded.extensions.authority_revision.is_none());
}
