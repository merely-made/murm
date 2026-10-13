// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Announce-based discovery with an authenticated `PeerID` binding.
//!
//! A Retinue destination hash cannot be synthesized from a Mere [`PeerID`]
//! because the identity also contains an X25519 key, so a destination must be
//! learned from an announce. Mere uses its master Ed25519 key as the signing
//! half of that Retinue identity. Retinue's verified announce therefore binds
//! the `PeerID`, dual-key identity, and destination name without extra app data.

use identity::{Ed25519Keypair, Ed25519Signature};
use retinue::destination::DestinationName;
use retinue::identity::Identity;

use crate::PeerID;

/// Legacy `peer_id(32) ++ master_signature(64)` app data.
const LEGACY_APP_DATA_LEN: usize = 32 + 64;

/// The message the old discovery format signed to bind a `PeerID` to a Retinue
/// identity and destination name.
///
/// Binding to the name hash (not just the identity) stops a peer from replaying
/// one ALPN's binding to claim a `PeerID` on a different ALPN.
fn binding_message(peer_id: &PeerID, name: &DestinationName, identity: &Identity) -> Vec<u8> {
    let mut msg = Vec::with_capacity(32 + 32 + 32 + 10);
    msg.extend_from_slice(identity.x25519_bytes());
    msg.extend_from_slice(identity.ed25519_bytes());
    msg.extend_from_slice(&peer_id.to_bytes());
    msg.extend_from_slice(name.name_hash().as_slice());
    msg
}

/// Build the current announce app data.
///
/// It is intentionally empty. The already-signed Retinue identity contains the
/// Mere public key, saving the 96-byte duplicate binding that made an otherwise
/// valid announce 263 bytes and therefore too large for a 255-byte LoRa frame.
pub(super) fn build_app_data(
    peer_id: &PeerID,
    _name: &DestinationName,
    identity: &Identity,
    master: &Ed25519Keypair,
) -> Vec<u8> {
    debug_assert_eq!(identity.ed25519_bytes(), &peer_id.to_bytes());
    debug_assert_eq!(master.public_key().to_bytes(), peer_id.to_bytes());
    Vec::new()
}

/// Recover the `PeerID` an announce binds to this Retinue identity.
///
/// Empty app data is the current format. The legacy 96-byte binding remains
/// readable so an updated node can still learn an older peer.
pub(super) fn recover_peer_id(
    app_data: &[u8],
    name: &DestinationName,
    identity: &Identity,
) -> Option<PeerID> {
    if app_data.is_empty() {
        return PeerID::from_bytes(identity.ed25519_bytes()).ok();
    }
    if app_data.len() < LEGACY_APP_DATA_LEN {
        return None;
    }
    let peer_bytes: [u8; 32] = app_data[..32].try_into().ok()?;
    let peer_id = PeerID::from_bytes(&peer_bytes).ok()?;
    let sig_bytes: [u8; 64] = app_data[32..96].try_into().ok()?;
    let signature = Ed25519Signature::from_bytes(&sig_bytes);

    if peer_id
        .public_key()
        .verify(&binding_message(&peer_id, name, identity), &signature)
    {
        Some(peer_id)
    } else {
        None
    }
}
