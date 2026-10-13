// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Peer-identifier type for transport operations.

use identity::{Ed25519PublicKey, IdentityError};

/// A peer's identifier on the transport network.
///
/// The peer's transport public key: a key [`identity`] derives for that
/// transport, never a persona's master (vault lock plan, rulings 92 and 96).
/// For iroh-backed transports, this maps directly to iroh's `NodeId` (their
/// term for the same underlying public key).
///
/// Named `PeerID` rather than `NodeId` to disambiguate from
/// `kernel::Graph` node identity — graph-object identity and peer
/// identity are distinct concepts that previously shared the name.
///
/// Carries equality and hashing semantics of the underlying public key, so
/// `PeerID` works as a `HashMap` / `HashSet` key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PeerID(Ed25519PublicKey);

impl PeerID {
    /// Construct a `PeerID` from a master Ed25519 public key.
    pub fn from_public_key(pk: Ed25519PublicKey) -> Self {
        Self(pk)
    }

    /// The underlying public key.
    pub fn public_key(&self) -> Ed25519PublicKey {
        self.0
    }

    /// 32-byte canonical encoding of the underlying public key.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// Decode from a 32-byte public-key encoding.
    ///
    /// Returns the same error as [`Ed25519PublicKey::from_bytes`] if the bytes
    /// don't form a valid public key.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, IdentityError> {
        Ed25519PublicKey::from_bytes(bytes).map(Self)
    }
}

impl From<Ed25519PublicKey> for PeerID {
    fn from(pk: Ed25519PublicKey) -> Self {
        Self(pk)
    }
}
