// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Verified local facts about a completed Noise handshake and protocol exchange.

use crate::{Alpn, PeerID};

/// Internal Noise XX/BLAKE2b profile with the Mere Ed25519 identity proof.
pub const NOISE_PROFILE: &[u8] = b"mere/noise-xx-25519-chachapoly-blake2b-ed25519";
/// Version of the profile's framing, identity proof and protocol declaration.
pub const NOISE_PROFILE_VERSION: u16 = 1;

/// Facts produced only after the remote Ed25519 session proof verifies.
///
/// Participants are always ordered by their handshake roles, so both ends
/// derive the same common binding. The hash identifies this handshake, not a
/// public key or a persistent principal. These facts do not grant authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoiseHandshakeFacts {
    pub(super) initiator: PeerID,
    pub(super) responder: PeerID,
    pub(super) handshake_hash: [u8; 64],
    pub(super) local_is_initiator: bool,
}

impl NoiseHandshakeFacts {
    /// The exact internal security profile implemented by this session.
    pub fn profile(&self) -> &'static [u8] {
        NOISE_PROFILE
    }
    /// The version of the internal profile.
    pub fn version(&self) -> u16 {
        NOISE_PROFILE_VERSION
    }
    /// Noise's BLAKE2b handshake hash, after all three XX messages.
    pub fn handshake_hash(&self) -> &[u8; 64] {
        &self.handshake_hash
    }
    /// The proven initiator's Ed25519 identity.
    pub fn initiator(&self) -> PeerID {
        self.initiator
    }
    /// The proven responder's Ed25519 identity.
    pub fn responder(&self) -> PeerID {
        self.responder
    }
    /// The remote participant's proven Ed25519 identity.
    pub fn peer(&self) -> PeerID {
        if self.local_is_initiator {
            self.responder
        } else {
            self.initiator
        }
    }
}

/// Verified handshake facts and the authenticated inner protocol declaration.
///
/// This is local adapter output, not a claim decoded from application bytes.
/// The initiator authored the declaration; the responder read it inside the
/// authenticated Noise channel. Admission still checks its expected protocol,
/// remote identity, delegation and the carrier's separate binding facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoiseSessionFacts {
    pub(super) handshake: NoiseHandshakeFacts,
    pub(super) protocol: Alpn,
}

impl NoiseSessionFacts {
    /// The verified handshake underlying this protocol session.
    pub fn handshake(&self) -> &NoiseHandshakeFacts {
        &self.handshake
    }
    /// The authenticated inner protocol declaration.
    pub fn protocol(&self) -> &Alpn {
        &self.protocol
    }
    /// The remote participant's proven Ed25519 identity.
    pub fn peer(&self) -> PeerID {
        self.handshake.peer()
    }
}
