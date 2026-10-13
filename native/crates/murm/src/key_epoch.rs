// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cabal-scoped encryption epochs.
//!
//! The cabal root secret fixes addressing and signing derivation. Encryption
//! epochs rotate underneath that stable identity. A p2panda DCGKA adapter can
//! install the next group secret here after it authorizes and processes a
//! control operation; Murm does not invent membership authority.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::xchacha20::{x_aead_decrypt, x_aead_encrypt};
use stickleback::{DropProtector, NativeDropError};
use zeroize::Zeroizing;

use crate::{CabalId, CabalKey};

const ENVELOPE_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 1 + 8 + NONCE_LEN;

/// Mere-local suite id for p2panda XChaCha protection with an epoch envelope.
pub const CABAL_DROP_SUITE: u16 = 0x4d01;

/// Monotonic encryption generation within one stable cabal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CabalKeyEpoch(pub u64);

/// Invalid local transition or unavailable decryption history.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CabalKeyringError {
    /// Epoch installation skipped or replayed a generation.
    #[error("expected cabal key epoch {expected}, received {actual}")]
    NonSequential {
        /// Required next epoch.
        expected: u64,
        /// Supplied epoch.
        actual: u64,
    },
    /// The requested historical decryption epoch has been forgotten or absent.
    #[error("cabal key epoch {0} is unavailable")]
    MissingEpoch(u64),
}

struct KeyringState {
    current: CabalKeyEpoch,
    keys: BTreeMap<CabalKeyEpoch, Zeroizing<[u8; 32]>>,
}

/// Local key history for one cabal's encrypted drops.
///
/// Clones share state so a protector handed to an export/import task observes
/// later authorized rotations. Old epochs remain readable until the host
/// explicitly forgets them.
#[derive(Clone)]
pub struct CabalKeyring {
    cabal_id: CabalId,
    state: Arc<RwLock<KeyringState>>,
}

impl std::fmt::Debug for CabalKeyring {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().unwrap();
        formatter
            .debug_struct("CabalKeyring")
            .field("cabal_id", &self.cabal_id)
            .field("current", &state.current)
            .field("retained_epochs", &state.keys.len())
            .finish()
    }
}

impl CabalKeyring {
    /// Establish epoch zero by domain-separating the cabal root secret.
    pub fn from_cabal_key(cabal_id: CabalId, cabal_key: &CabalKey) -> Self {
        let initial = blake3::derive_key("mere.murm.cabal-drop-epoch.v1", cabal_key.as_bytes());
        let mut keys = BTreeMap::new();
        keys.insert(CabalKeyEpoch(0), Zeroizing::new(initial));
        Self {
            cabal_id,
            state: Arc::new(RwLock::new(KeyringState {
                current: CabalKeyEpoch(0),
                keys,
            })),
        }
    }

    /// Stable cabal identity to which every protected envelope is bound.
    pub const fn cabal_id(&self) -> CabalId {
        self.cabal_id
    }

    /// Epoch used for new protection operations.
    pub fn current_epoch(&self) -> CabalKeyEpoch {
        self.state.read().unwrap().current
    }

    /// Install the next group secret after the group-state owner authorizes it.
    pub fn install(&self, epoch: CabalKeyEpoch, key: [u8; 32]) -> Result<(), CabalKeyringError> {
        let mut state = self.state.write().unwrap();
        let expected = state.current.0 + 1;
        if epoch.0 != expected {
            return Err(CabalKeyringError::NonSequential {
                expected,
                actual: epoch.0,
            });
        }
        state.keys.insert(epoch, Zeroizing::new(key));
        state.current = epoch;
        Ok(())
    }

    /// Forget decryption keys older than `first_retained`.
    pub fn forget_before(&self, first_retained: CabalKeyEpoch) -> u64 {
        let mut state = self.state.write().unwrap();
        let current = state.current;
        let before = state.keys.len();
        state
            .keys
            .retain(|epoch, _| *epoch >= first_retained || *epoch == current);
        (before - state.keys.len()) as u64
    }

    fn context(&self, aad: &[u8], epoch: CabalKeyEpoch) -> Vec<u8> {
        let mut context = Vec::with_capacity(aad.len() + 32 + 8);
        context.extend_from_slice(aad);
        context.extend_from_slice(self.cabal_id.as_bytes());
        context.extend_from_slice(&epoch.0.to_le_bytes());
        context
    }
}

impl DropProtector for CabalKeyring {
    fn suite_id(&self) -> u16 {
        CABAL_DROP_SUITE
    }

    fn protect(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, NativeDropError> {
        let (epoch, key) = {
            let state = self.state.read().unwrap();
            let epoch = state.current;
            let key = state.keys.get(&epoch).ok_or_else(|| {
                NativeDropError::Protection("current epoch is unavailable".into())
            })?;
            (epoch, Zeroizing::new(**key))
        };
        let nonce = Rng::default()
            .random_array::<NONCE_LEN>()
            .map_err(|error| NativeDropError::Protection(error.to_string()))?;
        let ciphertext = x_aead_encrypt(&key, plaintext, nonce, Some(&self.context(aad, epoch)))
            .map_err(|error| NativeDropError::Protection(error.to_string()))?;
        let mut protected = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        protected.push(ENVELOPE_VERSION);
        protected.extend_from_slice(&epoch.0.to_le_bytes());
        protected.extend_from_slice(&nonce);
        protected.extend_from_slice(&ciphertext);
        Ok(protected)
    }

    fn unprotect(
        &self,
        protected: &[u8],
        aad: &[u8],
        max_plaintext_bytes: u64,
    ) -> Result<Vec<u8>, NativeDropError> {
        if protected.len() < HEADER_LEN || protected[0] != ENVELOPE_VERSION {
            return Err(NativeDropError::Protection(
                "invalid cabal key-epoch envelope".into(),
            ));
        }
        let epoch =
            CabalKeyEpoch(u64::from_le_bytes(protected[1..9].try_into().map_err(
                |_| NativeDropError::Protection("invalid key epoch".into()),
            )?));
        let nonce: [u8; NONCE_LEN] = protected[9..HEADER_LEN]
            .try_into()
            .map_err(|_| NativeDropError::Protection("invalid nonce".into()))?;
        let key = {
            let state = self.state.read().unwrap();
            Zeroizing::new(**state.keys.get(&epoch).ok_or_else(|| {
                NativeDropError::Protection(CabalKeyringError::MissingEpoch(epoch.0).to_string())
            })?)
        };
        let plaintext = x_aead_decrypt(
            &key,
            &protected[HEADER_LEN..],
            nonce,
            Some(&self.context(aad, epoch)),
        )
        .map_err(|error| NativeDropError::Protection(error.to_string()))?;
        if plaintext.len() as u64 > max_plaintext_bytes {
            return Err(NativeDropError::Limit {
                limit: "recovered plaintext bytes",
                actual: plaintext.len() as u64,
                maximum: max_plaintext_bytes,
            });
        }
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_preserves_identity_and_old_reads_until_forgotten() {
        let cabal_id = CabalId::new([4; 32]);
        let keyring = CabalKeyring::from_cabal_key(cabal_id, &CabalKey::new([7; 32]));
        let old = keyring.protect(b"old", b"drop").unwrap();

        keyring.install(CabalKeyEpoch(1), [9; 32]).unwrap();
        let new = keyring.protect(b"new", b"drop").unwrap();
        assert_eq!(keyring.cabal_id(), cabal_id);
        assert_eq!(keyring.unprotect(&old, b"drop", 16).unwrap(), b"old");
        assert_eq!(keyring.unprotect(&new, b"drop", 16).unwrap(), b"new");

        assert_eq!(keyring.forget_before(CabalKeyEpoch(1)), 1);
        assert!(keyring.unprotect(&old, b"drop", 16).is_err());
        assert_eq!(keyring.unprotect(&new, b"drop", 16).unwrap(), b"new");
    }

    #[test]
    fn epochs_must_be_sequential() {
        let keyring = CabalKeyring::from_cabal_key(CabalId::new([4; 32]), &CabalKey::new([7; 32]));
        assert_eq!(
            keyring.install(CabalKeyEpoch(2), [9; 32]),
            Err(CabalKeyringError::NonSequential {
                expected: 1,
                actual: 2,
            })
        );
    }
}
