// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Reassembling whole frames out of an `onmessage` byte stream.
//!
//! A WebRTC "message" is not guaranteed to line up 1:1 with one
//! [`encode_frame`](crate::encode_frame) call once native and browser stacks
//! are interoperating — see the frame module's own doc comment. This is the
//! browser side's fix: buffer whatever arrives, in order, and drain complete
//! frames as soon as they are available.
//!
//! The ordering that matters is inherited straight from
//! [`FrameHeader::decode`]: an oversize declared length is rejected the
//! moment its four-byte prefix is complete, before this assembler waits for
//! (or grows its buffer toward) a payload that will only be refused. A
//! misbehaving peer cannot make this end buffer toward a length it announced
//! but never intends to send.

use crate::{FRAME_HEADER_BYTES, FrameError, FrameHeader};

/// A growable receive buffer that yields whole frame payloads.
#[derive(Debug, Default)]
pub struct FrameAssembler {
    buf: Vec<u8>,
}

impl FrameAssembler {
    /// An empty assembler.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Bytes buffered toward a frame that has not yet completed.
    ///
    /// Never exceeds one frame's worth for a peer speaking the protocol,
    /// because a completed frame is drained the moment it is available —
    /// exposed mainly so a caller can assert that invariant rather than take
    /// it on faith.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }

    /// Appends one received chunk and drains every whole frame now
    /// available, in arrival order.
    ///
    /// Returns as many frames as are complete after this call — zero, one,
    /// or several, if a single chunk happened to complete more than one.
    ///
    /// An oversize declared length is a hard error, returned instead of any
    /// frames. The chunk that revealed it is still appended first — those
    /// bytes were genuinely received, so buffering them is not the cost this
    /// crate guards against — but nothing further is buffered toward that
    /// declared length, and no attempt is made to allocate or return a
    /// payload for it. A caller that receives this error should treat the
    /// connection as done: the peer has violated the frame ceiling, and
    /// there is no partial-progress state worth resuming from.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            if self.buf.len() < FRAME_HEADER_BYTES {
                break;
            }
            // Never `ShortHeader` here — the length check above already
            // guarantees a full prefix — so this is `Ok` or `Oversize`, and
            // `Oversize` fires from the four-byte prefix alone, before any
            // payload byte is inspected.
            let header = FrameHeader::decode(&self.buf)?;
            let end = header.frame_len();
            if self.buf.len() < end {
                break;
            }
            frames.push(self.buf[FRAME_HEADER_BYTES..end].to_vec());
            self.buf.drain(..end);
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_FRAME_PAYLOAD_BYTES, encode_frame};
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn a_single_chunk_holding_one_whole_frame_decodes_immediately() {
        let framed = encode_frame(b"ping").expect("within bounds");
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push(&framed).expect("decodes");
        assert_eq!(frames, vec![b"ping".to_vec()]);
        assert_eq!(assembler.pending_len(), 0);
    }

    #[wasm_bindgen_test]
    fn a_frame_split_across_several_chunks_waits_for_the_rest() {
        let framed = encode_frame(b"hello world").expect("within bounds");
        let mut assembler = FrameAssembler::new();

        assert!(assembler.push(&framed[..2]).expect("no error").is_empty());
        assert!(assembler.push(&framed[2..6]).expect("no error").is_empty());
        let frames = assembler
            .push(&framed[6..])
            .expect("the final chunk completes it");
        assert_eq!(frames, vec![b"hello world".to_vec()]);
    }

    #[wasm_bindgen_test]
    fn one_chunk_can_complete_more_than_one_frame() {
        let mut framed = encode_frame(b"one").expect("within bounds");
        framed.extend(encode_frame(b"two").expect("within bounds"));
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push(&framed).expect("decodes both");
        assert_eq!(frames, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[wasm_bindgen_test]
    fn an_oversize_declared_length_is_rejected_from_the_prefix_alone() {
        let mut assembler = FrameAssembler::new();
        let declared = (MAX_FRAME_PAYLOAD_BYTES as u32) + 1;
        // Only the four-byte prefix arrives. No payload exists to wait for,
        // and the assembler must not sit buffering toward one that will
        // only be refused.
        let err = assembler
            .push(&declared.to_be_bytes())
            .expect_err("oversize prefix must be rejected immediately");
        assert_eq!(
            err,
            FrameError::Oversize {
                declared: u64::from(declared),
                max: MAX_FRAME_PAYLOAD_BYTES,
            }
        );
    }

    #[wasm_bindgen_test]
    fn a_frame_exactly_at_the_ceiling_still_assembles() {
        let framed = encode_frame(&vec![7u8; MAX_FRAME_PAYLOAD_BYTES]).expect("at the ceiling");
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push(&framed).expect("decodes");
        assert_eq!(frames[0].len(), MAX_FRAME_PAYLOAD_BYTES);
    }
}
