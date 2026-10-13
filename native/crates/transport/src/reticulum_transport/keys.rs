// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Deterministic Retinue identity construction from a Mere master key.

use hkdf::Hkdf;
use sha2::Sha256;

use identity::Ed25519Keypair;
use retinue::identity::PrivateIdentity;

/// HKDF-SHA256 context string for the X25519 half. Changing this changes every
/// derived Retinue identity, so treat it as a wire-format version.
const X25519_HKDF_INFO: &[u8] = b"mere-reticulum-identity-v1";

/// Derive a deterministic retinue identity from a Mere master keypair.
///
/// Reticulum needs a dual-key identity: 64 secret bytes, `x25519_secret(32) ++
/// ed25519_seed(32)`. The ECDH half is domain-separated through HKDF. The
/// signing half is Mere's master Ed25519 seed itself, so a verified Reticulum
/// identity carries the same public key as Mere's `PeerID`.
///
/// The derivation is pure, so the same master seed always yields the same retinue
/// destination across restarts.
pub(super) fn derive_identity(master: &Ed25519Keypair) -> PrivateIdentity {
    let seed = master.to_seed();
    let hk = Hkdf::<Sha256>::new(None, &seed);
    let mut secret = [0u8; 64];
    hk.expand(X25519_HKDF_INFO, &mut secret[..32])
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    secret[32..].copy_from_slice(&seed);

    PrivateIdentity::from_secret_bytes(&secret)
}
