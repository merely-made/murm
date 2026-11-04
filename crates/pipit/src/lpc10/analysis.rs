// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Analysis: turn a frame of speech into the parameters that describe it.
//!
//! Four things come out of each frame: whether it is voiced, its pitch
//! period, the shape of the vocal tract as ten reflection coefficients, and
//! the energy of what is left after prediction. That is the whole message.
//! The waveform is not transmitted and is not recoverable, which is what
//! buys the bitrate.

use super::{FRAME_SAMPLES, HISTORY, ORDER, PITCH_MAX, PITCH_MIN, Params};
use crate::math;

/// Pre-emphasis coefficient. Speech falls off about 6 dB per octave, so
/// flattening it first puts the prediction error where the ear is.
const PRE_EMPHASIS: f32 = 0.9375;

/// Correlation a pitch candidate must reach before the frame is called
/// voiced.
const VOICING_CORRELATION: f32 = 0.35;

/// Zero-crossing rate above which a frame reads as fricative noise even if
/// something correlated.
const VOICING_MAX_ZCR: f32 = 0.30;

/// Below this RMS a frame is silence; calling it unvoiced keeps the
/// synthesiser from hissing through the gaps.
const SILENCE_RMS: f32 = 12.0;

/// A shorter candidate this close to the best score is preferred, which is
/// what keeps the tracker off octave-down errors.
const SUBMULTIPLE_MARGIN: f32 = 0.80;

/// Rolling analysis state. Pitch needs to look back further than one frame,
/// so the analyser keeps enough history for the longest period it supports.
pub struct Analyzer {
    /// Raw input, oldest first; the current frame occupies the tail.
    history: [f32; HISTORY],
    /// Last raw sample of the previous frame, so pre-emphasis is continuous.
    previous_sample: f32,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub const fn new() -> Self {
        Self {
            history: [0.0; HISTORY],
            previous_sample: 0.0,
        }
    }

    /// Analyse one frame, which must be [`FRAME_SAMPLES`] long.
    pub fn analyze(&mut self, frame: &[i16]) -> Params {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES);
        self.history.copy_within(FRAME_SAMPLES.., 0);
        let tail = HISTORY - FRAME_SAMPLES;
        for (slot, &sample) in self.history[tail..].iter_mut().zip(frame) {
            *slot = sample as f32;
        }

        let (pitch, correlation) = self.estimate_pitch();
        let zcr = zero_crossing_rate(&self.history[tail..]);
        let rms = frame_rms(&self.history[tail..]);

        let voiced = rms >= SILENCE_RMS
            && correlation >= VOICING_CORRELATION
            && zcr <= VOICING_MAX_ZCR;

        let (rc, residual_rms) = self.linear_prediction();

        Params {
            voiced,
            pitch: pitch as u8,
            gain: residual_rms,
            rc,
        }
    }

    /// Normalised autocorrelation over the supported period range, on a
    /// smoothed copy of the signal.
    ///
    /// Returns the best period and its correlation. The correlation is what
    /// the voicing decision reads, so an unvoiced frame still reports the
    /// lag it liked most; the caller ignores it.
    fn estimate_pitch(&self) -> (usize, f32) {
        let mut smoothed = [0.0f32; HISTORY];
        smooth(&self.history, &mut smoothed);

        let start = HISTORY - FRAME_SAMPLES;
        let current = &smoothed[start..];
        let current_energy: f32 = current.iter().map(|s| s * s).sum();
        if current_energy <= 0.0 {
            return (PITCH_MIN, 0.0);
        }

        let mut best_lag = PITCH_MIN;
        let mut best_score = -1.0f32;
        let mut scores = [0.0f32; PITCH_MAX + 1];
        for lag in PITCH_MIN..=PITCH_MAX {
            let delayed = &smoothed[start - lag..start - lag + FRAME_SAMPLES];
            let mut cross = 0.0f32;
            let mut delayed_energy = 0.0f32;
            for (a, b) in current.iter().zip(delayed) {
                cross += a * b;
                delayed_energy += b * b;
            }
            let denominator = math::sqrt(current_energy * delayed_energy);
            let score = if denominator > 0.0 {
                cross / denominator
            } else {
                0.0
            };
            scores[lag] = score;
            if score > best_score {
                best_score = score;
                best_lag = lag;
            }
        }

        // A period of 2T correlates as well as T, so the raw maximum lands
        // an octave low about as often as not. Walk down the submultiples
        // and take the shortest one that still scores nearly as well.
        let mut chosen = best_lag;
        let mut divisor = 2;
        while best_lag / divisor >= PITCH_MIN {
            let candidate = best_lag / divisor;
            if scores[candidate] >= best_score * SUBMULTIPLE_MARGIN {
                chosen = candidate;
            }
            divisor += 1;
        }
        (chosen, best_score)
    }

    /// Windowed autocorrelation into reflection coefficients, plus the RMS
    /// of the prediction residual.
    fn linear_prediction(&mut self) -> ([f32; ORDER], f32) {
        let tail = HISTORY - FRAME_SAMPLES;
        let mut windowed = [0.0f32; FRAME_SAMPLES];
        let mut previous = self.previous_sample;
        for (i, &sample) in self.history[tail..].iter().enumerate() {
            windowed[i] = sample - PRE_EMPHASIS * previous;
            previous = sample;
        }
        self.previous_sample = previous;

        for (i, sample) in windowed.iter_mut().enumerate() {
            *sample *= hamming(i);
        }

        let mut autocorrelation = [0.0f32; ORDER + 1];
        for (lag, slot) in autocorrelation.iter_mut().enumerate() {
            *slot = windowed[lag..]
                .iter()
                .zip(&windowed)
                .map(|(a, b)| a * b)
                .sum();
        }

        if autocorrelation[0] <= 0.0 {
            return ([0.0; ORDER], 0.0);
        }
        // A touch of white noise and a mild lag window keep the recursion
        // away from the singular case that a pure tone or a clipped frame
        // would otherwise produce.
        autocorrelation[0] *= 1.0001;
        for (lag, slot) in autocorrelation.iter_mut().enumerate().skip(1) {
            let f = lag as f32 * 0.008;
            *slot *= math::exp2(-0.5 * f * f * core::f32::consts::LOG2_E);
        }

        let (rc, residual_energy) = levinson(&autocorrelation);
        let residual_rms = math::sqrt(residual_energy / FRAME_SAMPLES as f32);
        (rc, residual_rms)
    }
}

/// Levinson-Durbin: autocorrelation to reflection coefficients.
///
/// Returns the coefficients and the energy remaining after prediction. Every
/// returned coefficient satisfies `|k| < 1`, so the matching synthesis
/// filter is stable by construction.
fn levinson(r: &[f32; ORDER + 1]) -> ([f32; ORDER], f32) {
    let mut rc = [0.0f32; ORDER];
    let mut a = [0.0f32; ORDER + 1];
    let mut previous = [0.0f32; ORDER + 1];
    let mut error = r[0];

    for i in 1..=ORDER {
        let mut acc = r[i];
        for j in 1..i {
            acc -= previous[j] * r[i - j];
        }
        if error <= 0.0 {
            break;
        }
        let mut k = acc / error;
        // Clamp rather than trust the arithmetic: a marginally unstable
        // coefficient here becomes an exploding filter at the far end.
        k = k.clamp(-0.999, 0.999);
        rc[i - 1] = k;

        a[i] = k;
        for j in 1..i {
            a[j] = previous[j] - k * previous[i - j];
        }
        previous[..=i].copy_from_slice(&a[..=i]);
        error *= 1.0 - k * k;
    }
    (rc, error.max(0.0))
}

/// Reflection coefficients to direct-form predictor coefficients.
///
/// The inverse of what [`levinson`] accumulates, and the decoder's way back
/// to a filter it can run.
pub fn reflection_to_predictor(rc: &[f32; ORDER]) -> [f32; ORDER + 1] {
    let mut a = [0.0f32; ORDER + 1];
    let mut previous = [0.0f32; ORDER + 1];
    for i in 1..=ORDER {
        let k = rc[i - 1];
        a[i] = k;
        for j in 1..i {
            a[j] = previous[j] - k * previous[i - j];
        }
        previous[..=i].copy_from_slice(&a[..=i]);
    }
    a
}

/// Hamming window value at `i` over a frame.
fn hamming(i: usize) -> f32 {
    const SCALE: f32 = core::f32::consts::TAU / (FRAME_SAMPLES as f32 - 1.0);
    0.54 - 0.46 * math::cos(i as f32 * SCALE)
}

/// Five-tap smoothing, which keeps the fundamental and drops the fricative
/// energy that would otherwise confuse the pitch search.
fn smooth(input: &[f32; HISTORY], output: &mut [f32; HISTORY]) {
    for i in 0..HISTORY {
        let mut acc = 3.0 * input[i];
        let mut weight = 3.0;
        for (offset, w) in [(1usize, 2.0f32), (2, 1.0)] {
            if i >= offset {
                acc += w * input[i - offset];
                weight += w;
            }
            if i + offset < HISTORY {
                acc += w * input[i + offset];
                weight += w;
            }
        }
        output[i] = acc / weight;
    }
}

fn zero_crossing_rate(frame: &[f32]) -> f32 {
    let crossings = frame
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    crossings as f32 / (frame.len() - 1) as f32
}

fn frame_rms(frame: &[f32]) -> f32 {
    let energy: f32 = frame.iter().map(|s| s * s).sum();
    math::sqrt(energy / frame.len() as f32)
}
