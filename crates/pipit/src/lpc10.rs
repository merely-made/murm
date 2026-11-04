// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A 2.4 kbps LPC vocoder: 10th-order, 22.5 ms frames, 7 bytes each.
//!
//! Where ADPCM sends a cheaper description of the waveform, this sends no
//! waveform at all. Each frame carries only what it takes to *rebuild*
//! speech: is this voiced, at what pitch, through what vocal-tract shape,
//! at what energy. Ten reflection coefficients, a pitch, a gain, a voicing
//! bit. That is why it fits a LoRa link and why it sounds synthetic.
//!
//! # Relationship to FIPS-137
//!
//! This is built to the *structure* of LPC-10e, the 2.4 kbps US federal
//! standard: the same frame length, model order, parameter set, and bit
//! allocation. The quantizer tables are Pipit's own, because the standard's
//! tables were not available to implement from. **A Pipit LPC frame is
//! therefore not interchangeable with a FIPS-137 frame**, and this codec
//! does not claim that interoperability. A conformant variant, if it is
//! ever wanted, takes a new codec identifier rather than changing this one.
//!
//! # Measuring it
//!
//! Signal-to-noise ratio against the input is meaningless here and will
//! read as negative: the output is a resynthesis, not an approximation of
//! the original samples. What is meaningful is whether the spectral
//! envelope, pitch, voicing, and energy survive, which is what the tests
//! assert.

use crate::Error;

mod analysis;
pub mod half;
mod quant;
mod synth;

#[cfg(test)]
mod tests;

/// Samples per frame: 22.5 ms at 8 kHz.
pub const FRAME_SAMPLES: usize = 180;

/// Bytes per encoded frame. 7 bytes per 22.5 ms is 2,489 bps.
pub const FRAME_BYTES: usize = 7;

/// Predictor order: ten reflection coefficients.
pub const ORDER: usize = 10;

/// Shortest pitch period the coder represents, 400 Hz.
pub const PITCH_MIN: usize = 20;

/// Longest pitch period the coder represents, about 51 Hz.
pub const PITCH_MAX: usize = 156;

/// Samples of context the analyser keeps, enough for the longest period.
const HISTORY: usize = PITCH_MAX + FRAME_SAMPLES;

/// Everything one frame says about 22.5 ms of speech.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Voiced (vocal folds vibrating) or unvoiced (turbulent noise).
    pub voiced: bool,
    /// Pitch period in samples. Meaningful when `voiced`.
    pub pitch: u8,
    /// RMS of the prediction residual, which sets the excitation level.
    pub gain: f32,
    /// Reflection coefficients, all strictly inside (-1, 1).
    pub rc: [f32; ORDER],
}

impl Params {
    /// The parameters of silence, and the state a decoder starts from.
    pub const fn silence() -> Self {
        Self {
            voiced: false,
            pitch: PITCH_MIN as u8,
            gain: 0.0,
            rc: [0.0; ORDER],
        }
    }
}

/// Analyses speech into frames.
///
/// Stateful across frames: pitch analysis looks back further than one frame
/// and pre-emphasis is continuous.
#[derive(Default)]
pub struct Encoder {
    analyzer: analysis::Analyzer,
}

impl Encoder {
    pub const fn new() -> Self {
        Self {
            analyzer: analysis::Analyzer::new(),
        }
    }

    /// Encode exactly [`FRAME_SAMPLES`] samples into [`FRAME_BYTES`] bytes.
    pub fn encode_frame(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, Error> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(Error::InvalidHeader);
        }
        if out.len() < FRAME_BYTES {
            return Err(Error::ShortBuffer);
        }
        let params = self.analyzer.analyze(pcm);
        out[..FRAME_BYTES].copy_from_slice(&quant::pack(&params));
        Ok(FRAME_BYTES)
    }

    /// The parameters this frame would transmit, without packing them.
    /// Exposed for analysis tools and tests.
    pub fn analyze(&mut self, pcm: &[i16]) -> Result<Params, Error> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(Error::InvalidHeader);
        }
        Ok(self.analyzer.analyze(pcm))
    }
}

/// Rebuilds speech from frames.
///
/// Stateful, unlike [`crate::adpcm::decode_frame`]: synthesis carries filter
/// memory, pitch phase, and the previous frame's parameters for
/// interpolation. A lost frame costs a few milliseconds of convergence, not
/// the rest of the stream, because each frame describes itself completely.
#[derive(Default)]
pub struct Decoder {
    synthesizer: synth::Synthesizer,
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            synthesizer: synth::Synthesizer::new(),
        }
    }

    /// Decode one frame into exactly [`FRAME_SAMPLES`] samples.
    ///
    /// Every bit pattern is a valid frame, so this fails only when a buffer
    /// is the wrong size.
    pub fn decode_frame(&mut self, frame: &[u8], pcm: &mut [i16]) -> Result<usize, Error> {
        if frame.len() < FRAME_BYTES {
            return Err(Error::Truncated);
        }
        if pcm.len() != FRAME_SAMPLES {
            return Err(Error::ShortBuffer);
        }
        let params = quant::unpack(frame);
        self.synthesizer.synthesize(&params, pcm);
        Ok(FRAME_SAMPLES)
    }
}
