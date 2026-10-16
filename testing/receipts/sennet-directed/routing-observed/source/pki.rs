//! Pairwise primitives from RFC 7748 and the public Meshtastic security
//! reference: SHA-256 of X25519, AES-256-CCM, an eight-byte tag, no AAD.
//! Wire nonce placement is pinned by the identified 2.7.26 stock receive.
//! Routing envelopes require their own controlled captures.
//! This module neither invents identities nor treats public node IDs as keys.

use crate::transport::{BROADCAST_DESTINATION, Header, Packet};
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
    /// Bind a public key to the operator-pinned full node addresses. Public
    /// node numbers alone do not establish this binding or discover a key.
    pub fn peer(&self, local: u32, remote: u32, public: [u8; 32]) -> Result<PeerSession, PkiError> {
        if local == remote
            || matches!(local, 0 | BROADCAST_DESTINATION)
            || matches!(remote, 0 | BROADCAST_DESTINATION)
            || public == self.public_key()
        {
            return Err(PkiError::WrongScope);
        }
        Ok(PeerSession {
            local,
            remote,
            key: self.pairwise(public)?,
        })
    }
}

/// One explicitly pinned peer's directed packet codec. No channel fallback.
#[derive(Debug)]
pub struct PeerSession {
    local: u32,
    remote: u32,
    key: PairwiseKey,
}
impl PeerSession {
    /// The caller owns durable fresh IDs and random extra-nonce generation.
    /// Hop/routing flags are deliberately outside CCM's associated data.
    pub fn seal_packet(
        &self,
        header: Header,
        extra: [u8; 4],
        plain: &[u8],
    ) -> Result<Vec<u8>, PkiError> {
        if header.source != self.local
            || header.destination != self.remote
            || header.channel_hash != 0
        {
            return Err(PkiError::WrongScope);
        }
        let mut payload = self.key.seal(&packet_nonce(header, extra), plain)?;
        payload.extend_from_slice(&extra);
        Packet { header, payload }
            .encode()
            .map_err(|_| PkiError::InvalidFrame)
    }
    pub fn open_packet(&self, frame: &[u8]) -> Result<(Header, Vec<u8>), PkiError> {
        let packet = Packet::decode(frame).map_err(|_| PkiError::InvalidFrame)?;
        let header = packet.header;
        if header.source != self.remote
            || header.destination != self.local
            || header.channel_hash != 0
        {
            return Err(PkiError::WrongScope);
        }
        if packet.payload.len() < 12 {
            return Err(PkiError::Length);
        }
        let (cipher, extra) = packet.payload.split_at(packet.payload.len() - 4);
        let plain = self
            .key
            .open(&packet_nonce(header, extra.try_into().unwrap()), cipher)?;
        Ok((header, plain))
    }
}

fn packet_nonce(header: Header, extra: [u8; 4]) -> [u8; 13] {
    let mut nonce = [0; 13];
    nonce[..4].copy_from_slice(&header.packet_id.to_le_bytes());
    nonce[4..8].copy_from_slice(&extra);
    nonce[8..12].copy_from_slice(&header.source.to_le_bytes());
    nonce
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
    WrongScope,
    InvalidFrame,
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
    #[test]
    fn directed_packets_are_bound_to_pinned_addresses_and_capture_nonce_layout() {
        let alice = LocalIdentity::from_secret([7; 32]);
        let bob = LocalIdentity::from_secret([9; 32]);
        let a = alice.peer(11, 22, bob.public_key()).unwrap();
        let b = bob.peer(22, 11, alice.public_key()).unwrap();
        let header = Header {
            source: 11,
            destination: 22,
            packet_id: 101,
            hop_limit: 3,
            want_ack: true,
            via_mqtt: false,
            hop_start: 3,
            channel_hash: 0,
            next_hop: 0,
            relay_node: 11,
        };
        let bytes = a.seal_packet(header, [0, 1, 2, 3], b"test").unwrap();
        assert_eq!(
            bytes,
            hex("160000000b000000650000006b00000b0525a6f6ae7156f6b87cb30100010203")
        );
        assert_eq!(b.open_packet(&bytes).unwrap(), (header, b"test".to_vec()));
        for offset in [0, 4, 13] {
            let mut bad = bytes.clone();
            bad[offset] ^= 1;
            assert_eq!(b.open_packet(&bad), Err(PkiError::WrongScope));
        }
        for offset in 16..bytes.len() {
            let mut bad = bytes.clone();
            bad[offset] ^= 1;
            assert_eq!(b.open_packet(&bad), Err(PkiError::Authentication));
        }
        assert!(matches!(
            alice.peer(11, 22, alice.public_key()),
            Err(PkiError::WrongScope)
        ));
        assert!(matches!(
            alice.peer(11, 11, bob.public_key()),
            Err(PkiError::WrongScope)
        ));
        assert_eq!(
            a.seal_packet(
                Header {
                    destination: u32::MAX,
                    ..header
                },
                [0; 4],
                b"test"
            ),
            Err(PkiError::WrongScope)
        );
        // Public values from the authenticated stock capture, without its keys.
        assert_eq!(
            packet_nonce(
                Header {
                    source: 0xf66afb28,
                    packet_id: 0x4a46dd5f,
                    ..header
                },
                [0x2d, 0xf4, 0x51, 0x58]
            ),
            [
                0x5f, 0xdd, 0x46, 0x4a, 0x2d, 0xf4, 0x51, 0x58, 0x28, 0xfb, 0x6a, 0xf6, 0
            ]
        );
    }
}
