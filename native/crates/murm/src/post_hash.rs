// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! BLAKE3 cabal-id derivation.
//!
//! Mere hashes with BLAKE3 throughout: no BLAKE2b, no Cable-spec salt /
//! personalization parameters, no cabal.club wire interop. A cabal id is the
//! plain BLAKE3-256 hash of the (secret) cabal key.
//!
//! Post (operation) ids are *not* here: a post's id is its signed-header hash
//! (`Header::hash()`), computed in [`crate::post_wire::operation_id`], so the
//! id doubles as the p2panda log/backlink identity.

/// Derive a public cabal-id from a (secret) cabal key.
///
/// `cabal_id = BLAKE3-256(cabal_key)`. Mere-internal: a public address peers
/// can use to refer to a cabal without revealing the key. Returned as a 32-byte
/// array (the caller in `murm` wraps it in `murm::CabalId`).
pub fn hash_cabal_id(cabal_key: &[u8; 32]) -> [u8; 32] {
    *blake3::hash(cabal_key).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabal_id_is_deterministic_and_distinct() {
        let id = hash_cabal_id(&[0u8; 32]);
        assert_eq!(id, hash_cabal_id(&[0u8; 32]));
        assert_ne!(id, hash_cabal_id(&[1u8; 32]));
    }
}
