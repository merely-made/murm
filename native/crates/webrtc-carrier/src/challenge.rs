// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The link challenge: what a WebRTC session is bound to, and the 16 bytes
//! Notochord gets out of it.
//!
//! A WebRTC carrier reports no authenticated initiator. Notochord establishes
//! the subject itself, and the only thing that stops a captured hello from
//! being replayed onto a second connection is a link identifier that differs
//! between the two. Reticulum already supplies one from its link handshake;
//! WebRTC has no equivalent, so the carrier computes one.
//!
//! [`LinkChallenge`] is the transcript that gets computed over. It binds
//! exactly six things, per the browser WebRTC carrier plan §5:
//!
//! 1. the protocol,
//! 2. the data-channel label,
//! 3. the invite id,
//! 4. a fresh client nonce,
//! 5. a fresh server nonce,
//! 6. the role-tagged SHA-256 DTLS fingerprints of both ends.
//!
//! Both nonces are **inputs**. This crate has no randomness — not on native,
//! not in the browser — so freshness is the runtime adapter's obligation and
//! is visible in its signature rather than hidden in a core that quietly
//! reaches for the OS.
//!
//! ## Two names, frozen
//!
//! [`LINK_CHALLENGE_VERSION`] opens the transcript itself. The transcript
//! bytes get used twice — the host signs them (C2) and both ends hash them
//! (here) — so they carry their own version tag, and a signature over them
//! cannot be reinterpreted as a signature over a later transcript shape.
//!
//! [`SHARED_LINK_DOMAIN`] separates the derivation. The link id is not a bare
//! blake3 of the transcript: it is a hash under one domain string that means
//! "16-byte Notochord link identifier" and nothing else, so no other use of
//! the same transcript bytes can ever collide with it.
//!
//! Both strings are wire behaviour. Changing either changes every link both
//! ends derive, and browser and native must move together.

use blake3::Hasher;

use crate::codec::push_field;
use crate::error::ChallengeError;
use crate::fingerprint::{DtlsFingerprint, FingerprintRole};
use crate::invite::InviteId;

/// Width of each handshake nonce.
pub const NONCE_BYTES: usize = 32;

/// Width of Notochord's link identifier.
pub const SHARED_LINK_BYTES: usize = 16;

/// Ceiling on a variable-length transcript field.
///
/// The protocol and channel label are short names, not payloads. Bounding
/// them keeps a transcript small enough to sign, hash, and log without a
/// second thought, and keeps the four-byte length prefixes honest.
pub const MAX_TRANSCRIPT_FIELD_BYTES: usize = 255;

/// The version tag that opens every encoded transcript.
pub const LINK_CHALLENGE_VERSION: &str = "mere.webrtc-carrier/link-challenge/v1";

/// The domain separation string for shared-link derivation.
pub const SHARED_LINK_DOMAIN: &str = "mere.webrtc-carrier/shared-link/v1";

/// The pre-admission binding between one WebRTC connection and one invitation.
///
/// Construct it on both ends from the same six facts and both ends derive the
/// same [`shared_link`](Self::shared_link). Any disagreement — a substituted
/// fingerprint, a stale nonce, the wrong invitation — produces a different
/// link, and the Notochord handshake above it fails rather than admitting a
/// session bound to something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkChallenge {
    protocol: Vec<u8>,
    channel_label: Vec<u8>,
    invite: InviteId,
    client_nonce: [u8; NONCE_BYTES],
    server_nonce: [u8; NONCE_BYTES],
    client_fingerprint: DtlsFingerprint,
    server_fingerprint: DtlsFingerprint,
}

impl LinkChallenge {
    /// Assembles a transcript, checking the role slots and the field bounds.
    ///
    /// The fingerprint arguments are named for the slots they fill, and a
    /// fingerprint carrying the other role is refused rather than accepted
    /// and tagged. Passing the two the wrong way round is the exact mistake
    /// the role tag exists to catch, so it is caught here, at the
    /// constructor, and not left to show up as an unexplained link mismatch
    /// at the far end.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        protocol: impl Into<Vec<u8>>,
        channel_label: impl Into<Vec<u8>>,
        invite: InviteId,
        client_nonce: [u8; NONCE_BYTES],
        server_nonce: [u8; NONCE_BYTES],
        client_fingerprint: DtlsFingerprint,
        server_fingerprint: DtlsFingerprint,
    ) -> Result<Self, ChallengeError> {
        let protocol = protocol.into();
        let channel_label = channel_label.into();
        check_field("protocol", &protocol)?;
        check_field("channel_label", &channel_label)?;
        check_role(FingerprintRole::Client, client_fingerprint)?;
        check_role(FingerprintRole::Server, server_fingerprint)?;
        Ok(Self {
            protocol,
            channel_label,
            invite,
            client_nonce,
            server_nonce,
            client_fingerprint,
            server_fingerprint,
        })
    }

    /// The protocol this channel carries, e.g. `mere/graphshell/v1`.
    pub fn protocol(&self) -> &[u8] {
        &self.protocol
    }

    /// The negotiated data-channel label.
    pub fn channel_label(&self) -> &[u8] {
        &self.channel_label
    }

    /// The invitation this connection belongs to.
    pub const fn invite(&self) -> InviteId {
        self.invite
    }

    /// The browser's nonce.
    pub const fn client_nonce(&self) -> &[u8; NONCE_BYTES] {
        &self.client_nonce
    }

    /// The host's nonce.
    pub const fn server_nonce(&self) -> &[u8; NONCE_BYTES] {
        &self.server_nonce
    }

    /// The browser's DTLS fingerprint.
    pub const fn client_fingerprint(&self) -> &DtlsFingerprint {
        &self.client_fingerprint
    }

    /// The host's DTLS fingerprint.
    pub const fn server_fingerprint(&self) -> &DtlsFingerprint {
        &self.server_fingerprint
    }

    /// The canonical encoding: the bytes the host signs and both ends hash.
    ///
    /// Every field is length-prefixed, fixed-width fields included, so the
    /// encoding is injective — no regrouping of the same bytes across two
    /// fields can produce the same transcript, which is what stops a
    /// `protocol` that swallowed the channel label from deriving the link a
    /// legitimate pair would.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, LINK_CHALLENGE_VERSION.as_bytes());
        push_field(&mut out, &self.protocol);
        push_field(&mut out, &self.channel_label);
        push_field(&mut out, self.invite.as_bytes());
        push_field(&mut out, &self.client_nonce);
        push_field(&mut out, &self.server_nonce);
        push_field(&mut out, &self.client_fingerprint.canonical_bytes());
        push_field(&mut out, &self.server_fingerprint.canonical_bytes());
        out
    }

    /// Derives Notochord's 16-byte link identifier.
    ///
    /// `blake3( domain || len(transcript) || transcript )`,
    /// truncated to the first 16 bytes. The domain string is length-prefixed
    /// for the same reason the transcript's own fields are: so the two inputs
    /// cannot be re-split into a different pair that hashes identically.
    ///
    /// This is the value [`IngressContext::webrtc`][ingress] carries into
    /// Notochord's `SessionFacts`, and the value Notochord's
    /// `initiator_link_binding` proves possession of — the same grammar
    /// Reticulum already uses for a carrier that cannot authenticate peers.
    ///
    /// [ingress]: https://docs.rs/transport
    pub fn shared_link(&self) -> [u8; SHARED_LINK_BYTES] {
        let transcript = self.encode();
        // The same shape `graphshell::browser_carrier` uses for its own link:
        // blake3, the domain raw at the front, then a u64-le length prefix.
        // Two carriers filling the same 16-byte Notochord slot by two
        // different recipes is a difference with no reason behind it.
        let mut hasher = Hasher::new();
        hasher.update(SHARED_LINK_DOMAIN.as_bytes());
        hasher.update(&(transcript.len() as u64).to_le_bytes());
        hasher.update(&transcript);
        let mut link = [0u8; SHARED_LINK_BYTES];
        link.copy_from_slice(&hasher.finalize().as_bytes()[..SHARED_LINK_BYTES]);
        link
    }
}

fn check_field(name: &'static str, value: &[u8]) -> Result<(), ChallengeError> {
    if value.is_empty() {
        return Err(ChallengeError::FieldEmpty { field: name });
    }
    if value.len() > MAX_TRANSCRIPT_FIELD_BYTES {
        return Err(ChallengeError::FieldTooLong {
            field: name,
            len: value.len(),
            max: MAX_TRANSCRIPT_FIELD_BYTES,
        });
    }
    Ok(())
}

fn check_role(
    expected: FingerprintRole,
    fingerprint: DtlsFingerprint,
) -> Result<(), ChallengeError> {
    if fingerprint.role() != expected {
        return Err(ChallengeError::RoleMismatch {
            expected: expected.name(),
            got: fingerprint.role().name(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::DTLS_FINGERPRINT_BYTES;

    fn challenge() -> LinkChallenge {
        LinkChallenge::new(
            b"mere/graphshell/v1".to_vec(),
            b"mere-graphshell".to_vec(),
            InviteId::from_bytes([9; 16]),
            [0x11; NONCE_BYTES],
            [0x22; NONCE_BYTES],
            DtlsFingerprint::new(FingerprintRole::Client, [0xaa; DTLS_FINGERPRINT_BYTES]),
            DtlsFingerprint::new(FingerprintRole::Server, [0xbb; DTLS_FINGERPRINT_BYTES]),
        )
        .expect("valid")
    }

    #[test]
    fn the_encoding_is_a_run_of_length_prefixed_fields() {
        let encoded = challenge().encode();
        let expected = 8 * 8
            + LINK_CHALLENGE_VERSION.len()
            + b"mere/graphshell/v1".len()
            + b"mere-graphshell".len()
            + 16
            + NONCE_BYTES * 2
            + 33 * 2;
        assert_eq!(encoded.len(), expected);
        assert_eq!(
            &encoded[..8],
            &(LINK_CHALLENGE_VERSION.len() as u64).to_le_bytes()
        );
    }

    #[test]
    fn derivation_is_deterministic() {
        assert_eq!(challenge().shared_link(), challenge().shared_link());
    }

    #[test]
    fn a_fingerprint_in_the_wrong_slot_is_refused() {
        let swapped = LinkChallenge::new(
            b"mere/graphshell/v1".to_vec(),
            b"mere-graphshell".to_vec(),
            InviteId::from_bytes([9; 16]),
            [0x11; NONCE_BYTES],
            [0x22; NONCE_BYTES],
            DtlsFingerprint::new(FingerprintRole::Server, [0xaa; DTLS_FINGERPRINT_BYTES]),
            DtlsFingerprint::new(FingerprintRole::Server, [0xbb; DTLS_FINGERPRINT_BYTES]),
        );
        assert_eq!(
            swapped,
            Err(ChallengeError::RoleMismatch {
                expected: "client",
                got: "server"
            })
        );
    }

    #[test]
    fn empty_and_oversize_fields_are_refused() {
        let empty = LinkChallenge::new(
            Vec::new(),
            b"mere-graphshell".to_vec(),
            InviteId::from_bytes([9; 16]),
            [0x11; NONCE_BYTES],
            [0x22; NONCE_BYTES],
            DtlsFingerprint::new(FingerprintRole::Client, [0xaa; DTLS_FINGERPRINT_BYTES]),
            DtlsFingerprint::new(FingerprintRole::Server, [0xbb; DTLS_FINGERPRINT_BYTES]),
        );
        assert_eq!(empty, Err(ChallengeError::FieldEmpty { field: "protocol" }));

        let long = LinkChallenge::new(
            b"mere/graphshell/v1".to_vec(),
            vec![b'x'; MAX_TRANSCRIPT_FIELD_BYTES + 1],
            InviteId::from_bytes([9; 16]),
            [0x11; NONCE_BYTES],
            [0x22; NONCE_BYTES],
            DtlsFingerprint::new(FingerprintRole::Client, [0xaa; DTLS_FINGERPRINT_BYTES]),
            DtlsFingerprint::new(FingerprintRole::Server, [0xbb; DTLS_FINGERPRINT_BYTES]),
        );
        assert_eq!(
            long,
            Err(ChallengeError::FieldTooLong {
                field: "channel_label",
                len: MAX_TRANSCRIPT_FIELD_BYTES + 1,
                max: MAX_TRANSCRIPT_FIELD_BYTES,
            })
        );
    }
}
