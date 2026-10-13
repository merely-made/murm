// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The carrier core's error types.
//!
//! One enum per concern rather than a single crate-wide error: a caller
//! reading a frame off the data channel and a caller parsing an SDP
//! fingerprint are in different failure worlds, and collapsing them would
//! force every match arm to consider the other's variants.

use thiserror::Error;

/// A framing failure on the data channel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameError {
    /// The declared payload length exceeds
    /// [`MAX_FRAME_PAYLOAD_BYTES`](crate::MAX_FRAME_PAYLOAD_BYTES).
    ///
    /// This is raised from the four-byte length prefix alone, before the
    /// payload is read, allocated, or deserialized. A peer cannot make this
    /// end reserve memory by announcing a frame it never sends.
    #[error("frame declares {declared} payload bytes, over the {max}-byte maximum")]
    Oversize {
        /// The length the peer declared, as read from the prefix.
        declared: u64,
        /// The ceiling it broke.
        max: usize,
    },
    /// Fewer than [`FRAME_HEADER_BYTES`](crate::FRAME_HEADER_BYTES) bytes are
    /// available, so the length prefix is not yet complete.
    #[error("frame header incomplete: {available} of {needed} prefix bytes")]
    ShortHeader {
        /// Bytes needed for the length prefix.
        needed: usize,
        /// Bytes actually available.
        available: usize,
    },
    /// The prefix is complete and within bounds, but the payload has not all
    /// arrived yet. The caller reads more and retries; nothing was allocated.
    #[error("frame incomplete: {available} of {needed} bytes")]
    Incomplete {
        /// Total frame length (prefix included) the payload needs.
        needed: usize,
        /// Bytes actually available.
        available: usize,
    },
}

/// A malformed invitation identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InviteIdError {
    /// The hex text did not decode to exactly
    /// [`INVITE_ID_BYTES`](crate::INVITE_ID_BYTES) bytes.
    #[error("invite id must be {expected} hex-encoded bytes, got {got} characters")]
    Length {
        /// Byte width an invite id has.
        expected: usize,
        /// Characters supplied.
        got: usize,
    },
    /// A character outside `[0-9a-fA-F]` appeared in the hex text.
    #[error("invite id contains a non-hex character")]
    NotHex,
}

/// A malformed DTLS fingerprint.
///
/// Every variant is a hard reject. Nothing here truncates, pads, or coerces:
/// a fingerprint that does not parse exactly is a fingerprint that must not
/// enter a transcript, because the transcript is the only thing standing
/// between the session and a signaling intermediary terminating two DTLS
/// sessions and relaying between them.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FingerprintError {
    /// The SDP attribute named a hash function other than `sha-256`.
    #[error("fingerprint algorithm must be sha-256, got `{got}`")]
    Algorithm {
        /// The algorithm token as it appeared.
        got: String,
    },
    /// The attribute value carried no algorithm token, or no hex after it.
    #[error("fingerprint attribute is not `<algorithm> <hex>`")]
    Attribute,
    /// The colon-separated hex did not carry exactly 32 octet groups.
    #[error("fingerprint must be 32 octets, got {got}")]
    OctetCount {
        /// Groups found between the colons.
        got: usize,
    },
    /// A group was not exactly two uppercase hex digits.
    ///
    /// RFC 8122's grammar is `2UHEX *(":" 2UHEX)` with `UHEX` uppercase, and
    /// browsers emit uppercase. Lowercase is rejected rather than accepted
    /// quietly, so a peer that drifts from the grammar is visible.
    #[error("fingerprint octet {index} is not two uppercase hex digits")]
    Octet {
        /// Zero-based position of the offending group.
        index: usize,
    },
}

/// The link challenge could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChallengeError {
    /// A fingerprint was supplied in the wrong role slot.
    ///
    /// The whole point of the role tag is that the client and server halves
    /// are not interchangeable, so the constructor refuses to build a
    /// transcript that would quietly bind them the wrong way round.
    #[error("expected the {expected} fingerprint in this slot, got the {got} one")]
    RoleMismatch {
        /// The role that slot binds.
        expected: &'static str,
        /// The role the supplied fingerprint carried.
        got: &'static str,
    },
    /// A variable-length transcript field ran past
    /// [`MAX_TRANSCRIPT_FIELD_BYTES`](crate::MAX_TRANSCRIPT_FIELD_BYTES).
    #[error("transcript field `{field}` is {len} bytes, over the {max}-byte maximum")]
    FieldTooLong {
        /// Which field overran.
        field: &'static str,
        /// Its length.
        len: usize,
        /// The ceiling it broke.
        max: usize,
    },
    /// A variable-length transcript field was empty.
    ///
    /// An empty protocol or channel label is never a real one, and admitting
    /// it would put a zero-length field into a transcript whose whole job is
    /// to be unambiguous.
    #[error("transcript field `{field}` is empty")]
    FieldEmpty {
        /// Which field was empty.
        field: &'static str,
    },
}

/// An [`InviteV1`](crate::InviteV1) could not be built, encoded, decoded, or
/// read from a URL fragment.
///
/// Every variant is a hard reject, in keeping with the rest of this crate:
/// `InviteV1` carries a redemption secret, and the fragment-hygiene
/// done-condition in the browser WebRTC carrier plan's C2 phase means a
/// malformed or oversize invite must fail closed before it is logged,
/// allocated further, or treated as a value with meaning.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InviteError {
    /// The input exceeds [`MAX_INVITE_BYTES`](crate::MAX_INVITE_BYTES).
    ///
    /// Raised from a length alone — [`InviteV1::decode`](crate::InviteV1::decode)
    /// checks the byte slice's length before parsing a single field, and
    /// [`InviteV1::parse_fragment`](crate::InviteV1::parse_fragment) checks
    /// the base64url text's length before decoding it — so `got` is either a
    /// byte count or a character count depending on which caller raised it,
    /// never a size computed by allocating past the ceiling first.
    #[error("invite is {got} bytes, over the {max}-byte maximum")]
    Oversize {
        /// The size actually seen.
        got: usize,
        /// The ceiling it broke.
        max: usize,
    },
    /// The text was not a well-formed invite fragment: it lacked
    /// [`INVITE_FRAGMENT_PREFIX`](crate::INVITE_FRAGMENT_PREFIX) after an
    /// optional leading `#`, or its base64url body did not decode.
    #[error("text is not a well-formed invite fragment")]
    BadFragment,
    /// The two-byte version prefix did not match
    /// [`INVITE_V1_VERSION`](crate::INVITE_V1_VERSION).
    #[error("invite version {got} is not supported")]
    BadVersion {
        /// The version tag actually read.
        got: u16,
    },
    /// A variable-length field was empty.
    ///
    /// An empty profile id, domain, path, or action is never a real one, the
    /// same rule [`ChallengeError::FieldEmpty`] enforces for the link
    /// transcript.
    #[error("invite field `{field}` is empty")]
    FieldEmpty {
        /// Which field was empty.
        field: &'static str,
    },
    /// A variable-length field ran past
    /// [`MAX_TRANSCRIPT_FIELD_BYTES`](crate::MAX_TRANSCRIPT_FIELD_BYTES).
    #[error("invite field `{field}` is {got} bytes, over the {max}-byte maximum")]
    FieldTooLong {
        /// Which field overran.
        field: &'static str,
        /// Its length.
        got: usize,
        /// The ceiling it broke.
        max: usize,
    },
    /// The bytes do not parse as an encoded `InviteV1`.
    ///
    /// Covers every shape failure that is not a version or field-bound
    /// mismatch: a length prefix that runs past the end of the buffer, a
    /// fixed-width field whose declared length does not match its width,
    /// bytes left over after the last field, or a string field that is not
    /// valid UTF-8.
    #[error("invite bytes are malformed")]
    Malformed,
}

/// Why a [`crate::Backpressure`] policy was refused.
///
/// Every arm is a shape that would misbehave at runtime rather than merely
/// being unusual, which is why they are refused at construction instead of
/// clamped silently.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackpressureError {
    /// The high-water mark exceeds the configuration ceiling.
    #[error("high-water mark {requested} exceeds the {max}-byte ceiling")]
    HighWaterTooLarge {
        /// What was asked for.
        requested: usize,
        /// The ceiling.
        max: usize,
    },
    /// A zero high-water mark would pause a sender that has written nothing.
    #[error("high-water mark must be above zero")]
    HighWaterZero,
    /// The marks are equal or inverted, which oscillates instead of damping.
    #[error("low-water mark {low} must be strictly below high-water {high}")]
    MarksNotSeparated {
        /// The high mark.
        high: usize,
        /// The low mark.
        low: usize,
    },
}
