// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Deterministic Noise static keys, and the identity proof that binds a Noise
//! session to a Mere `PeerID`.
//!
//! ## Why a proof is needed at all
//!
//! Noise `XX` authenticates the peer's **X25519 static key** and nothing else.
//! Mere's [`PeerID`] is an **Ed25519** public key. Those are different keys, so
//! a completed handshake does not by itself tell us who the peer is in Mere's
//! terms.
//!
//! So immediately after the handshake each side sends its Ed25519 public key
//! plus a signature over **this session's handshake hash**. Verifying it
//! promotes "some X25519 key completed a handshake" into "this `PeerID`,
//! provably". This is the same shape libp2p-noise uses for the same reason.
//!
//! Binding to the handshake hash rather than to the static key is the stronger
//! choice, and it was also the practical one: `snow` does not hand back the
//! local static public key before the handshake, and computing it would have
//! meant adding a second `x25519-dalek` major to a workspace deliberately
//! unifying on one. The hash is unique per session, so a proof captured from
//! one session cannot be replayed into another, which signing a long-lived
//! static key would have allowed.
//!
//! The signature covers a domain-separated message, so a signature harvested
//! from this protocol cannot be replayed as a signature for anything else Mere
//! signs.

use hkdf::Hkdf;
use sha2::Sha256;

use identity::{Ed25519Keypair, Ed25519PublicKey, Ed25519Signature};

use crate::PeerID;

/// HKDF-SHA256 context for the Noise static key. Changing this changes every
/// derived static key, so treat it as a wire-format version.
const STATIC_HKDF_INFO: &[u8] = b"mere-noise-static-v1";

/// Domain separator for the identity proof. Also wire-format: a peer built
/// against a different prefix will fail to verify.
const PROOF_PREFIX: &[u8] = b"mere-noise-identity-v1:";

/// The proof each side sends: 32 bytes of Ed25519 public key followed by a
/// 64-byte signature.
pub(super) const PROOF_LEN: usize = 32 + 64;

/// Derive the Noise static secret from the Ed25519 identity being proven.
///
/// Domain-separated through HKDF from that keypair's seed, exactly as the
/// Reticulum lane derives its X25519 half, so no two lanes can end up sharing
/// key material by accident. Pure, so the same identity always yields the same
/// Noise static key across restarts.
///
/// Note what this does *not* do: it derives from whatever identity the caller
/// chose, not from a fixed master. Pass a child or ephemeral keypair and the
/// static key is unlinkable to the carrier's, which is what the layered
/// identity in the module docs rests on.
pub(super) fn derive_static_secret(identity: &Ed25519Keypair) -> [u8; 32] {
    let seed = identity.to_seed();
    let hk = Hkdf::<Sha256>::new(None, &seed);
    let mut secret = [0u8; 32];
    hk.expand(STATIC_HKDF_INFO, &mut secret)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    secret
}

/// The message signed to bind an Ed25519 identity to one Noise session.
fn proof_message(handshake_hash: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(PROOF_PREFIX.len() + handshake_hash.len());
    message.extend_from_slice(PROOF_PREFIX);
    message.extend_from_slice(handshake_hash);
    message
}

/// Build the proof that `identity` completed this session.
pub(super) fn build_proof(identity: &Ed25519Keypair, handshake_hash: &[u8]) -> Vec<u8> {
    let signature = identity.sign(&proof_message(handshake_hash));
    let mut payload = Vec::with_capacity(PROOF_LEN);
    payload.extend_from_slice(&identity.public_key().to_bytes());
    payload.extend_from_slice(&signature.to_bytes());
    payload
}

/// Why a peer's identity proof was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofError {
    /// The payload was not `PROOF_LEN` bytes.
    Malformed,
    /// The 32 key bytes were not a valid Ed25519 public key.
    BadKey,
    /// The signature did not verify against this session's handshake hash.
    /// This is the case that matters: someone completed a handshake while
    /// claiming an identity they cannot sign for, or replayed a proof from a
    /// different session.
    Unproven,
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "identity proof is the wrong length"),
            Self::BadKey => write!(f, "identity proof carries an invalid Ed25519 key"),
            Self::Unproven => {
                write!(
                    f,
                    "identity proof does not sign this session's handshake hash"
                )
            },
        }
    }
}

/// Verify a peer's proof against this session's handshake hash, returning the
/// `PeerID` it proves.
///
/// Both sides compute the same handshake hash, and only a peer that actually
/// completed *this* handshake can sign it.
pub(super) fn verify_proof(payload: &[u8], handshake_hash: &[u8]) -> Result<PeerID, ProofError> {
    if payload.len() != PROOF_LEN {
        return Err(ProofError::Malformed);
    }
    let key_bytes: [u8; 32] = payload[..32].try_into().expect("checked length");
    let signature_bytes: [u8; 64] = payload[32..].try_into().expect("checked length");

    let public = Ed25519PublicKey::from_bytes(&key_bytes).map_err(|_| ProofError::BadKey)?;
    let signature = Ed25519Signature::from_bytes(&signature_bytes);

    if !public.verify(&proof_message(handshake_hash), &signature) {
        return Err(ProofError::Unproven);
    }
    Ok(PeerID::from_public_key(public))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair(seed: u8) -> Ed25519Keypair {
        Ed25519Keypair::from_seed([seed; 32])
    }

    #[test]
    fn the_static_key_is_deterministic_and_domain_separated() {
        let master = keypair(7);
        assert_eq!(
            derive_static_secret(&master),
            derive_static_secret(&master),
            "the same seed always yields the same static key"
        );
        assert_ne!(
            derive_static_secret(&master),
            master.to_seed(),
            "the Noise static key is not the master seed itself"
        );
        assert_ne!(
            derive_static_secret(&master),
            derive_static_secret(&keypair(8)),
            "different masters yield different static keys"
        );
    }

    #[test]
    fn a_proof_verifies_against_the_session_it_signed() {
        let master = keypair(1);
        let hash = [42u8; 32];
        let payload = build_proof(&master, &hash);

        assert_eq!(payload.len(), PROOF_LEN);
        assert_eq!(
            verify_proof(&payload, &hash).unwrap(),
            PeerID::from_public_key(master.public_key()),
        );
    }

    #[test]
    fn a_proof_from_another_session_is_refused() {
        // The replay the hash binding exists to stop: a proof captured from
        // one session presented in another.
        let master = keypair(1);
        let payload = build_proof(&master, &[42u8; 32]);
        assert_eq!(
            verify_proof(&payload, &[43u8; 32]),
            Err(ProofError::Unproven)
        );
    }

    #[test]
    fn a_proof_signed_by_someone_else_is_refused() {
        let mine = keypair(1);
        let theirs = keypair(2);
        let hash = [42u8; 32];

        // Their key, my signature: swap the key half and it must not verify.
        let mut payload = build_proof(&mine, &hash);
        payload[..32].copy_from_slice(&theirs.public_key().to_bytes());
        assert_eq!(verify_proof(&payload, &hash), Err(ProofError::Unproven));
    }

    #[test]
    fn malformed_payloads_are_refused_rather_than_panicking() {
        assert_eq!(verify_proof(&[], &[0u8; 32]), Err(ProofError::Malformed));
        assert_eq!(
            verify_proof(&[0u8; PROOF_LEN - 1], &[0u8; 32]),
            Err(ProofError::Malformed)
        );
        assert_eq!(
            verify_proof(&[0u8; PROOF_LEN + 1], &[0u8; 32]),
            Err(ProofError::Malformed)
        );
    }
}
