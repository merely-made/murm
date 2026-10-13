// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Role-tagged DTLS fingerprint canonicalization.
//!
//! WebRTC authenticates the DTLS connection described by the fingerprints the
//! two ends exchanged in SDP. It does not establish a subject, and browsers do
//! not expose a DTLS exporter to ordinary application script, so the
//! fingerprints themselves are the only handle the carrier has on "this
//! connection and no other".
//!
//! Two rules follow, and both are wire-freezing.
//!
//! **The role travels with the digest.** A fingerprint is canonicalized as a
//! one-byte role tag followed by the 32 raw digest bytes, so the client and
//! server halves are not interchangeable. Swapping them yields a different
//! transcript, a different shared link, and a failed session — which is what
//! stops a signaling intermediary from terminating two DTLS sessions and
//! replaying one end's binding at the other.
//!
//! **The text form is parsed strictly or not at all.** A fingerprint that
//! nearly parses is worth less than one that does not parse: a silent
//! truncation would bind a session to a prefix and call it proof.

use std::fmt;

use crate::codec::{hex_digit_upper, to_hex_upper};
use crate::error::FingerprintError;

/// Width of a SHA-256 DTLS fingerprint digest.
pub const DTLS_FINGERPRINT_BYTES: usize = 32;

/// Width of the canonical role-tagged form: one role byte plus the digest.
pub const CANONICAL_FINGERPRINT_BYTES: usize = 1 + DTLS_FINGERPRINT_BYTES;

/// The SDP hash-function token this carrier accepts.
pub const FINGERPRINT_ALGORITHM: &str = "sha-256";

/// Which end of the DTLS handshake a fingerprint belongs to.
///
/// Named for the DTLS roles rather than the WebRTC ones because the DTLS role
/// is what the fingerprint actually describes. In this carrier the browser is
/// the [`Client`](FingerprintRole::Client) — it offers, initiates, and holds
/// the invitation — and the native host is the
/// [`Server`](FingerprintRole::Server), which answers and signs the
/// transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FingerprintRole {
    /// The initiating end: the browser.
    Client,
    /// The responding end: the native host.
    Server,
}

impl FingerprintRole {
    /// The byte this role contributes to a canonical fingerprint.
    ///
    /// Neither tag is zero. A zeroed buffer is therefore not a valid
    /// canonical fingerprint of either role, so a caller who forgot to fill
    /// one in gets a mismatch rather than a plausible-looking transcript.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Client => 0x01,
            Self::Server => 0x02,
        }
    }

    /// The role's name, for error text.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

impl fmt::Display for FingerprintRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One end's SHA-256 DTLS certificate fingerprint, tagged with its role.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DtlsFingerprint {
    role: FingerprintRole,
    digest: [u8; DTLS_FINGERPRINT_BYTES],
}

impl DtlsFingerprint {
    /// Tags 32 raw digest bytes with a role.
    ///
    /// The raw form is the primary constructor: a native stack usually has
    /// the digest in hand and should not have to render it to text and parse
    /// it back to enter a transcript.
    pub const fn new(role: FingerprintRole, digest: [u8; DTLS_FINGERPRINT_BYTES]) -> Self {
        Self { role, digest }
    }

    /// Parses the colon-separated hex an SDP `a=fingerprint` line carries.
    ///
    /// Strict, per RFC 8122's `2UHEX *(":" 2UHEX)`: exactly 32 groups, exactly
    /// two hex digits each, uppercase only, single colons, no surrounding
    /// whitespace. Browsers emit exactly this. Lowercase is a reject rather
    /// than a quiet acceptance, so a stack that drifts from the grammar shows
    /// up as an error at the first handshake instead of as a difference of
    /// opinion about a fingerprint later.
    pub fn parse_sdp_hex(role: FingerprintRole, text: &str) -> Result<Self, FingerprintError> {
        let bytes = text.as_bytes();
        let groups: Vec<&[u8]> = bytes.split(|byte| *byte == b':').collect();
        if groups.len() != DTLS_FINGERPRINT_BYTES {
            return Err(FingerprintError::OctetCount { got: groups.len() });
        }
        let mut digest = [0u8; DTLS_FINGERPRINT_BYTES];
        for (index, group) in groups.iter().enumerate() {
            if group.len() != 2 {
                return Err(FingerprintError::Octet { index });
            }
            let hi = hex_digit_upper(group[0]).ok_or(FingerprintError::Octet { index })?;
            let lo = hex_digit_upper(group[1]).ok_or(FingerprintError::Octet { index })?;
            digest[index] = (hi << 4) | lo;
        }
        Ok(Self { role, digest })
    }

    /// Parses a whole SDP `a=fingerprint:` attribute value — the
    /// `sha-256 AB:CD:...` after the colon.
    ///
    /// The algorithm token is compared case-insensitively: it names an
    /// algorithm rather than contributing bytes to any transcript, so its
    /// spelling cannot affect the derived link. Anything other than
    /// [`FINGERPRINT_ALGORITHM`] is refused outright — an SHA-1 fingerprint is
    /// not a weaker binding this carrier accepts with a warning, it is one it
    /// does not accept.
    pub fn parse_sdp_attribute(
        role: FingerprintRole,
        value: &str,
    ) -> Result<Self, FingerprintError> {
        let mut parts = value.split(' ').filter(|part| !part.is_empty());
        let algorithm = parts.next().ok_or(FingerprintError::Attribute)?;
        let hex = parts.next().ok_or(FingerprintError::Attribute)?;
        if parts.next().is_some() {
            return Err(FingerprintError::Attribute);
        }
        if !algorithm.eq_ignore_ascii_case(FINGERPRINT_ALGORITHM) {
            return Err(FingerprintError::Algorithm {
                got: algorithm.to_owned(),
            });
        }
        Self::parse_sdp_hex(role, hex)
    }

    /// Which end this fingerprint describes.
    pub const fn role(&self) -> FingerprintRole {
        self.role
    }

    /// The raw digest, without the role tag.
    pub const fn digest(&self) -> &[u8; DTLS_FINGERPRINT_BYTES] {
        &self.digest
    }

    /// The canonical form: role tag, then digest.
    ///
    /// This is the only shape that enters a transcript.
    pub fn canonical_bytes(&self) -> [u8; CANONICAL_FINGERPRINT_BYTES] {
        let mut out = [0u8; CANONICAL_FINGERPRINT_BYTES];
        out[0] = self.role.tag();
        out[1..].copy_from_slice(&self.digest);
        out
    }

    /// Renders the digest back to SDP's uppercase colon-separated hex.
    ///
    /// The role tag is not part of the text form; SDP has no place to carry
    /// it, and the role is known from which description the line came out of.
    pub fn to_sdp_hex(&self) -> String {
        let hex = to_hex_upper(&self.digest);
        let mut out = String::with_capacity(DTLS_FINGERPRINT_BYTES * 3 - 1);
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            if index > 0 {
                out.push(':');
            }
            out.push(pair[0] as char);
            out.push(pair[1] as char);
        }
        out
    }
}

impl fmt::Debug for DtlsFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DtlsFingerprint({}, {})", self.role, self.to_sdp_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hex() -> String {
        DtlsFingerprint::new(FingerprintRole::Client, [0xab; DTLS_FINGERPRINT_BYTES]).to_sdp_hex()
    }

    #[test]
    fn sdp_hex_round_trips() {
        let text = sample_hex();
        assert!(text.starts_with("AB:AB:"));
        assert_eq!(text.len(), DTLS_FINGERPRINT_BYTES * 3 - 1);
        let parsed =
            DtlsFingerprint::parse_sdp_hex(FingerprintRole::Client, &text).expect("parses");
        assert_eq!(parsed.digest(), &[0xab; DTLS_FINGERPRINT_BYTES]);
        assert_eq!(parsed.role(), FingerprintRole::Client);
    }

    #[test]
    fn an_attribute_value_round_trips() {
        let value = format!("sha-256 {}", sample_hex());
        let parsed =
            DtlsFingerprint::parse_sdp_attribute(FingerprintRole::Server, &value).expect("parses");
        assert_eq!(parsed.role(), FingerprintRole::Server);
        assert_eq!(parsed.digest(), &[0xab; DTLS_FINGERPRINT_BYTES]);
    }

    #[test]
    fn a_weaker_algorithm_is_refused() {
        let value = format!("sha-1 {}", sample_hex());
        assert_eq!(
            DtlsFingerprint::parse_sdp_attribute(FingerprintRole::Server, &value),
            Err(FingerprintError::Algorithm {
                got: "sha-1".to_owned()
            })
        );
    }

    #[test]
    fn lowercase_hex_is_refused() {
        let text = sample_hex().to_ascii_lowercase();
        assert_eq!(
            DtlsFingerprint::parse_sdp_hex(FingerprintRole::Client, &text),
            Err(FingerprintError::Octet { index: 0 })
        );
    }

    #[test]
    fn the_role_tag_distinguishes_the_two_ends() {
        let digest = [0x5a; DTLS_FINGERPRINT_BYTES];
        let client = DtlsFingerprint::new(FingerprintRole::Client, digest);
        let server = DtlsFingerprint::new(FingerprintRole::Server, digest);
        assert_ne!(client.canonical_bytes(), server.canonical_bytes());
        assert_eq!(client.canonical_bytes()[1..], server.canonical_bytes()[1..]);
    }
}
