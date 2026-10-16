//! Pairwise primitives from RFC 7748 and the public Meshtastic security
//! reference: SHA-256 of X25519, AES-256-CCM, an eight-byte tag, no AAD.
//! Wire nonce placement and routing envelopes require separate stock captures.
//! This module neither invents identities nor treats public node IDs as keys.

use alloc::vec::Vec;
use ccm::{
    Ccm,
    aead::{Aead, KeyInit},
    consts::{U8, U13},
};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

type Cipher = Ccm<aes::Aes256, U8, U13>;
/// Payload capacity after the eight-byte tag and four-byte extra nonce.
pub const MAX_PLAINTEXT: usize = crate::transport::MAX_PAYLOAD_LEN - 12;

/// A released protocol secret. Only its X25519 public key can be read back.
pub struct LocalIdentity(StaticSecret);
impl core::fmt::Debug for LocalIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("LocalIdentity(<redacted>)")
    }
}
impl LocalIdentity {
    /// Consume one explicitly released protocol secret; no RNG or fallback.
    pub fn from_secret(mut secret: [u8; 32]) -> Self {
        let identity = Self(StaticSecret::from(secret));
        secret.zeroize();
        identity
    }
    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }
    /// Refuse low-order/noncontributory peers before deriving any cipher key.
    pub fn pairwise(&self, peer: [u8; 32]) -> Result<PairwiseKey, PkiError> {
        let shared = self.0.diffie_hellman(&PublicKey::from(peer));
        if !shared.was_contributory() {
            return Err(PkiError::NoncontributoryPeer);
        }
        let mut digest = Sha256::digest(shared.as_bytes());
        let key = PairwiseKey(Zeroizing::new(digest[..].try_into().unwrap()));
        digest[..].zeroize();
        Ok(key)
    }
}

/// Zeroizing, redacted pairwise cipher material. It is not serializable.
pub struct PairwiseKey(Zeroizing<[u8; 32]>);
impl core::fmt::Debug for PairwiseKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PairwiseKey(<redacted>)")
    }
}
impl PairwiseKey {
    /// Caller must reserve a fresh source/packet ID before constructing the
    /// nonce. Reusing a nonce with this key is forbidden across reconnects.
    pub fn seal(&self, nonce: &[u8; 13], plaintext: &[u8]) -> Result<Vec<u8>, PkiError> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(PkiError::Length);
        }
        Cipher::new_from_slice(self.0.as_ref())
            .unwrap()
            .encrypt(nonce.into(), plaintext)
            .map_err(|_| PkiError::Authentication)
    }
    /// Nothing is returned when the complete tag fails authentication.
    pub fn open(&self, nonce: &[u8; 13], ciphertext_tag: &[u8]) -> Result<Vec<u8>, PkiError> {
        if !(8..=MAX_PLAINTEXT + 8).contains(&ciphertext_tag.len()) {
            return Err(PkiError::Length);
        }
        Cipher::new_from_slice(self.0.as_ref())
            .unwrap()
            .decrypt(nonce.into(), ciphertext_tag)
            .map_err(|_| PkiError::Authentication)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkiError {
    NoncontributoryPeer,
    Length,
    Authentication,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    fn hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| u8::from_str_radix(core::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn independent_cryptography_oracle_and_authentication_refusals() {
        let alice = LocalIdentity::from_secret([7; 32]);
        let bob = LocalIdentity::from_secret([9; 32]);
        let a = alice.pairwise(bob.public_key()).unwrap();
        let b = bob.pairwise(alice.public_key()).unwrap();
        let nonce = core::array::from_fn(|i| i as u8);
        let plain = b"Sennet pairwise primitive";
        let encrypted = a.seal(&nonce, plain).unwrap();
        assert_eq!(
            alice.public_key().as_slice(),
            hex("13be4feaeaf204c7fd3358fc9c00721881d174278128227ec674f37f7fe97b6d")
        );
        assert_eq!(
            bob.public_key().as_slice(),
            hex("57db4b359f23ae5e146e4e2512056704722506348c150c14753d0c933d04d421")
        );
        assert_eq!(
            encrypted,
            hex("6554e112867ff37e16d638b1cf2bf9291e7a7edbf9b184cee8ebe2929109eef935")
        );
        assert_eq!(b.open(&nonce, &encrypted).unwrap(), plain);
        assert_eq!(format!("{alice:?}"), "LocalIdentity(<redacted>)");
        assert_eq!(format!("{a:?}"), "PairwiseKey(<redacted>)");
        let wrong = LocalIdentity::from_secret([11; 32])
            .pairwise(alice.public_key())
            .unwrap();
        assert_eq!(
            wrong.open(&nonce, &encrypted),
            Err(PkiError::Authentication)
        );
        for i in 0..encrypted.len() {
            let mut damaged = encrypted.clone();
            damaged[i] ^= 1;
            assert_eq!(b.open(&nonce, &damaged), Err(PkiError::Authentication));
        }
        for i in 0..nonce.len() {
            let mut damaged = nonce;
            damaged[i] ^= 1;
            assert_eq!(b.open(&damaged, &encrypted), Err(PkiError::Authentication));
        }
        assert!(matches!(
            alice.pairwise([0; 32]),
            Err(PkiError::NoncontributoryPeer)
        ));
        assert_eq!(
            a.seal(&nonce, &[0; MAX_PLAINTEXT + 1]),
            Err(PkiError::Length)
        );
        assert_eq!(a.open(&nonce, &[0; 7]), Err(PkiError::Length));
    }
}
