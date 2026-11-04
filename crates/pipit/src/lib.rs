// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

#![no_std]
#![forbid(unsafe_code)]

//! A small speech codec for voices that travel light.
//!
//! Pipit encodes speech for links where bytes are expensive: recorded voice
//! messages carried store-and-forward over a mesh, and later live or
//! push-to-talk calls over the same paths. It is the lofi lane on purpose.
//! Where a hifi codec preserves the recording, Pipit preserves the message.
//!
//! # Three codecs, one for each kind of link
//!
//! - [`Codec::ImaAdpcm`]: 4 bits per sample, about 32 kbps at 8 kHz. A
//!   cheaper description of the waveform, so it still sounds like the
//!   talker's recording. Right for a WiFi or TCP bearer, too fat for LoRa.
//! - [`Codec::Lpc10`]: a 10th-order vocoder at 2,489 bps, thirteen times
//!   smaller. It transmits no waveform at all, only what it takes to
//!   *rebuild* speech: voicing, pitch, vocal-tract shape, energy. That is
//!   why a ten-second memo fits in about 3 KB, and why it sounds synthetic.
//!   See [`lpc10`] for what it does and does not claim about FIPS-137.
//! - [`Codec::Lpc10Half`]: the same vocoder at 1,600 bps, by sending the
//!   vocal tract once per two frames instead of every frame. The tract moves
//!   slowly while pitch and energy move fast, so this drops a third of the
//!   airtime for almost no measured spectral cost. Ten seconds is 2 KB.
//!
//! Clips name their codec, so a receiver decodes either without prior
//! agreement and a third can be added without a format break.
//!
//! # Two layers
//!
//! - **Frames** are the streaming primitive a call drives directly. Each
//!   frame describes itself, so a frame lost mid-call costs its own few
//!   milliseconds and nothing after it: an [`adpcm`] frame carries the
//!   predictor state it starts from, and an [`lpc10`] frame carries a whole
//!   parametric description of its own audio.
//! - **Clips** ([`clip`]) are the stored primitive: a header naming codec,
//!   mode, sample rate, and frame geometry, then frames back to back. A
//!   recorded voice message is a clip.
//!
//! A clip is only the payload. Sender, recipient, timestamp, and signature
//! belong to whatever envelope carries it, which keeps a clip decodable by a
//! host with no radio stack.
//!
//! # Portability
//!
//! `no_std` and dependency-free. The frame API allocates nothing and runs on
//! caller buffers; the `alloc` feature (on by default) adds the whole-clip
//! conveniences. Firmware builds with `default-features = false`.
//!
//! ```
//! use pipit::{ClipParams, decode_clip, encode_clip};
//!
//! let pcm: Vec<i16> = (0..8_000).map(|i| ((i as f32 / 8.0).sin() * 8000.0) as i16).collect();
//!
//! // One second of speech: 4,168 bytes as ADPCM, 333 through the vocoder,
//! // 225 at half rate.
//! let clip = encode_clip(&pcm, ClipParams::lpc10_half())?;
//! assert_eq!(clip.len(), 225);
//!
//! let (header, decoded) = decode_clip(&clip)?;
//! assert_eq!(header.duration_ms(), 1000);
//! assert_eq!(decoded.len(), pcm.len());
//! # Ok::<(), pipit::Error>(())
//! ```

#[cfg(feature = "alloc")]
extern crate alloc;
// The built-in test harness needs `std`, and the math tests compare against
// it. Nothing outside `cfg(test)` may use it.
#[cfg(test)]
extern crate std;

pub mod adpcm;
pub mod clip;
pub mod lpc10;
mod math;

pub use clip::{ClipHeader, ClipParams, Codec, HEADER_LEN, MAGIC, MAX_FRAME_SAMPLES, VERSION};

#[cfg(feature = "alloc")]
pub use clip::{decode_clip, encode_clip};

/// Everything that can go wrong reading or writing Pipit audio.
///
/// Decoding is a hostile-input surface: a clip may arrive from a stranger
/// over the air. Every path here returns an error rather than panicking, and
/// no declared length is trusted enough to size a buffer on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The caller's output buffer is smaller than the encoded form needs.
    ShortBuffer,
    /// Input ended before the declared structure did.
    Truncated,
    /// Leading bytes are not [`MAGIC`].
    BadMagic,
    /// Container version this build does not read.
    UnsupportedVersion(u8),
    /// Codec identifier this build does not know.
    UnknownCodec(u8),
    /// Codec mode this build does not implement.
    UnsupportedMode(u8),
    /// Header fields are self-inconsistent or out of range.
    InvalidHeader,
    /// A frame's coder state is not a value the codec can have produced.
    InvalidState,
    /// More audio than the container's counters can address.
    TooLong,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortBuffer => write!(f, "output buffer too small"),
            Self::Truncated => write!(f, "input ended early"),
            Self::BadMagic => write!(f, "not a pipit clip"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported clip version {v}"),
            Self::UnknownCodec(id) => write!(f, "unknown codec id {id}"),
            Self::UnsupportedMode(m) => write!(f, "unsupported codec mode {m}"),
            Self::InvalidHeader => write!(f, "invalid clip header"),
            Self::InvalidState => write!(f, "invalid coder state in frame"),
            Self::TooLong => write!(f, "audio too long for the container"),
        }
    }
}

impl core::error::Error for Error {}
