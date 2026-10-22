//! Destinations and their private keys.
//!
//! A destination is 256 bytes of encryption key, 128 of signing key, and a certificate whose
//! 3-byte header (type, big-endian length) at offset 384 says how much more follows. A SAM
//! private key is the destination followed by its private keys, so the public half is found
//! by reading that certificate length.

use sha2::{Digest, Sha256};

use crate::encoding::{base32_encode, base64_decode, base64_encode};
use crate::{Error, Result};

/// EdDSA-SHA512-Ed25519, the signature type RNS and current routers use.
pub const SIGNATURE_ED25519: u16 = 7;

const CERT_AT: usize = 384;
const HEADER_LEN: usize = CERT_AT + 3;

/// A public destination.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Destination {
    bytes: Vec<u8>,
}

impl Destination {
    /// Parse a base64 destination as SAM and the netDB carry it.
    pub fn from_base64(text: &str) -> Result<Self> {
        let bytes = base64_decode(text).ok_or(Error::InvalidInput("destination is not base64"))?;
        Self::from_bytes(bytes)
    }

    /// Wrap destination bytes, checking that the certificate accounts for all of them.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        match public_len(&bytes) {
            Some(len) if len == bytes.len() => Ok(Self { bytes }),
            _ => Err(Error::InvalidInput(
                "destination length disagrees with its certificate",
            )),
        }
    }

    /// The raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Base64, as SAM takes it.
    pub fn to_base64(&self) -> String {
        base64_encode(&self.bytes)
    }

    /// The `.b32.i2p` name: base32 of the destination's SHA-256.
    pub fn b32(&self) -> String {
        let mut name = base32_encode(&Sha256::digest(&self.bytes));
        name.push_str(".b32.i2p");
        name
    }
}

impl core::fmt::Debug for Destination {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Destination").field(&self.b32()).finish()
    }
}

/// A destination's private key, in SAM's base64 form. Its `Debug` shows only the public name.
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKey {
    base64: String,
    public: Destination,
}

impl PrivateKey {
    /// Parse SAM's `PRIV` / `DESTINATION` base64, which RNS also stores as an I2P keyfile.
    pub fn from_base64(text: &str) -> Result<Self> {
        let text = text.trim();
        let bytes = base64_decode(text).ok_or(Error::InvalidInput("private key is not base64"))?;
        let len =
            public_len(&bytes)
                .filter(|&len| len < bytes.len())
                .ok_or(Error::InvalidInput(
                    "private key is shorter than its destination",
                ))?;
        let public = Destination::from_bytes(bytes[..len].to_vec())?;
        Ok(Self {
            base64: text.to_string(),
            public,
        })
    }

    /// The public destination.
    pub fn destination(&self) -> &Destination {
        &self.public
    }

    /// The secret base64, for storing it or handing it to `SESSION CREATE`.
    pub fn expose_base64(&self) -> &str {
        &self.base64
    }
}

impl core::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("PrivateKey")
            .field(&self.public.b32())
            .finish()
    }
}

fn public_len(bytes: &[u8]) -> Option<usize> {
    let cert = bytes.get(CERT_AT + 1..HEADER_LEN)?;
    Some(HEADER_LEN + usize::from(u16::from_be_bytes([cert[0], cert[1]])))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A destination with a 4-byte key certificate, and a private key after it.
    pub(crate) fn sample(seed: u8) -> (Vec<u8>, Vec<u8>) {
        let mut public: Vec<u8> = (0..CERT_AT).map(|i| (i as u8).wrapping_mul(seed)).collect();
        public.extend([5, 0, 4, 0, 7, 0, 4]);
        let mut private = public.clone();
        private.extend([seed; 256 + 32]);
        (public, private)
    }

    #[test]
    fn a_private_key_yields_its_destination_and_b32() {
        let (public, private) = sample(3);
        let key = PrivateKey::from_base64(&base64_encode(&private)).unwrap();
        assert_eq!(key.destination().as_bytes(), public.as_slice());
        let dest = Destination::from_base64(&base64_encode(&public)).unwrap();
        assert_eq!(&dest, key.destination());
        let b32 = dest.b32();
        assert_eq!(b32.len(), 52 + ".b32.i2p".len());
        assert!(b32.ends_with(".b32.i2p"));
        assert_eq!(&b32[..52], base32_encode(&Sha256::digest(&public)));
        // Debug never prints the secret.
        assert!(!format!("{key:?}").contains(key.expose_base64()));
    }

    #[test]
    fn lengths_must_agree_with_the_certificate() {
        let (public, _) = sample(1);
        assert!(Destination::from_bytes(public[..public.len() - 1].to_vec()).is_err());
        let mut long = public.clone();
        long.push(0);
        assert!(Destination::from_bytes(long).is_err());
        // A "private key" that is only a destination has no private half.
        assert!(PrivateKey::from_base64(&base64_encode(&public)).is_err());
        assert!(Destination::from_base64("not base64!").is_err());
    }
}
