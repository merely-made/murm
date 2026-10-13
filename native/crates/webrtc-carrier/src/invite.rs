// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Invitation identifiers and the C2 invitation payload.
//!
//! [`InviteId`] is the identifier: opaque, 16 bytes, no internal structure
//! this crate reads, no version byte, no checksum. Whoever mints one decides
//! what it means. It is what the carrier core needs to bind *which*
//! invitation a channel belongs to, and it is bound into
//! [`LinkChallenge`](crate::LinkChallenge) without ever carrying the
//! invitation's authority.
//!
//! [`InviteV1`] is that authority: the redemption secret, the expected host
//! key, network and profile references, the permitted service action, the
//! expiry and use ceiling, and a [`ReleaseRefV1`](luggage::ReleaseRefV1)
//! naming the release the
//! inviter claims to be running (browser WebRTC carrier plan §5). It is a
//! bounded, versioned payload meant to travel in a URL fragment — see
//! [`InviteV1::to_fragment`] — never in a query string, a referrer, or a log
//! line, because it carries [`InviteV1::redemption_seed`].
//!
//! Two more transcripts live here alongside it, both over a
//! [`LinkChallenge`](crate::LinkChallenge): [`challenge_signature_bytes`] is
//! what the host signs to vouch for a link, and [`redemption_signing_bytes`]
//! is what the browser's ephemeral redemption key signs to prove possession
//! of the invite's secret without ever handing that secret, or the key, to
//! the host. Three domain strings —
//! [`INVITE_DESCRIPTOR_DOMAIN`], [`HOST_CHALLENGE_SIGNATURE_DOMAIN`], and
//! [`REDEMPTION_PROOF_DOMAIN`] — keep those three signable byte strings from
//! ever colliding with each other or with [`SHARED_LINK_DOMAIN`](crate::SHARED_LINK_DOMAIN),
//! even though more than one of them wraps the same [`LinkChallenge::encode`](crate::LinkChallenge::encode)
//! transcript.

use std::fmt;
use std::hash::{Hash, Hasher};

use luggage::ReleaseRefV1;

use crate::challenge::{LinkChallenge, MAX_TRANSCRIPT_FIELD_BYTES};
use crate::codec::{b64url_decode, b64url_encode, ct_eq, hex_digit, push_field, to_hex_lower};
use crate::error::{InviteError, InviteIdError};

/// Width of an invitation identifier.
///
/// 16 bytes, matching Notochord's link identifier: wide enough that a public
/// rendezvous reference cannot be guessed, narrow enough to sit in a URL
/// fragment beside the redemption secret without the fragment becoming a
/// paragraph.
pub const INVITE_ID_BYTES: usize = 16;

/// The public reference to one invitation.
///
/// Equality is constant-time. The identifier is a public rendezvous reference
/// rather than a secret — the secret is the redemption value C2 adds — but it
/// is compared against attacker-supplied input on the accept path, and a
/// comparison that leaks its match prefix through timing is a habit worth not
/// having in a carrier.
#[derive(Clone, Copy, Eq)]
pub struct InviteId([u8; INVITE_ID_BYTES]);

impl InviteId {
    /// Wraps 16 raw bytes.
    pub const fn from_bytes(bytes: [u8; INVITE_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw bytes.
    pub const fn as_bytes(&self) -> &[u8; INVITE_ID_BYTES] {
        &self.0
    }

    /// Copies the raw bytes out.
    pub const fn to_bytes(&self) -> [u8; INVITE_ID_BYTES] {
        self.0
    }

    /// Renders the identifier as 32 lowercase hex characters.
    pub fn to_hex(&self) -> String {
        to_hex_lower(&self.0)
    }

    /// Parses 32 hex characters, either case, no separators.
    ///
    /// Case-insensitive on purpose, and this is not a hole in the strictness
    /// the fingerprint parser insists on: the transcript binds
    /// [`as_bytes`](Self::as_bytes), so two spellings of one identifier
    /// produce one transcript. Nothing about the text is wire-visible.
    pub fn parse_hex(text: &str) -> Result<Self, InviteIdError> {
        let bytes = text.as_bytes();
        if bytes.len() != INVITE_ID_BYTES * 2 {
            return Err(InviteIdError::Length {
                expected: INVITE_ID_BYTES,
                got: bytes.len(),
            });
        }
        let mut out = [0u8; INVITE_ID_BYTES];
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            let hi = hex_digit(pair[0]).ok_or(InviteIdError::NotHex)?;
            let lo = hex_digit(pair[1]).ok_or(InviteIdError::NotHex)?;
            out[index] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

impl PartialEq for InviteId {
    fn eq(&self, other: &Self) -> bool {
        ct_eq(&self.0, &other.0)
    }
}

impl Hash for InviteId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for InviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InviteId({})", self.to_hex())
    }
}

impl fmt::Display for InviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl From<[u8; INVITE_ID_BYTES]> for InviteId {
    fn from(bytes: [u8; INVITE_ID_BYTES]) -> Self {
        Self(bytes)
    }
}

/// The wire version [`InviteV1::encode`] opens every invite with.
pub const INVITE_V1_VERSION: u16 = 1;

/// Ceiling on the *encoded* invite, in bytes.
///
/// This bounds the whole wire form [`InviteV1::encode`] produces, not any
/// one field — [`InviteV1::decode`] rejects a slice longer than this before
/// it reads even the version, and [`InviteV1::new`] refuses to construct an
/// invite whose own encoding would break it. In practice the four bounded
/// string fields (each capped at [`MAX_TRANSCRIPT_FIELD_BYTES`]) keep every
/// legal invite well under this ceiling; it exists as the hard, checked
/// number a decoder trusts before allocating or parsing anything, not as a
/// limit invites are expected to approach.
pub const MAX_INVITE_BYTES: usize = 2048;

/// The URL-fragment tag [`InviteV1::to_fragment`] prepends and
/// [`InviteV1::parse_fragment`] requires.
pub const INVITE_FRAGMENT_PREFIX: &str = "mwi1.";

/// Domain separator for [`InviteV1::signing_bytes`].
pub const INVITE_DESCRIPTOR_DOMAIN: &str = "mere.webrtc-carrier/invite-descriptor/v1";

/// Domain separator for [`challenge_signature_bytes`] — what the host signs.
pub const HOST_CHALLENGE_SIGNATURE_DOMAIN: &str = "mere.webrtc-carrier/host-challenge-signature/v1";

/// Domain separator for [`redemption_signing_bytes`] — what the browser's
/// redemption key signs.
pub const REDEMPTION_PROOF_DOMAIN: &str = "mere.webrtc-carrier/redemption-proof/v1";

/// Ceiling on the base64url text [`InviteV1::parse_fragment`] will decode.
///
/// Derived from [`MAX_INVITE_BYTES`]: `ceil(4n/3)`, computed as
/// `(4n + 2) / 3` under integer division — the exact length an unpadded
/// base64url encoding of `n` bytes produces, for every remainder `n` can
/// leave mod 3. That makes this the largest body [`InviteV1::to_fragment`]
/// could ever produce, not merely a round number above it. Checked before
/// any base64 decoding happens, so a fragment whose body is already too
/// long to be a real invite is rejected on a string length, not on the
/// bytes it would have produced.
const MAX_FRAGMENT_BODY_BYTES: usize = (MAX_INVITE_BYTES * 4 + 2) / 3;

/// The C2 invitation: everything a browser needs to redeem a capability into
/// a narrow, expiring delegation, and nothing an application ever executes on
/// trust alone.
///
/// Every field here is an input the inviter chose, in the same spirit as
/// [`LinkChallenge`]: this crate computes over the invite, it does not mint
/// the redemption seed, pick the expiry, or generate the release reference.
/// [`InviteV1::new`] only checks shape — nonempty, bounded strings, and an
/// encoding that fits [`MAX_INVITE_BYTES`] — never freshness or authority.
///
/// [`redemption_seed`](InviteV1::redemption_seed) is the one field this type
/// treats as a secret rather than a plain fact: it is what
/// [`redemption_signing_bytes`] proves possession of, it must never reach a
/// log, and this type's `Debug` implementation is written by hand so that
/// deriving it later cannot quietly put it back.
pub struct InviteV1 {
    rendezvous: InviteId,
    redemption_seed: [u8; 32],
    expected_host_key: [u8; 32],
    network: [u8; 32],
    profile_id: String,
    profile_revision: u64,
    domain: String,
    path: String,
    action: String,
    expires_at_ms: u64,
    max_uses: u32,
    release: ReleaseRefV1,
}

impl InviteV1 {
    /// Assembles an invite, checking the bounded string fields and that the
    /// result encodes within [`MAX_INVITE_BYTES`].
    ///
    /// `profile_id`, `domain`, `path`, and `action` must each be nonempty
    /// and no longer than [`MAX_TRANSCRIPT_FIELD_BYTES`] — the same ceiling
    /// [`LinkChallenge::new`] holds its own string fields to, reused rather
    /// than duplicated. Every other field is accepted as given: this
    /// constructor validates shape, not meaning — an already-expired
    /// `expires_at_ms` or a `max_uses` of zero is a policy question for the
    /// host that mints the invite, not a shape this type refuses.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rendezvous: InviteId,
        redemption_seed: [u8; 32],
        expected_host_key: [u8; 32],
        network: [u8; 32],
        profile_id: impl Into<String>,
        profile_revision: u64,
        domain: impl Into<String>,
        path: impl Into<String>,
        action: impl Into<String>,
        expires_at_ms: u64,
        max_uses: u32,
        release: ReleaseRefV1,
    ) -> Result<Self, InviteError> {
        let profile_id = profile_id.into();
        let domain = domain.into();
        let path = path.into();
        let action = action.into();
        check_field("profile_id", &profile_id)?;
        check_field("domain", &domain)?;
        check_field("path", &path)?;
        check_field("action", &action)?;

        let invite = Self {
            rendezvous,
            redemption_seed,
            expected_host_key,
            network,
            profile_id,
            profile_revision,
            domain,
            path,
            action,
            expires_at_ms,
            max_uses,
            release,
        };

        let encoded_len = invite.encode().len();
        if encoded_len > MAX_INVITE_BYTES {
            return Err(InviteError::Oversize {
                got: encoded_len,
                max: MAX_INVITE_BYTES,
            });
        }

        Ok(invite)
    }

    /// The public rendezvous identifier this invitation belongs to.
    pub const fn rendezvous(&self) -> InviteId {
        self.rendezvous
    }

    /// The redemption secret. Possession of this, proved against a
    /// [`LinkChallenge`] via [`redemption_signing_bytes`], is what lets a
    /// browser turn this invite into a delegation. It must never be logged;
    /// see this type's hand-written `Debug` implementation below.
    pub const fn redemption_seed(&self) -> &[u8; 32] {
        &self.redemption_seed
    }

    /// The native host public key the browser expects to see sign the link
    /// challenge.
    pub const fn expected_host_key(&self) -> &[u8; 32] {
        &self.expected_host_key
    }

    /// The Notochord network this invitation admits into.
    pub const fn network(&self) -> &[u8; 32] {
        &self.network
    }

    /// The Personae profile this invitation redeems against.
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// The profile revision this invitation was minted against.
    pub const fn profile_revision(&self) -> u64 {
        self.profile_revision
    }

    /// The rendezvous domain, e.g. `mer3ly.net`.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The join path under `domain`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The one Graphshell service action this invitation permits.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// When this invitation stops being redeemable, in Unix milliseconds.
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    /// The configurable ceiling on how many sessions this invitation may
    /// redeem.
    pub const fn max_uses(&self) -> u32 {
        self.max_uses
    }

    /// The release this invitation claims to be running.
    ///
    /// See [`ReleaseRefV1`](luggage::ReleaseRefV1): display and Luggage-lookup
    /// only, never an admission input. Luggage owns this type; the carrier
    /// carries it and never interprets it.
    pub const fn release(&self) -> ReleaseRefV1 {
        self.release
    }

    /// The canonical encoding: a `u16`-le version, then every field behind
    /// its own length prefix, in declared order.
    ///
    /// Fixed-width fields are length-prefixed too, for the reason
    /// [`LinkChallenge::encode`] gives its own: with no unprefixed run
    /// anywhere in the transcript, no regrouping of the same bytes across
    /// two fields can produce an identical encoding.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&INVITE_V1_VERSION.to_le_bytes());
        push_field(&mut out, self.rendezvous.as_bytes());
        push_field(&mut out, &self.redemption_seed);
        push_field(&mut out, &self.expected_host_key);
        push_field(&mut out, &self.network);
        push_field(&mut out, self.profile_id.as_bytes());
        push_field(&mut out, &self.profile_revision.to_le_bytes());
        push_field(&mut out, self.domain.as_bytes());
        push_field(&mut out, self.path.as_bytes());
        push_field(&mut out, self.action.as_bytes());
        push_field(&mut out, &self.expires_at_ms.to_le_bytes());
        push_field(&mut out, &self.max_uses.to_le_bytes());
        push_field(&mut out, &self.release.manifest_blake3);
        push_field(&mut out, &self.release.publisher_key_id);
        out
    }

    /// Parses [`encode`](Self::encode)'s output back into an invite.
    ///
    /// Strict, in the order that matters: `bytes.len()` is checked against
    /// [`MAX_INVITE_BYTES`] before a single field is read, so an oversize
    /// slice is refused on its length alone rather than by allocating or
    /// parsing into it. Then the version must match exactly, every field's
    /// declared length must fit what remains of the buffer (a fixed-width
    /// field whose declared length is not exactly its width is refused, not
    /// truncated or padded), and no bytes may follow the last field. The
    /// parsed fields are handed to [`Self::new`], so a decoded invite obeys
    /// exactly the bounds a freshly constructed one does.
    pub fn decode(bytes: &[u8]) -> Result<Self, InviteError> {
        if bytes.len() > MAX_INVITE_BYTES {
            return Err(InviteError::Oversize {
                got: bytes.len(),
                max: MAX_INVITE_BYTES,
            });
        }

        let mut pos = 0usize;
        let version = read_u16_le(bytes, &mut pos)?;
        if version != INVITE_V1_VERSION {
            return Err(InviteError::BadVersion { got: version });
        }

        let rendezvous =
            InviteId::from_bytes(read_fixed_field::<INVITE_ID_BYTES>(bytes, &mut pos)?);
        let redemption_seed = read_fixed_field::<32>(bytes, &mut pos)?;
        let expected_host_key = read_fixed_field::<32>(bytes, &mut pos)?;
        let network = read_fixed_field::<32>(bytes, &mut pos)?;
        let profile_id = read_string_field(bytes, &mut pos)?;
        let profile_revision = read_u64_field(bytes, &mut pos)?;
        let domain = read_string_field(bytes, &mut pos)?;
        let path = read_string_field(bytes, &mut pos)?;
        let action = read_string_field(bytes, &mut pos)?;
        let expires_at_ms = read_u64_field(bytes, &mut pos)?;
        let max_uses = read_u32_field(bytes, &mut pos)?;
        let manifest_blake3 = read_fixed_field::<32>(bytes, &mut pos)?;
        let publisher_key_id = read_fixed_field::<32>(bytes, &mut pos)?;

        if pos != bytes.len() {
            return Err(InviteError::Malformed);
        }

        Self::new(
            rendezvous,
            redemption_seed,
            expected_host_key,
            network,
            profile_id,
            profile_revision,
            domain,
            path,
            action,
            expires_at_ms,
            max_uses,
            ReleaseRefV1 {
                manifest_blake3,
                publisher_key_id,
            },
        )
    }

    /// The bytes a host signs (or a browser verifies) to vouch for this
    /// invite descriptor: [`INVITE_DESCRIPTOR_DOMAIN`], then
    /// [`Self::encode`], each length-prefixed.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, INVITE_DESCRIPTOR_DOMAIN.as_bytes());
        push_field(&mut out, &self.encode());
        out
    }

    /// Renders this invite as a URL fragment: [`INVITE_FRAGMENT_PREFIX`]
    /// followed by unpadded base64url of [`Self::encode`].
    ///
    /// The caller is what puts this behind a `#` in an actual URL; this type
    /// has no notion of a URL beyond the fragment text itself.
    pub fn to_fragment(&self) -> String {
        format!("{INVITE_FRAGMENT_PREFIX}{}", b64url_encode(&self.encode()))
    }

    /// Parses [`Self::to_fragment`]'s output, with or without a leading `#`.
    ///
    /// Order matters here too: the prefix is checked first, then the
    /// base64url body's *text* length is capped before any decoding is
    /// attempted, then the body is decoded with a strict alphabet (no
    /// padding, no characters outside the 64-symbol set), and only then is
    /// [`Self::decode`] asked to parse the resulting bytes.
    pub fn parse_fragment(fragment: &str) -> Result<Self, InviteError> {
        let text = fragment.strip_prefix('#').unwrap_or(fragment);
        let body = text
            .strip_prefix(INVITE_FRAGMENT_PREFIX)
            .ok_or(InviteError::BadFragment)?;

        if body.len() > MAX_FRAGMENT_BODY_BYTES {
            return Err(InviteError::Oversize {
                got: body.len(),
                max: MAX_FRAGMENT_BODY_BYTES,
            });
        }

        let bytes = b64url_decode(body).ok_or(InviteError::BadFragment)?;
        Self::decode(&bytes)
    }
}

impl fmt::Debug for InviteV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Hand-written, not derived, and that is the whole point: a derived
        // `Debug` would print `redemption_seed` as a plain byte array, and
        // this is the one field the fragment-hygiene done-condition says
        // must never reach a log. Every other field renders normally.
        f.debug_struct("InviteV1")
            .field("rendezvous", &self.rendezvous)
            .field("redemption_seed", &"[redacted; 32 bytes]")
            .field("expected_host_key", &self.expected_host_key)
            .field("network", &self.network)
            .field("profile_id", &self.profile_id)
            .field("profile_revision", &self.profile_revision)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("action", &self.action)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("max_uses", &self.max_uses)
            .field("release", &self.release)
            .finish()
    }
}

fn check_field(name: &'static str, value: &str) -> Result<(), InviteError> {
    if value.is_empty() {
        return Err(InviteError::FieldEmpty { field: name });
    }
    if value.len() > MAX_TRANSCRIPT_FIELD_BYTES {
        return Err(InviteError::FieldTooLong {
            field: name,
            got: value.len(),
            max: MAX_TRANSCRIPT_FIELD_BYTES,
        });
    }
    Ok(())
}

/// Reads the raw two-byte little-endian version prefix. Not length-prefixed
/// like the fields after it — it is what says how to interpret them.
fn read_u16_le(bytes: &[u8], pos: &mut usize) -> Result<u16, InviteError> {
    let end = pos.checked_add(2).ok_or(InviteError::Malformed)?;
    let slice = bytes.get(*pos..end).ok_or(InviteError::Malformed)?;
    let value = u16::from_le_bytes([slice[0], slice[1]]);
    *pos = end;
    Ok(value)
}

/// Reads one `push_field`-encoded field: an 8-byte little-endian length,
/// then that many bytes. `None` from `bytes.get` (a length prefix that runs
/// past the buffer, or a declared length longer than what remains) becomes
/// `Malformed` rather than a panic or a short read.
fn read_field<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], InviteError> {
    let len_end = pos.checked_add(8).ok_or(InviteError::Malformed)?;
    let len_bytes = bytes.get(*pos..len_end).ok_or(InviteError::Malformed)?;
    let mut len_array = [0u8; 8];
    len_array.copy_from_slice(len_bytes);
    let len = usize::try_from(u64::from_le_bytes(len_array)).map_err(|_| InviteError::Malformed)?;
    let field_end = len_end.checked_add(len).ok_or(InviteError::Malformed)?;
    let field = bytes
        .get(len_end..field_end)
        .ok_or(InviteError::Malformed)?;
    *pos = field_end;
    Ok(field)
}

/// Reads a length-prefixed field whose declared length must be exactly `N`.
/// A fixed-width field is not truncated or padded to fit — a length prefix
/// that disagrees with `N` is `Malformed`.
fn read_fixed_field<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], InviteError> {
    let field = read_field(bytes, pos)?;
    <[u8; N]>::try_from(field).map_err(|_| InviteError::Malformed)
}

fn read_u64_field(bytes: &[u8], pos: &mut usize) -> Result<u64, InviteError> {
    Ok(u64::from_le_bytes(read_fixed_field::<8>(bytes, pos)?))
}

fn read_u32_field(bytes: &[u8], pos: &mut usize) -> Result<u32, InviteError> {
    Ok(u32::from_le_bytes(read_fixed_field::<4>(bytes, pos)?))
}

fn read_string_field(bytes: &[u8], pos: &mut usize) -> Result<String, InviteError> {
    let field = read_field(bytes, pos)?;
    String::from_utf8(field.to_vec()).map_err(|_| InviteError::Malformed)
}

/// The bytes the HOST signs to vouch for one [`LinkChallenge`]:
/// [`HOST_CHALLENGE_SIGNATURE_DOMAIN`], then the challenge's own
/// [`encode`](LinkChallenge::encode), each length-prefixed.
///
/// Distinct from the bare transcript [`LinkChallenge::shared_link`] hashes
/// and from [`redemption_signing_bytes`] below: three different domain
/// strings wrap overlapping bytes, so a signature meant for one can never be
/// replayed as a signature — or a hash — meant for another.
pub fn challenge_signature_bytes(challenge: &LinkChallenge) -> Vec<u8> {
    let mut out = Vec::new();
    push_field(&mut out, HOST_CHALLENGE_SIGNATURE_DOMAIN.as_bytes());
    push_field(&mut out, &challenge.encode());
    out
}

/// The bytes the BROWSER's ephemeral redemption key signs to prove
/// possession of the invite's redemption seed, bound to both the link
/// challenge and the subject the delegation will name:
/// [`REDEMPTION_PROOF_DOMAIN`], then the challenge's
/// [`encode`](LinkChallenge::encode), then `subject`, each length-prefixed.
///
/// Binding `subject` is what stops a captured redemption proof from being
/// replayed for a different ephemeral Personae subject; binding the
/// challenge is what stops it from being replayed on a second WebRTC
/// connection. The host never sees the private key that signs this — only
/// these bytes and the signature over them.
pub fn redemption_signing_bytes(challenge: &LinkChallenge, subject: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::new();
    push_field(&mut out, REDEMPTION_PROOF_DOMAIN.as_bytes());
    push_field(&mut out, &challenge.encode());
    push_field(&mut out, subject);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_in_either_case() {
        let id = InviteId::from_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ]);
        assert_eq!(id.to_hex(), "000102030405060708090a0b0c0d0e0f");
        assert_eq!(InviteId::parse_hex(&id.to_hex()).expect("parses"), id);
        assert_eq!(
            InviteId::parse_hex("000102030405060708090A0B0C0D0E0F").expect("parses"),
            id
        );
    }

    #[test]
    fn short_and_non_hex_text_is_refused() {
        assert_eq!(
            InviteId::parse_hex("00"),
            Err(InviteIdError::Length {
                expected: INVITE_ID_BYTES,
                got: 2
            })
        );
        assert_eq!(
            InviteId::parse_hex("000102030405060708090a0b0c0d0e0g"),
            Err(InviteIdError::NotHex)
        );
    }

    #[test]
    fn equality_holds_over_the_whole_width() {
        let a = InviteId::from_bytes([7; INVITE_ID_BYTES]);
        let mut tail = [7u8; INVITE_ID_BYTES];
        tail[INVITE_ID_BYTES - 1] = 8;
        assert_ne!(a, InviteId::from_bytes(tail));
        assert_eq!(a, InviteId::from_bytes([7; INVITE_ID_BYTES]));
    }

    use crate::challenge::NONCE_BYTES;
    use crate::fingerprint::{DTLS_FINGERPRINT_BYTES, DtlsFingerprint, FingerprintRole};

    /// Every `InviteV1` constructor argument, gathered so a single-field
    /// variant can be built by cloning this and mutating one line — the same
    /// shape `tests/vectors.rs` uses for `LinkChallenge`.
    #[derive(Clone)]
    struct SampleParams {
        rendezvous: InviteId,
        redemption_seed: [u8; 32],
        expected_host_key: [u8; 32],
        network: [u8; 32],
        profile_id: String,
        profile_revision: u64,
        domain: String,
        path: String,
        action: String,
        expires_at_ms: u64,
        max_uses: u32,
        release: ReleaseRefV1,
    }

    impl SampleParams {
        fn baseline() -> Self {
            Self {
                rendezvous: InviteId::from_bytes([1; INVITE_ID_BYTES]),
                redemption_seed: [2; 32],
                expected_host_key: [3; 32],
                network: [4; 32],
                profile_id: "profile-1".to_owned(),
                profile_revision: 7,
                domain: "mer3ly.net".to_owned(),
                path: "/join/abc123".to_owned(),
                action: "graphshell.read".to_owned(),
                expires_at_ms: 1_800_000_000_000,
                max_uses: 3,
                release: ReleaseRefV1 {
                    manifest_blake3: [5; 32],
                    publisher_key_id: [6; 32],
                },
            }
        }

        fn build(self) -> InviteV1 {
            self.build_result().expect("the sample params are valid")
        }

        fn build_result(self) -> Result<InviteV1, InviteError> {
            InviteV1::new(
                self.rendezvous,
                self.redemption_seed,
                self.expected_host_key,
                self.network,
                self.profile_id,
                self.profile_revision,
                self.domain,
                self.path,
                self.action,
                self.expires_at_ms,
                self.max_uses,
                self.release,
            )
        }
    }

    fn sample_invite() -> InviteV1 {
        SampleParams::baseline().build()
    }

    fn sample_challenge() -> LinkChallenge {
        LinkChallenge::new(
            b"mere/graphshell/v1".to_vec(),
            b"mere-graphshell".to_vec(),
            InviteId::from_bytes([9; INVITE_ID_BYTES]),
            [0x11; NONCE_BYTES],
            [0x22; NONCE_BYTES],
            DtlsFingerprint::new(FingerprintRole::Client, [0xaa; DTLS_FINGERPRINT_BYTES]),
            DtlsFingerprint::new(FingerprintRole::Server, [0xbb; DTLS_FINGERPRINT_BYTES]),
        )
        .expect("valid")
    }

    #[test]
    fn encode_decode_round_trips_every_field() {
        let invite = sample_invite();
        let encoded = invite.encode();
        let decoded = InviteV1::decode(&encoded).expect("decodes");

        assert_eq!(decoded.rendezvous(), invite.rendezvous());
        assert_eq!(decoded.redemption_seed(), invite.redemption_seed());
        assert_eq!(decoded.expected_host_key(), invite.expected_host_key());
        assert_eq!(decoded.network(), invite.network());
        assert_eq!(decoded.profile_id(), invite.profile_id());
        assert_eq!(decoded.profile_revision(), invite.profile_revision());
        assert_eq!(decoded.domain(), invite.domain());
        assert_eq!(decoded.path(), invite.path());
        assert_eq!(decoded.action(), invite.action());
        assert_eq!(decoded.expires_at_ms(), invite.expires_at_ms());
        assert_eq!(decoded.max_uses(), invite.max_uses());
        assert_eq!(decoded.release(), invite.release());
        assert_eq!(decoded.encode(), encoded);
    }

    #[test]
    fn fragment_round_trips_with_and_without_a_leading_hash() {
        let invite = sample_invite();
        let fragment = invite.to_fragment();
        assert!(fragment.starts_with(INVITE_FRAGMENT_PREFIX));

        let parsed = InviteV1::parse_fragment(&fragment).expect("parses without '#'");
        assert_eq!(parsed.encode(), invite.encode());

        let with_hash = format!("#{fragment}");
        let parsed_hash = InviteV1::parse_fragment(&with_hash).expect("parses with '#'");
        assert_eq!(parsed_hash.encode(), invite.encode());
    }

    #[test]
    fn fragment_rejects_the_wrong_prefix() {
        let fragment = sample_invite().to_fragment();
        let body = fragment.strip_prefix(INVITE_FRAGMENT_PREFIX).unwrap();
        let wrong_prefix = format!("mwi2.{body}");
        assert_eq!(
            InviteV1::parse_fragment(&wrong_prefix).unwrap_err(),
            InviteError::BadFragment
        );
        assert_eq!(
            InviteV1::parse_fragment("not a fragment at all").unwrap_err(),
            InviteError::BadFragment
        );
    }

    #[test]
    fn fragment_rejects_a_padding_character() {
        let fragment = sample_invite().to_fragment();
        let mut body = fragment
            .strip_prefix(INVITE_FRAGMENT_PREFIX)
            .unwrap()
            .to_owned();
        // Same length as a real body, but the last character is padding
        // rather than a length%4==1 shortcut catching it for the wrong
        // reason.
        body.pop();
        body.push('=');
        let padded = format!("{INVITE_FRAGMENT_PREFIX}{body}");
        assert_eq!(
            InviteV1::parse_fragment(&padded).unwrap_err(),
            InviteError::BadFragment
        );
    }

    #[test]
    fn fragment_rejects_a_character_outside_the_alphabet() {
        let fragment = sample_invite().to_fragment();
        let mut body = fragment
            .strip_prefix(INVITE_FRAGMENT_PREFIX)
            .unwrap()
            .to_owned();
        body.pop();
        body.push('!');
        let bad = format!("{INVITE_FRAGMENT_PREFIX}{body}");
        assert_eq!(
            InviteV1::parse_fragment(&bad).unwrap_err(),
            InviteError::BadFragment
        );
    }

    #[test]
    fn fragment_rejects_an_oversize_body_before_decoding() {
        // Well past MAX_FRAGMENT_BODY_BYTES; if the cap ran after decoding
        // instead of before, `got` below would be a decoded byte count, not
        // this string's own character count.
        let huge_body = "A".repeat(MAX_INVITE_BYTES * 2);
        let fragment = format!("{INVITE_FRAGMENT_PREFIX}{huge_body}");
        match InviteV1::parse_fragment(&fragment) {
            Err(InviteError::Oversize { got, max }) => {
                assert_eq!(got, huge_body.len());
                assert_eq!(max, MAX_FRAGMENT_BODY_BYTES);
            },
            other => panic!("expected Oversize, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_oversize_input_before_parsing() {
        // All zero bytes: if the oversize check ran after even the version
        // were read, this would fail as `BadVersion { got: 0 }` instead,
        // which is exactly how this test proves the ordering.
        let bytes = vec![0u8; MAX_INVITE_BYTES + 1];
        assert_eq!(
            InviteV1::decode(&bytes).unwrap_err(),
            InviteError::Oversize {
                got: bytes.len(),
                max: MAX_INVITE_BYTES,
            }
        );
    }

    #[test]
    fn decode_rejects_the_wrong_version() {
        let mut encoded = sample_invite().encode();
        encoded[0] = 0xff;
        encoded[1] = 0xff;
        assert_eq!(
            InviteV1::decode(&encoded).unwrap_err(),
            InviteError::BadVersion { got: 0xffff }
        );
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let mut encoded = sample_invite().encode();
        encoded.push(0);
        assert_eq!(
            InviteV1::decode(&encoded).unwrap_err(),
            InviteError::Malformed
        );
    }

    #[test]
    fn decode_rejects_truncation_at_every_field_boundary() {
        let encoded = sample_invite().encode();
        for len in 0..encoded.len() {
            assert!(
                InviteV1::decode(&encoded[..len]).is_err(),
                "prefix of length {len} unexpectedly decoded"
            );
        }
        assert!(InviteV1::decode(&encoded).is_ok());
    }

    #[test]
    fn changing_any_single_field_changes_signing_bytes() {
        let baseline = sample_invite().signing_bytes();

        let base = SampleParams::baseline();
        let mut rendezvous = base.clone();
        rendezvous.rendezvous = InviteId::from_bytes([0xaa; INVITE_ID_BYTES]);
        let mut redemption_seed = base.clone();
        redemption_seed.redemption_seed[0] ^= 0x01;
        let mut expected_host_key = base.clone();
        expected_host_key.expected_host_key[0] ^= 0x01;
        let mut network = base.clone();
        network.network[0] ^= 0x01;
        let mut profile_id = base.clone();
        profile_id.profile_id = "profile-2".to_owned();
        let mut profile_revision = base.clone();
        profile_revision.profile_revision += 1;
        let mut domain = base.clone();
        domain.domain = "example.mer3ly.net".to_owned();
        let mut path = base.clone();
        path.path = "/join/different".to_owned();
        let mut action = base.clone();
        action.action = "graphshell.write".to_owned();
        let mut expires_at_ms = base.clone();
        expires_at_ms.expires_at_ms += 1;
        let mut max_uses = base.clone();
        max_uses.max_uses += 1;
        let mut manifest_blake3 = base.clone();
        manifest_blake3.release.manifest_blake3[0] ^= 0x01;
        let mut publisher_key_id = base.clone();
        publisher_key_id.release.publisher_key_id[0] ^= 0x01;

        let variants = [
            ("rendezvous", rendezvous),
            ("redemption_seed", redemption_seed),
            ("expected_host_key", expected_host_key),
            ("network", network),
            ("profile_id", profile_id),
            ("profile_revision", profile_revision),
            ("domain", domain),
            ("path", path),
            ("action", action),
            ("expires_at_ms", expires_at_ms),
            ("max_uses", max_uses),
            ("manifest_blake3", manifest_blake3),
            ("publisher_key_id", publisher_key_id),
        ];

        let variant_count = variants.len();
        let mut seen = vec![baseline.clone()];
        for (field, params) in variants {
            let signing = params.build().signing_bytes();
            assert_ne!(
                signing, baseline,
                "changing `{field}` left signing_bytes unchanged"
            );
            assert!(
                !seen.contains(&signing),
                "`{field}` collided with an earlier variant"
            );
            seen.push(signing);
        }
        assert_eq!(seen.len(), variant_count + 1);
    }

    #[test]
    fn challenge_and_redemption_signing_bytes_are_domain_separated() {
        let challenge = sample_challenge();
        let subject = [7u8; 32];
        let other_subject = [8u8; 32];

        let transcript = challenge.encode();
        let sig_bytes = challenge_signature_bytes(&challenge);
        let redemption_bytes = redemption_signing_bytes(&challenge, &subject);
        let redemption_other = redemption_signing_bytes(&challenge, &other_subject);

        assert_ne!(sig_bytes, transcript);
        assert_ne!(redemption_bytes, transcript);
        assert_ne!(sig_bytes, redemption_bytes);
        assert_ne!(redemption_bytes, redemption_other);
    }

    #[test]
    fn debug_redacts_the_redemption_seed_but_not_sibling_fields() {
        let seed = [0x42u8; 32];
        let host_key = [3u8; 32];
        let mut params = SampleParams::baseline();
        params.redemption_seed = seed;
        params.expected_host_key = host_key;
        let invite = params.build();

        let rendered = format!("{invite:?}");
        assert!(rendered.contains("[redacted; 32 bytes]"));
        assert!(!rendered.contains(&crate::codec::to_hex_lower(&seed)));
        // Not just the hex form: the raw derive-style array rendering must
        // not leak either.
        assert!(!rendered.contains(&format!("{seed:?}")));
        // A sibling 32-byte field DOES render normally, proving the
        // omission is deliberate rather than every field being suppressed.
        assert!(rendered.contains(&format!("{host_key:?}")));
    }

    #[test]
    fn constructor_rejects_empty_and_oversize_string_fields() {
        let mut empty = SampleParams::baseline();
        empty.action = String::new();
        assert_eq!(
            empty.build_result().unwrap_err(),
            InviteError::FieldEmpty { field: "action" }
        );

        let mut long = SampleParams::baseline();
        long.domain = "x".repeat(MAX_TRANSCRIPT_FIELD_BYTES + 1);
        let got = long.domain.len();
        assert_eq!(
            long.build_result().unwrap_err(),
            InviteError::FieldTooLong {
                field: "domain",
                got,
                max: MAX_TRANSCRIPT_FIELD_BYTES,
            }
        );
    }
}
