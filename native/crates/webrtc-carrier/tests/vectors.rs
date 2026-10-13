// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The cross-target vectors.
//!
//! Browser and native share wire types and test vectors, not one runtime
//! (browser WebRTC carrier plan §2). This file is that share. The canonical
//! vector below is a fixed input and a fixed *literal* expected output: it is
//! recomputed by nothing, so running it under `wasm32-unknown-unknown` and
//! under the host toolchain is a real comparison rather than two independent
//! copies of the same arithmetic agreeing with themselves.
//!
//! The remaining tests are the differential half — the properties the
//! transcript exists to have. Each one asserts that some substitution an
//! attacker or a bug would attempt lands on a *different* link.

use webrtc_carrier::{
    ChallengeError, DTLS_FINGERPRINT_BYTES, DtlsFingerprint, FRAME_HEADER_BYTES, FingerprintError,
    FingerprintRole, FrameError, FrameHeader, InviteId, LinkChallenge, MAX_FRAME_PAYLOAD_BYTES,
    NONCE_BYTES, decode_frame, encode_frame,
};

/// The canonical vector's inputs, in one place.
///
/// A second implementation (the browser's, or a future `str0m` answerer)
/// reproduces the vector from exactly these:
///
/// ```text
/// protocol       "mere/graphshell/v1"                 (ASCII)
/// channel label  "mere-graphshell"                    (ASCII)
/// invite id      000102030405060708090a0b0c0d0e0f     (16 bytes)
/// client nonce   11 * 32                              (32 bytes)
/// server nonce   22 * 32                              (32 bytes)
/// client dtls    AA * 32, role tag 0x01               (32 bytes)
/// server dtls    BB * 32, role tag 0x02               (32 bytes)
/// ```
const PROTOCOL: &[u8] = b"mere/graphshell/v1";
const CHANNEL_LABEL: &[u8] = b"mere-graphshell";
const INVITE: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];
const CLIENT_NONCE: [u8; NONCE_BYTES] = [0x11; NONCE_BYTES];
const SERVER_NONCE: [u8; NONCE_BYTES] = [0x22; NONCE_BYTES];
const CLIENT_DIGEST: [u8; DTLS_FINGERPRINT_BYTES] = [0xaa; DTLS_FINGERPRINT_BYTES];
const SERVER_DIGEST: [u8; DTLS_FINGERPRINT_BYTES] = [0xbb; DTLS_FINGERPRINT_BYTES];

/// The frozen answer.
///
/// Changing `LINK_CHALLENGE_VERSION`, `SHARED_LINK_DOMAIN`, the field order,
/// the length-prefix width, or the truncation moves this value. That is the
/// point: it is a tripwire on wire behaviour, and it must be edited
/// deliberately and in step with every other implementation, never "updated
/// to match" a build that started failing.
const EXPECTED_SHARED_LINK: [u8; 16] = [
    0x46, 0x92, 0xc8, 0x8e, 0x70, 0x47, 0x0f, 0x7e, 0x4b, 0x7b, 0xa4, 0x6b, 0x7f, 0xce, 0x78, 0xb2,
];

/// The canonical transcript's encoded length, in bytes.
///
/// Eight length prefixes at eight bytes each, then the eight fields.
const EXPECTED_TRANSCRIPT_LEN: usize = 280;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical() -> LinkChallenge {
    challenge_with(
        PROTOCOL,
        CHANNEL_LABEL,
        INVITE,
        CLIENT_NONCE,
        SERVER_NONCE,
        CLIENT_DIGEST,
        SERVER_DIGEST,
    )
}

fn challenge_with(
    protocol: &[u8],
    channel_label: &[u8],
    invite: [u8; 16],
    client_nonce: [u8; NONCE_BYTES],
    server_nonce: [u8; NONCE_BYTES],
    client_digest: [u8; DTLS_FINGERPRINT_BYTES],
    server_digest: [u8; DTLS_FINGERPRINT_BYTES],
) -> LinkChallenge {
    LinkChallenge::new(
        protocol.to_vec(),
        channel_label.to_vec(),
        InviteId::from_bytes(invite),
        client_nonce,
        server_nonce,
        DtlsFingerprint::new(FingerprintRole::Client, client_digest),
        DtlsFingerprint::new(FingerprintRole::Server, server_digest),
    )
    .expect("the vector's inputs are valid")
}

#[test]
fn canonical_shared_link_vector() {
    let challenge = canonical();
    let transcript = challenge.encode();
    assert_eq!(
        transcript.len(),
        EXPECTED_TRANSCRIPT_LEN,
        "transcript encoding changed shape: {}",
        hex(&transcript)
    );

    let link = challenge.shared_link();
    assert_eq!(
        link,
        EXPECTED_SHARED_LINK,
        "shared_link drifted; computed {}",
        hex(&link)
    );
    assert_eq!(hex(&link), "4692c88e70470f7e4b7ba46b7fce78b2");
}

#[test]
fn both_ends_derive_the_same_link_from_the_same_facts() {
    // The browser builds one and the host builds the other; nothing but the
    // six bound facts crosses between them.
    let browser_side = canonical();
    let host_side = canonical();
    assert_eq!(browser_side.shared_link(), host_side.shared_link());
    assert_eq!(browser_side.encode(), host_side.encode());
}

#[test]
fn swapping_the_client_and_server_fingerprints_changes_the_link() {
    let swapped = challenge_with(
        PROTOCOL,
        CHANNEL_LABEL,
        INVITE,
        CLIENT_NONCE,
        SERVER_NONCE,
        SERVER_DIGEST,
        CLIENT_DIGEST,
    );
    assert_ne!(canonical().shared_link(), swapped.shared_link());
}

#[test]
fn the_role_tag_is_what_makes_the_swap_visible() {
    // Same two digests, same slots, but with the role tags stripped the two
    // transcripts would differ only by digest order. The tag means the
    // canonical bytes differ as well, so a peer cannot present the host's
    // fingerprint as its own and land on the same link.
    let client = DtlsFingerprint::new(FingerprintRole::Client, CLIENT_DIGEST);
    let mistagged = DtlsFingerprint::new(FingerprintRole::Server, CLIENT_DIGEST);
    assert_ne!(client.canonical_bytes(), mistagged.canonical_bytes());

    let refused = LinkChallenge::new(
        PROTOCOL.to_vec(),
        CHANNEL_LABEL.to_vec(),
        InviteId::from_bytes(INVITE),
        CLIENT_NONCE,
        SERVER_NONCE,
        mistagged,
        DtlsFingerprint::new(FingerprintRole::Server, SERVER_DIGEST),
    );
    assert_eq!(
        refused,
        Err(ChallengeError::RoleMismatch {
            expected: "client",
            got: "server",
        })
    );
}

#[test]
fn changing_any_single_transcript_field_changes_the_link() {
    let baseline = canonical().shared_link();

    let mut invite = INVITE;
    invite[15] ^= 0x01;
    let mut client_nonce = CLIENT_NONCE;
    client_nonce[0] ^= 0x01;
    let mut server_nonce = SERVER_NONCE;
    server_nonce[NONCE_BYTES - 1] ^= 0x01;
    let mut client_digest = CLIENT_DIGEST;
    client_digest[0] ^= 0x01;
    let mut server_digest = SERVER_DIGEST;
    server_digest[DTLS_FINGERPRINT_BYTES - 1] ^= 0x01;

    let variants = [
        (
            "protocol",
            challenge_with(
                b"mere/graphshell/v2",
                CHANNEL_LABEL,
                INVITE,
                CLIENT_NONCE,
                SERVER_NONCE,
                CLIENT_DIGEST,
                SERVER_DIGEST,
            ),
        ),
        (
            "channel_label",
            challenge_with(
                PROTOCOL,
                b"mere-graphshel1",
                INVITE,
                CLIENT_NONCE,
                SERVER_NONCE,
                CLIENT_DIGEST,
                SERVER_DIGEST,
            ),
        ),
        (
            "invite",
            challenge_with(
                PROTOCOL,
                CHANNEL_LABEL,
                invite,
                CLIENT_NONCE,
                SERVER_NONCE,
                CLIENT_DIGEST,
                SERVER_DIGEST,
            ),
        ),
        (
            "client_nonce",
            challenge_with(
                PROTOCOL,
                CHANNEL_LABEL,
                INVITE,
                client_nonce,
                SERVER_NONCE,
                CLIENT_DIGEST,
                SERVER_DIGEST,
            ),
        ),
        (
            "server_nonce",
            challenge_with(
                PROTOCOL,
                CHANNEL_LABEL,
                INVITE,
                CLIENT_NONCE,
                server_nonce,
                CLIENT_DIGEST,
                SERVER_DIGEST,
            ),
        ),
        (
            "client_fingerprint",
            challenge_with(
                PROTOCOL,
                CHANNEL_LABEL,
                INVITE,
                CLIENT_NONCE,
                SERVER_NONCE,
                client_digest,
                SERVER_DIGEST,
            ),
        ),
        (
            "server_fingerprint",
            challenge_with(
                PROTOCOL,
                CHANNEL_LABEL,
                INVITE,
                CLIENT_NONCE,
                SERVER_NONCE,
                CLIENT_DIGEST,
                server_digest,
            ),
        ),
    ];

    let mut seen = vec![baseline];
    for (field, variant) in variants {
        let link = variant.shared_link();
        assert_ne!(link, baseline, "changing `{field}` left the link unchanged");
        assert!(
            !seen.contains(&link),
            "`{field}` collided with an earlier variant"
        );
        seen.push(link);
    }
    assert_eq!(seen.len(), 8);
}

#[test]
fn the_encoding_is_unambiguous_across_field_splittings() {
    // The concatenation attack: move the boundary between two adjacent
    // variable-length fields. Without a length prefix on each, both of these
    // would encode as the same run of bytes and derive the same link.
    let left = challenge_with(
        b"mere/graphshell",
        b"/v1mere-graphshell",
        INVITE,
        CLIENT_NONCE,
        SERVER_NONCE,
        CLIENT_DIGEST,
        SERVER_DIGEST,
    );
    let right = challenge_with(
        b"mere/graphshell/v1",
        b"mere-graphshell",
        INVITE,
        CLIENT_NONCE,
        SERVER_NONCE,
        CLIENT_DIGEST,
        SERVER_DIGEST,
    );

    let stripped = |challenge: &LinkChallenge| {
        let mut joined = challenge.protocol().to_vec();
        joined.extend_from_slice(challenge.channel_label());
        joined
    };
    assert_eq!(
        stripped(&left),
        stripped(&right),
        "the two splittings must share their unprefixed bytes, or this test proves nothing"
    );

    assert_ne!(left.encode(), right.encode());
    assert_ne!(left.shared_link(), right.shared_link());
}

#[test]
fn an_oversize_frame_is_rejected_before_allocation() {
    // Only the four-byte prefix is supplied — no payload exists to allocate
    // for, and none is read. The error is `Oversize`, not `Incomplete`, which
    // is the proof that the ceiling check runs before the length is ever
    // treated as a size to reserve.
    let declared = (MAX_FRAME_PAYLOAD_BYTES + 1) as u32;
    let header_only = declared.to_be_bytes();
    assert_eq!(header_only.len(), FRAME_HEADER_BYTES);

    assert_eq!(
        FrameHeader::decode(&header_only),
        Err(FrameError::Oversize {
            declared: u64::from(declared),
            max: MAX_FRAME_PAYLOAD_BYTES,
        })
    );
    assert_eq!(
        decode_frame(&header_only),
        Err(FrameError::Oversize {
            declared: u64::from(declared),
            max: MAX_FRAME_PAYLOAD_BYTES,
        })
    );

    // The pathological case a naive `Vec::with_capacity` would honour.
    assert_eq!(
        decode_frame(&u32::MAX.to_be_bytes()),
        Err(FrameError::Oversize {
            declared: u64::from(u32::MAX),
            max: MAX_FRAME_PAYLOAD_BYTES,
        })
    );

    // The send side refuses to produce one in the first place.
    assert!(encode_frame(&vec![0u8; MAX_FRAME_PAYLOAD_BYTES + 1]).is_err());

    // A frame exactly at the ceiling still round-trips.
    let at_ceiling = encode_frame(&vec![7u8; MAX_FRAME_PAYLOAD_BYTES]).expect("at the ceiling");
    let (payload, consumed) = decode_frame(&at_ceiling).expect("decodes");
    assert_eq!(payload.len(), MAX_FRAME_PAYLOAD_BYTES);
    assert_eq!(consumed, at_ceiling.len());
}

#[test]
fn a_malformed_fingerprint_is_rejected() {
    let good = DtlsFingerprint::new(FingerprintRole::Server, SERVER_DIGEST).to_sdp_hex();
    assert_eq!(
        DtlsFingerprint::parse_sdp_hex(FingerprintRole::Server, &good)
            .expect("the well-formed control parses")
            .digest(),
        &SERVER_DIGEST
    );

    // Truncated: 31 octets instead of 32. Rejected, not zero-padded.
    let truncated = good[..good.len() - 3].to_owned();
    assert_eq!(
        DtlsFingerprint::parse_sdp_hex(FingerprintRole::Server, &truncated),
        Err(FingerprintError::OctetCount { got: 31 })
    );

    // One octet is a single digit. Rejected, not left-padded.
    let short_octet = good.replacen("BB:", "B:", 1);
    assert_eq!(
        DtlsFingerprint::parse_sdp_hex(FingerprintRole::Server, &short_octet),
        Err(FingerprintError::Octet { index: 0 })
    );

    // Lowercase, which RFC 8122's grammar does not admit.
    assert_eq!(
        DtlsFingerprint::parse_sdp_hex(FingerprintRole::Server, &good.to_ascii_lowercase()),
        Err(FingerprintError::Octet { index: 0 })
    );

    // A stray space inside the hex.
    let spaced = good.replacen("BB:BB", "BB :B", 1);
    assert!(DtlsFingerprint::parse_sdp_hex(FingerprintRole::Server, &spaced).is_err());

    // A whole attribute naming a weaker hash.
    assert_eq!(
        DtlsFingerprint::parse_sdp_attribute(FingerprintRole::Server, &format!("sha-1 {good}")),
        Err(FingerprintError::Algorithm {
            got: "sha-1".to_owned()
        })
    );

    // An attribute with no hex at all.
    assert_eq!(
        DtlsFingerprint::parse_sdp_attribute(FingerprintRole::Server, "sha-256"),
        Err(FingerprintError::Attribute)
    );
}
