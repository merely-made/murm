// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The clip container: a self-describing block of encoded speech.
//!
//! A clip is what a recorded voice message *is* on the wire. It names its own
//! codec, mode, sample rate, and frame geometry, so a receiver can decode one
//! without out-of-band agreement and a later codec can be added without a
//! format break. Frames follow the header back to back.
//!
//! The clip is deliberately only the payload. It carries no sender,
//! recipient, timestamp, signature, or routing: those belong to whatever
//! envelope carries it (an LXMF field, a Reticulum resource, a file). Keeping
//! that line means a host with no radio stack can still decode a clip.

use crate::{Error, adpcm, lpc10};

/// Leading bytes of every clip.
pub const MAGIC: [u8; 5] = *b"PIPIT";

/// Container version this build writes and reads.
pub const VERSION: u8 = 1;

/// Bytes of clip header preceding the first frame.
pub const HEADER_LEN: usize = 18;

/// Upper bound on frame length, so a hostile header cannot ask for an
/// unreasonable buffer.
pub const MAX_FRAME_SAMPLES: u16 = 4096;

/// Which codec produced a clip's frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Codec {
    /// 4-bit IMA ADPCM, Pipit framing, about 32 kbps. Mode is always 0.
    /// Any frame length; sized for WiFi and TCP bearers.
    ImaAdpcm,
    /// 10th-order LPC vocoder, 2,489 bps. Mode is always 0. Frames are
    /// fixed at [`lpc10::FRAME_SAMPLES`]; sized for LoRa.
    Lpc10,
    /// The same vocoder at half rate, 1,600 bps, by sending the spectrum
    /// once per two frames. Mode is always 0. Frames are fixed at
    /// [`lpc10::half::SUPERFRAME_SAMPLES`]; sized for a link where airtime
    /// is the binding constraint.
    Lpc10Half,
}

impl Codec {
    /// Wire identifier. Values are permanent once shipped; a new codec takes
    /// a new number rather than redefining one.
    pub const fn id(self) -> u8 {
        match self {
            Self::ImaAdpcm => 1,
            Self::Lpc10 => 2,
            Self::Lpc10Half => 3,
        }
    }

    fn from_id(id: u8) -> Result<Self, Error> {
        match id {
            1 => Ok(Self::ImaAdpcm),
            2 => Ok(Self::Lpc10),
            3 => Ok(Self::Lpc10Half),
            other => Err(Error::UnknownCodec(other)),
        }
    }

    /// Encoded length of one frame of `frame_samples` samples.
    pub const fn frame_encoded_len(self, frame_samples: usize) -> usize {
        match self {
            Self::ImaAdpcm => adpcm::frame_encoded_len(frame_samples),
            Self::Lpc10 => lpc10::FRAME_BYTES,
            Self::Lpc10Half => lpc10::half::SUPERFRAME_BYTES,
        }
    }

    /// The only frame length this codec accepts, when it has one.
    ///
    /// The vocoder's quantizers and pitch range are tied to its 22.5 ms
    /// frame, so unlike ADPCM it does not take an arbitrary geometry.
    pub const fn required_frame_samples(self) -> Option<u16> {
        match self {
            Self::ImaAdpcm => None,
            Self::Lpc10 => Some(lpc10::FRAME_SAMPLES as u16),
            Self::Lpc10Half => Some(lpc10::half::SUPERFRAME_SAMPLES as u16),
        }
    }
}

/// A clip's declared shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipHeader {
    pub codec: Codec,
    /// Codec-specific mode. Zero for [`Codec::ImaAdpcm`].
    pub mode: u8,
    pub sample_rate: u32,
    pub frame_samples: u16,
    /// Total PCM samples, which trims the padding in the final frame.
    pub sample_count: u32,
}

impl ClipHeader {
    /// Frames needed to hold `sample_count` samples.
    pub const fn frame_count(&self) -> u32 {
        self.sample_count.div_ceil(self.frame_samples as u32)
    }

    /// Duration in milliseconds, rounded down.
    pub const fn duration_ms(&self) -> u32 {
        (self.sample_count as u64 * 1000 / self.sample_rate as u64) as u32
    }

    /// Total clip length in bytes, header included.
    pub const fn encoded_len(&self) -> usize {
        HEADER_LEN
            + self.frame_count() as usize
                * self.codec.frame_encoded_len(self.frame_samples as usize)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.sample_rate == 0 {
            return Err(Error::InvalidHeader);
        }
        if self.frame_samples == 0 || self.frame_samples > MAX_FRAME_SAMPLES {
            return Err(Error::InvalidHeader);
        }
        if self.mode != 0 {
            return Err(Error::UnsupportedMode(self.mode));
        }
        // A codec with fixed geometry refuses any other, rather than
        // producing frames neither side can parse.
        match self.codec.required_frame_samples() {
            Some(required) if self.frame_samples != required => Err(Error::InvalidHeader),
            _ => Ok(()),
        }
    }

    /// Write the header into `out`, returning bytes written.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Error> {
        self.validate()?;
        if out.len() < HEADER_LEN {
            return Err(Error::ShortBuffer);
        }
        out[0..5].copy_from_slice(&MAGIC);
        out[5] = VERSION;
        out[6] = self.codec.id();
        out[7] = self.mode;
        out[8..12].copy_from_slice(&self.sample_rate.to_le_bytes());
        out[12..14].copy_from_slice(&self.frame_samples.to_le_bytes());
        out[14..18].copy_from_slice(&self.sample_count.to_le_bytes());
        Ok(HEADER_LEN)
    }

    /// Parse a header from the front of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::Truncated);
        }
        if bytes[0..5] != MAGIC {
            return Err(Error::BadMagic);
        }
        if bytes[5] != VERSION {
            return Err(Error::UnsupportedVersion(bytes[5]));
        }
        let header = Self {
            codec: Codec::from_id(bytes[6])?,
            mode: bytes[7],
            sample_rate: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            frame_samples: u16::from_le_bytes([bytes[12], bytes[13]]),
            sample_count: u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]),
        };
        header.validate()?;
        Ok(header)
    }
}

/// How a clip should be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipParams {
    pub codec: Codec,
    pub sample_rate: u32,
    pub frame_samples: u16,
}

impl ClipParams {
    /// ADPCM at 8 kHz in 20 ms frames: the fat-bearer default.
    pub const fn adpcm() -> Self {
        Self {
            codec: Codec::ImaAdpcm,
            sample_rate: 8_000,
            frame_samples: 160,
        }
    }

    /// The vocoder at half rate, 1,600 bps: a third less airtime than
    /// [`Self::lpc10`], at the cost of spectral detail during fast
    /// articulation.
    pub const fn lpc10_half() -> Self {
        Self {
            codec: Codec::Lpc10Half,
            sample_rate: 8_000,
            frame_samples: crate::lpc10::half::SUPERFRAME_SAMPLES as u16,
        }
    }

    /// The vocoder at 8 kHz, in the only frame length it accepts. This is
    /// the setting for a link where bytes are scarce.
    pub const fn lpc10() -> Self {
        Self {
            codec: Codec::Lpc10,
            sample_rate: 8_000,
            frame_samples: crate::lpc10::FRAME_SAMPLES as u16,
        }
    }
}

impl Default for ClipParams {
    fn default() -> Self {
        Self::adpcm()
    }
}

#[cfg(feature = "alloc")]
mod with_alloc {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Encode PCM into a complete clip.
    ///
    /// A trailing partial frame is padded by holding the last sample, so the
    /// padding adds no transient; `sample_count` in the header trims it on
    /// decode.
    pub fn encode_clip(pcm: &[i16], params: ClipParams) -> Result<Vec<u8>, Error> {
        let sample_count = u32::try_from(pcm.len()).map_err(|_| Error::TooLong)?;
        let header = ClipHeader {
            codec: params.codec,
            mode: 0,
            sample_rate: params.sample_rate,
            frame_samples: params.frame_samples,
            sample_count,
        };
        header.validate()?;

        let frame_samples = params.frame_samples as usize;
        let mut out = vec![0u8; header.encoded_len()];
        header.write(&mut out)?;

        let mut adpcm_encoder = adpcm::Encoder::new();
        let mut lpc_encoder = lpc10::Encoder::new();
        let mut half_encoder = lpc10::half::Encoder::new();
        let mut scratch = vec![0i16; frame_samples];
        let frame_len = params.codec.frame_encoded_len(frame_samples);
        for (i, chunk) in pcm.chunks(frame_samples).enumerate() {
            scratch[..chunk.len()].copy_from_slice(chunk);
            // Hold the last sample through the padding rather than dropping
            // to silence.
            let pad = chunk.last().copied().unwrap_or(0);
            scratch[chunk.len()..].fill(pad);
            let at = HEADER_LEN + i * frame_len;
            let frame = &mut out[at..at + frame_len];
            match params.codec {
                Codec::ImaAdpcm => adpcm_encoder.encode_frame(&scratch, frame)?,
                Codec::Lpc10 => lpc_encoder.encode_frame(&scratch, frame)?,
                Codec::Lpc10Half => half_encoder.encode_superframe(&scratch, frame)?,
            };
        }
        Ok(out)
    }

    /// Decode a complete clip into PCM.
    pub fn decode_clip(bytes: &[u8]) -> Result<(ClipHeader, Vec<i16>), Error> {
        let header = ClipHeader::parse(bytes)?;
        // Trust the byte length, not the declared count: a header claiming
        // more audio than it carries is truncated, not an allocation hint.
        if bytes.len() < header.encoded_len() {
            return Err(Error::Truncated);
        }

        let frame_samples = header.frame_samples as usize;
        let frame_len = header.codec.frame_encoded_len(frame_samples);
        let mut pcm = vec![0i16; header.sample_count as usize];
        let mut lpc_decoder = lpc10::Decoder::new();
        let mut half_decoder = lpc10::half::Decoder::new();
        // The vocoder always renders a whole frame; a trailing partial one
        // is rendered here and trimmed on the way out.
        let mut scratch = vec![0i16; frame_samples];
        for (i, out) in pcm.chunks_mut(frame_samples).enumerate() {
            let at = HEADER_LEN + i * frame_len;
            let frame = &bytes[at..at + frame_len];
            match header.codec {
                Codec::ImaAdpcm => {
                    adpcm::decode_frame(frame, out)?;
                },
                Codec::Lpc10 => {
                    lpc_decoder.decode_frame(frame, &mut scratch)?;
                    out.copy_from_slice(&scratch[..out.len()]);
                },
                Codec::Lpc10Half => {
                    half_decoder.decode_superframe(frame, &mut scratch)?;
                    out.copy_from_slice(&scratch[..out.len()]);
                },
            }
        }
        Ok((header, pcm))
    }
}

#[cfg(feature = "alloc")]
pub use with_alloc::{decode_clip, encode_clip};

#[cfg(test)]
mod tests;
