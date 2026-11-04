// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Synthesis: rebuild speech from the parameters, not from the waveform.
//!
//! An excitation is manufactured, either a train of pulses at the
//! transmitted pitch or white noise, and pushed through the all-pole filter
//! the reflection coefficients describe. The result sounds like the talker
//! and carries the words; it does not resemble the original waveform sample
//! by sample, and no measure that compares them sample by sample says
//! anything useful about it.

use super::{FRAME_SAMPLES, ORDER, Params, analysis};
use crate::math;

/// Parameters are interpolated across this many subframes, so a filter
/// never jumps between frames. Reflection coefficients interpolate safely:
/// both endpoints sit inside (-1, 1) and so does everything between them.
const SUBFRAMES: usize = 4;
const SUBFRAME_LEN: usize = FRAME_SAMPLES / SUBFRAMES;

/// Matches the analyser's pre-emphasis.
const PRE_EMPHASIS: f32 = 0.9375;

/// Ceiling on filter state, so no arithmetic edge can push the recursion to
/// infinity and leave it there.
const STATE_LIMIT: f32 = 1.0e6;

/// Rolling synthesis state.
///
/// Unlike the ADPCM decoder this one is not stateless: it carries filter
/// memory, the pitch pulse phase, and the previous frame's parameters for
/// interpolation. A frame still *describes* itself completely, so a decoder
/// that missed earlier frames recovers within a few milliseconds rather
/// than desynchronising.
pub struct Synthesizer {
    previous: Params,
    filter_memory: [f32; ORDER],
    deemphasis: f32,
    /// Samples until the next glottal pulse, carried across frames so pitch
    /// stays continuous.
    pulse_countdown: usize,
    rng: u32,
}

impl Default for Synthesizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Synthesizer {
    pub const fn new() -> Self {
        Self {
            previous: Params::silence(),
            filter_memory: [0.0; ORDER],
            deemphasis: 0.0,
            pulse_countdown: 0,
            rng: 0x2545_f491,
        }
    }

    /// Uniform noise in [-1, 1) from a xorshift generator. Deterministic on
    /// purpose: the same frame decodes to the same samples everywhere.
    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Synthesise one frame from its parameters.
    pub fn synthesize(&mut self, params: &Params, out: &mut [i16]) {
        debug_assert_eq!(out.len(), FRAME_SAMPLES);

        for subframe in 0..SUBFRAMES {
            // Ramp from the previous frame's shape to this one.
            let t = (subframe as f32 + 0.5) / SUBFRAMES as f32;
            let mut rc = [0.0f32; ORDER];
            for (i, slot) in rc.iter_mut().enumerate() {
                *slot = self.previous.rc[i] + t * (params.rc[i] - self.previous.rc[i]);
            }
            let gain = self.previous.gain + t * (params.gain - self.previous.gain);
            let predictor = analysis::reflection_to_predictor(&rc);

            let period = if params.voiced {
                params.pitch.max(1) as usize
            } else {
                0
            };

            let start = subframe * SUBFRAME_LEN;
            for sample in &mut out[start..start + SUBFRAME_LEN] {
                let excitation = if params.voiced {
                    if self.pulse_countdown == 0 {
                        self.pulse_countdown = period.saturating_sub(1);
                        // One pulse per period carrying the whole subframe's
                        // energy: RMS of an impulse train of amplitude A and
                        // period T is A / sqrt(T).
                        gain * math::sqrt(period as f32)
                    } else {
                        self.pulse_countdown -= 1;
                        0.0
                    }
                } else {
                    // Uniform noise has RMS 1/sqrt(3); scale to match.
                    self.noise() * gain * 1.732_050_8
                };

                let mut value = excitation;
                for (j, &memory) in self.filter_memory.iter().enumerate() {
                    value += predictor[j + 1] * memory;
                }
                if !value.is_finite() {
                    value = 0.0;
                }
                let value = value.clamp(-STATE_LIMIT, STATE_LIMIT);

                self.filter_memory.copy_within(..ORDER - 1, 1);
                self.filter_memory[0] = value;

                // Undo the analyser's pre-emphasis.
                let output = value + PRE_EMPHASIS * self.deemphasis;
                let output = if output.is_finite() {
                    output.clamp(-STATE_LIMIT, STATE_LIMIT)
                } else {
                    0.0
                };
                self.deemphasis = output;

                *sample = output.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            }
        }
        self.previous = *params;
    }
}
