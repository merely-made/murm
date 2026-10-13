// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Bounded, length-prefixed carrier frames.
//!
//! A WebRTC data channel is message-oriented, but a browser and a native
//! stack disagree about how large a message may be and where an
//! implementation is free to fragment. The carrier therefore carries its own
//! framing: a four-byte big-endian payload length followed by that many
//! bytes, with one explicit ceiling both ends know.
//!
//! ## The ordering that matters
//!
//! [`decode_frame`] and [`FrameHeader::decode`] reject an oversize frame from
//! the **length prefix alone**. No buffer is reserved, no payload is copied,
//! and nothing is deserialized before the declared length has been compared
//! against [`MAX_FRAME_PAYLOAD_BYTES`]. That ordering is the point of the
//! module, not an implementation detail: a peer that announces a four-gigabyte
//! frame gets an error, not an allocation.
//!
//! Backpressure — high- and low-water marks over `bufferedAmount` — belongs to
//! the runtime adapters in C1, not here. This module bounds one frame.

use crate::error::FrameError;

/// Width of the length prefix that opens every frame.
pub const FRAME_HEADER_BYTES: usize = 4;

/// The largest payload a single frame may carry: 64 KiB minus the header.
///
/// Chosen so the **whole frame**, header included, fits inside a 64 KiB SCTP
/// message — not just the payload. 64 KiB (65,536 bytes) is str0m's own
/// `DEFAULT_REMOTE_MAX_MESSAGE_SIZE` (`sctp/mod.rs:36`) and is SCTP's default
/// `max-message-size` for a peer that never negotiates otherwise; browsers are
/// not the constraint here; they advertise 262,144 (256 KiB) in SDP, confirmed
/// against a live Chrome offer. A `MAX_FRAME_PAYLOAD_BYTES` of a full 65,536
/// therefore overshoots a default-SCTP peer by exactly
/// [`FRAME_HEADER_BYTES`] and only survived because every browser tested
/// against advertises the larger limit. Anything past this ceiling is the
/// application's job to chunk.
pub const MAX_FRAME_PAYLOAD_BYTES: usize = 65_536 - FRAME_HEADER_BYTES;

/// The largest complete frame, prefix included: exactly 64 KiB (65,536
/// bytes), by construction from [`MAX_FRAME_PAYLOAD_BYTES`] above.
pub const MAX_FRAME_BYTES: usize = FRAME_HEADER_BYTES + MAX_FRAME_PAYLOAD_BYTES;

/// The length prefix of a frame, already checked against the ceiling.
///
/// Holding one of these is the proof that a declared length is admissible:
/// it cannot be constructed from a prefix that overran
/// [`MAX_FRAME_PAYLOAD_BYTES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameHeader {
    payload_len: u32,
}

impl FrameHeader {
    /// Builds a header for a payload of `payload_len` bytes.
    pub fn new(payload_len: usize) -> Result<Self, FrameError> {
        if payload_len > MAX_FRAME_PAYLOAD_BYTES {
            return Err(FrameError::Oversize {
                declared: payload_len as u64,
                max: MAX_FRAME_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            payload_len: payload_len as u32,
        })
    }

    /// Reads a header off the front of `bytes`, checking the ceiling.
    ///
    /// `bytes` needs only the [`FRAME_HEADER_BYTES`] prefix; the payload need
    /// not have arrived. That is deliberate — the ceiling check must be
    /// answerable before a caller decides how much to buffer.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < FRAME_HEADER_BYTES {
            return Err(FrameError::ShortHeader {
                needed: FRAME_HEADER_BYTES,
                available: bytes.len(),
            });
        }
        let mut prefix = [0u8; FRAME_HEADER_BYTES];
        prefix.copy_from_slice(&bytes[..FRAME_HEADER_BYTES]);
        let declared = u32::from_be_bytes(prefix);
        if declared as usize > MAX_FRAME_PAYLOAD_BYTES {
            return Err(FrameError::Oversize {
                declared: u64::from(declared),
                max: MAX_FRAME_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            payload_len: declared,
        })
    }

    /// The declared payload length, guaranteed `<= MAX_FRAME_PAYLOAD_BYTES`.
    pub const fn payload_len(&self) -> usize {
        self.payload_len as usize
    }

    /// The whole frame's length, prefix included.
    pub const fn frame_len(&self) -> usize {
        FRAME_HEADER_BYTES + self.payload_len as usize
    }

    /// The wire form of the prefix.
    pub const fn encode(&self) -> [u8; FRAME_HEADER_BYTES] {
        self.payload_len.to_be_bytes()
    }
}

/// Frames `payload` into a fresh buffer.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    let header = FrameHeader::new(payload.len())?;
    let mut out = Vec::with_capacity(header.frame_len());
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Frames `payload` onto the end of `out`, returning the bytes appended.
///
/// The send path's allocation-free form: one outbound buffer, reused.
pub fn encode_frame_into(payload: &[u8], out: &mut Vec<u8>) -> Result<usize, FrameError> {
    let header = FrameHeader::new(payload.len())?;
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(payload);
    Ok(header.frame_len())
}

/// Borrows one frame's payload from the front of `buf`.
///
/// Returns the payload and the number of bytes consumed, so a caller draining
/// a receive buffer can advance by the second element. The payload is
/// borrowed, never copied: the decoder allocates nothing at all, which is
/// what makes "rejected before allocation" a property of the code rather than
/// a claim about it.
pub fn decode_frame(buf: &[u8]) -> Result<(&[u8], usize), FrameError> {
    let header = FrameHeader::decode(buf)?;
    let end = header.frame_len();
    if buf.len() < end {
        return Err(FrameError::Incomplete {
            needed: end,
            available: buf.len(),
        });
    }
    Ok((&buf[FRAME_HEADER_BYTES..end], end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_payload() {
        let framed = encode_frame(b"ping").expect("within bounds");
        assert_eq!(framed.len(), FRAME_HEADER_BYTES + 4);
        let (payload, consumed) = decode_frame(&framed).expect("decodes");
        assert_eq!(payload, b"ping");
        assert_eq!(consumed, framed.len());
    }

    #[test]
    fn round_trips_an_empty_payload() {
        let framed = encode_frame(b"").expect("within bounds");
        let (payload, consumed) = decode_frame(&framed).expect("decodes");
        assert!(payload.is_empty());
        assert_eq!(consumed, FRAME_HEADER_BYTES);
    }

    #[test]
    fn decodes_the_first_of_several_frames() {
        let mut buf = Vec::new();
        encode_frame_into(b"one", &mut buf).expect("within bounds");
        encode_frame_into(b"two", &mut buf).expect("within bounds");
        let (first, consumed) = decode_frame(&buf).expect("decodes");
        assert_eq!(first, b"one");
        let (second, _) = decode_frame(&buf[consumed..]).expect("decodes");
        assert_eq!(second, b"two");
    }

    #[test]
    fn a_partial_payload_is_incomplete_not_an_error() {
        let framed = encode_frame(b"ping").expect("within bounds");
        let err = decode_frame(&framed[..FRAME_HEADER_BYTES + 2]).unwrap_err();
        assert_eq!(
            err,
            FrameError::Incomplete {
                needed: FRAME_HEADER_BYTES + 4,
                available: FRAME_HEADER_BYTES + 2,
            }
        );
    }

    #[test]
    fn a_truncated_prefix_is_a_short_header() {
        let err = decode_frame(&[0, 0]).unwrap_err();
        assert_eq!(
            err,
            FrameError::ShortHeader {
                needed: FRAME_HEADER_BYTES,
                available: 2,
            }
        );
    }

    #[test]
    fn the_ceiling_is_inclusive() {
        assert!(FrameHeader::new(MAX_FRAME_PAYLOAD_BYTES).is_ok());
        assert!(FrameHeader::new(MAX_FRAME_PAYLOAD_BYTES + 1).is_err());
    }
}
