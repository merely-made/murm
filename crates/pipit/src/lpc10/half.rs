// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Half-rate mode: 1,600 bps, by sending the spectrum half as often.
//!
//! The full-rate frame spends 41 of its 53 bits on the vocal tract and only
//! 12 on pitch, gain and voicing. But the tract moves slowly, while pitch and
//! energy move fast, so sending both at the same rate overpays for the slow
//! one.
//!
//! A superframe covers two 22.5 ms frames and carries the spectrum once, from
//! the second frame, plus pitch, gain and voicing for each. That is 41 + 24 =
//! 65 bits in 9 bytes per 45 ms: 1,600 bps against the full rate's 2,489, a
//! 36% saving with no trained data and no new analysis.
//!
//! The first frame's spectrum is not transmitted. The decoder places it
//! halfway between the previous superframe's and this one's, which is safe in
//! the reflection-coefficient domain: the midpoint of two values inside
//! (-1, 1) is also inside it, so the interpolated filter is stable by
//! construction.
//!
//! What this costs is spectral resolution during fast articulation, since a
//! consonant that comes and goes inside 45 ms is only half seen. Pitch and
//! energy are unaffected, being sent per frame as before.

use super::{Error, FRAME_SAMPLES, ORDER, Params, analysis, quant, synth};

/// Frames per superframe.
pub const FRAMES: usize = 2;

/// Samples per superframe: 45 ms at 8 kHz.
pub const SUPERFRAME_SAMPLES: usize = FRAME_SAMPLES * FRAMES;

/// Bytes per encoded superframe. 9 bytes per 45 ms is 1,600 bps.
pub const SUPERFRAME_BYTES: usize = 9;

/// Pack two analysed frames, taking the spectrum from the second.
fn pack(first: &Params, second: &Params) -> [u8; SUPERFRAME_BYTES] {
    let mut writer = quant::BitWriter::<SUPERFRAME_BYTES>::new();
    for params in [first, second] {
        writer.write(u32::from(params.voiced), 1);
        writer.write(quant::pitch_code(params.pitch), quant::PITCH_BITS);
        writer.write(quant::gain_code(params.gain), quant::GAIN_BITS);
    }
    for (i, &k) in second.rc.iter().enumerate() {
        writer.write(quant::rc_code(i, k), quant::RC_BITS[i]);
    }
    writer.bytes
}

/// Unpack a superframe.
///
/// Both returned frames carry the transmitted spectrum; the caller replaces
/// the first frame's with its interpolation, which is the one thing the
/// bitstream does not say.
fn unpack(bytes: &[u8]) -> (Params, Params) {
    let mut reader = quant::BitReader::new(bytes);
    let mut frames = [Params::silence(); FRAMES];
    for frame in &mut frames {
        frame.voiced = reader.read(1) == 1;
        frame.pitch = quant::pitch_from_code(reader.read(quant::PITCH_BITS));
        frame.gain = quant::gain_from_code(reader.read(quant::GAIN_BITS));
    }
    let mut rc = [0.0f32; ORDER];
    for (i, slot) in rc.iter_mut().enumerate() {
        *slot = quant::rc_from_code(i, reader.read(quant::RC_BITS[i]));
    }
    frames[0].rc = rc;
    frames[1].rc = rc;
    (frames[0], frames[1])
}

/// Analyses speech into half-rate superframes.
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

    /// Encode exactly [`SUPERFRAME_SAMPLES`] samples into
    /// [`SUPERFRAME_BYTES`] bytes.
    pub fn encode_superframe(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, Error> {
        if pcm.len() != SUPERFRAME_SAMPLES {
            return Err(Error::InvalidHeader);
        }
        if out.len() < SUPERFRAME_BYTES {
            return Err(Error::ShortBuffer);
        }
        // Both halves are analysed in full; only the first half's spectrum
        // goes unsent, so the analyser's history stays continuous.
        let first = self.analyzer.analyze(&pcm[..FRAME_SAMPLES]);
        let second = self.analyzer.analyze(&pcm[FRAME_SAMPLES..]);
        out[..SUPERFRAME_BYTES].copy_from_slice(&pack(&first, &second));
        Ok(SUPERFRAME_BYTES)
    }
}

/// Rebuilds speech from half-rate superframes.
pub struct Decoder {
    synthesizer: synth::Synthesizer,
    /// The last transmitted spectrum, which the next superframe's first
    /// frame interpolates from.
    previous_rc: [f32; ORDER],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            synthesizer: synth::Synthesizer::new(),
            previous_rc: [0.0; ORDER],
        }
    }

    /// Decode one superframe into exactly [`SUPERFRAME_SAMPLES`] samples.
    ///
    /// Every bit pattern is a valid superframe, so this fails only when a
    /// buffer is the wrong size.
    pub fn decode_superframe(&mut self, frame: &[u8], pcm: &mut [i16]) -> Result<usize, Error> {
        if frame.len() < SUPERFRAME_BYTES {
            return Err(Error::Truncated);
        }
        if pcm.len() != SUPERFRAME_SAMPLES {
            return Err(Error::ShortBuffer);
        }
        let (mut first, second) = unpack(frame);
        for (slot, (&previous, &current)) in first
            .rc
            .iter_mut()
            .zip(self.previous_rc.iter().zip(second.rc.iter()))
        {
            *slot = 0.5 * (previous + current);
        }
        self.synthesizer
            .synthesize(&first, &mut pcm[..FRAME_SAMPLES]);
        self.synthesizer
            .synthesize(&second, &mut pcm[FRAME_SAMPLES..]);
        self.previous_rc = second.rc;
        Ok(SUPERFRAME_SAMPLES)
    }
}

/// Test-only view of [`unpack`], so the suite can assert on transmitted
/// parameters rather than only on rendered audio.
#[cfg(test)]
pub(crate) fn tests_unpack(bytes: &[u8]) -> (Params, Params) {
    unpack(bytes)
}
