// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! IMA ADPCM: 4 bits per sample, framed so each frame decodes alone.
//!
//! The nibble arithmetic is the standard IMA/DVI algorithm (the 89-entry step
//! table and 16-entry index table below). The *framing* is Pipit's own and is
//! deliberately not IMA WAV's block layout, so nothing here claims IMA WAV
//! file compatibility.
//!
//! Each frame carries the predictor state it starts from, which is what makes
//! a frame independently decodable: a frame lost in a call costs its own
//! samples and nothing after it, and a drop can be decoded from any frame
//! boundary. The encoder still runs its state continuously across frames, so
//! the only cost of framing is the 3-byte header.

use crate::Error;

/// Bytes of predictor state at the head of every frame: `i16` predictor
/// (little-endian) then the step index.
pub const FRAME_HEADER_LEN: usize = 3;

/// Largest step index the tables define.
const MAX_INDEX: u8 = 88;

/// IMA step sizes, indexed by the running step index.
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// How each code moves the step index. Symmetric across the sign bit.
const INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// Encoded length of a frame holding `frame_samples` samples.
pub const fn frame_encoded_len(frame_samples: usize) -> usize {
    FRAME_HEADER_LEN + frame_samples.div_ceil(2)
}

/// The predictor and step index the coder carries between samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    predictor: i16,
    index: u8,
}

impl State {
    fn step(self) -> i32 {
        // `index` is clamped on every update and validated on parse, so this
        // index is always in range.
        STEP_TABLE[self.index as usize]
    }

    fn advance_index(&mut self, code: u8) {
        let next = self.index as i32 + INDEX_TABLE[(code & 0x0f) as usize] as i32;
        self.index = next.clamp(0, MAX_INDEX as i32) as u8;
    }

    fn set_predictor(&mut self, value: i32) {
        self.predictor = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }

    /// Encode one sample, moving the state exactly as the decoder will.
    fn encode_sample(&mut self, sample: i16) -> u8 {
        let mut step = self.step();
        let mut diff = sample as i32 - self.predictor as i32;
        let sign: u8 = if diff < 0 { 8 } else { 0 };
        if diff < 0 {
            diff = -diff;
        }

        // Three greedy bits, largest step first; `vpdiff` accumulates the
        // reconstruction the decoder will compute from the same code.
        let mut code: u8 = 0;
        let mut vpdiff = step >> 3;
        if diff >= step {
            code |= 4;
            diff -= step;
            vpdiff += step;
        }
        step >>= 1;
        if diff >= step {
            code |= 2;
            diff -= step;
            vpdiff += step;
        }
        step >>= 1;
        if diff >= step {
            code |= 1;
            vpdiff += step;
        }

        let predictor = self.predictor as i32;
        self.set_predictor(if sign != 0 {
            predictor - vpdiff
        } else {
            predictor + vpdiff
        });
        let code = code | sign;
        self.advance_index(code);
        code
    }

    /// Decode one code, returning the reconstructed sample.
    fn decode_sample(&mut self, code: u8) -> i16 {
        let step = self.step();
        let mut vpdiff = step >> 3;
        if code & 4 != 0 {
            vpdiff += step;
        }
        if code & 2 != 0 {
            vpdiff += step >> 1;
        }
        if code & 1 != 0 {
            vpdiff += step >> 2;
        }

        let predictor = self.predictor as i32;
        self.set_predictor(if code & 8 != 0 {
            predictor - vpdiff
        } else {
            predictor + vpdiff
        });
        self.advance_index(code);
        self.predictor
    }
}

/// Streaming encoder. Carries predictor state across frames; stamps each
/// frame with the state it began from.
///
/// This is the frame-level API a live call drives directly, without the clip
/// container.
#[derive(Clone, Copy, Debug, Default)]
pub struct Encoder {
    state: State,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode `pcm` into `out`, returning the bytes written.
    ///
    /// `out` must hold [`frame_encoded_len`] bytes for `pcm.len()` samples.
    /// An odd sample count leaves the final high nibble zero.
    pub fn encode_frame(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, Error> {
        let needed = frame_encoded_len(pcm.len());
        if out.len() < needed {
            return Err(Error::ShortBuffer);
        }

        // The header is the state *before* this frame, so a decoder needs
        // nothing that came earlier.
        out[0..2].copy_from_slice(&self.state.predictor.to_le_bytes());
        out[2] = self.state.index;

        for (i, chunk) in pcm.chunks(2).enumerate() {
            let low = self.state.encode_sample(chunk[0]);
            let high = match chunk.get(1) {
                Some(&sample) => self.state.encode_sample(sample),
                None => 0,
            };
            out[FRAME_HEADER_LEN + i] = low | (high << 4);
        }
        Ok(needed)
    }
}

/// Decode one frame into `pcm`, returning the samples written.
///
/// Stateless by construction: the frame carries its own starting state, so
/// frames may be decoded out of order, skipped, or dropped. `pcm.len()` names
/// how many samples the caller wants and must not exceed what the frame
/// holds.
pub fn decode_frame(frame: &[u8], pcm: &mut [i16]) -> Result<usize, Error> {
    if frame.len() < FRAME_HEADER_LEN {
        return Err(Error::Truncated);
    }
    let index = frame[2];
    if index > MAX_INDEX {
        return Err(Error::InvalidState);
    }
    let mut state = State {
        predictor: i16::from_le_bytes([frame[0], frame[1]]),
        index,
    };

    let payload = &frame[FRAME_HEADER_LEN..];
    if pcm.len() > payload.len() * 2 {
        return Err(Error::Truncated);
    }
    for (i, sample) in pcm.iter_mut().enumerate() {
        let byte = payload[i / 2];
        let code = if i % 2 == 0 { byte & 0x0f } else { byte >> 4 };
        *sample = state.decode_sample(code);
    }
    Ok(pcm.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A speech-ish test signal: a voiced-sounding fundamental plus a
    /// harmonic, amplitude-enveloped so the coder's step adaptation is
    /// actually exercised in both directions.
    fn signal(len: usize) -> [i16; 512] {
        assert!(len <= 512);
        let mut out = [0i16; 512];
        for (i, s) in out.iter_mut().enumerate().take(len) {
            let t = i as f32 / 8000.0;
            let envelope = 0.3 + 0.7 * (t * 6.0).sin().abs();
            let v = (t * 220.0 * core::f32::consts::TAU).sin() * 0.6
                + (t * 440.0 * core::f32::consts::TAU).sin() * 0.25;
            *s = (v * envelope * 12000.0) as i16;
        }
        out
    }

    fn snr_db(original: &[i16], decoded: &[i16]) -> f32 {
        let mut sig = 0.0f64;
        let mut noise = 0.0f64;
        for (o, d) in original.iter().zip(decoded) {
            let o = *o as f64;
            let e = o - *d as f64;
            sig += o * o;
            noise += e * e;
        }
        if noise == 0.0 {
            return f32::INFINITY;
        }
        (10.0 * (sig / noise).log10()) as f32
    }

    #[test]
    fn frame_encoded_len_packs_two_samples_per_byte() {
        assert_eq!(frame_encoded_len(0), 3);
        assert_eq!(frame_encoded_len(1), 4);
        assert_eq!(frame_encoded_len(2), 4);
        assert_eq!(frame_encoded_len(160), 83);
    }

    #[test]
    fn round_trip_reaches_steady_state_snr() {
        let pcm = signal(320);
        let mut encoder = Encoder::new();
        let mut out = [0u8; 256];
        let written = encoder.encode_frame(&pcm[..320], &mut out).unwrap();
        assert_eq!(written, frame_encoded_len(320));

        let mut decoded = [0i16; 320];
        decode_frame(&out[..written], &mut decoded).unwrap();
        let steady = snr_db(&pcm[80..320], &decoded[80..]);
        assert!(steady > 30.0, "steady-state SNR should clear 30 dB: {steady}");
    }

    #[test]
    fn cold_start_converges_within_ten_milliseconds() {
        // The step index starts at 0 (step size 7) and has to climb to the
        // signal's amplitude, so a coder starting cold is quiet-then-correct
        // rather than wrong. Measured, not assumed: the first 5 ms sit near
        // 8 dB and the coder is at full quality by 10 ms. Anything that
        // regresses this convergence is a real defect, so it is pinned here.
        let pcm = signal(320);
        let mut encoder = Encoder::new();
        let mut out = [0u8; 256];
        let written = encoder.encode_frame(&pcm[..320], &mut out).unwrap();
        let mut decoded = [0i16; 320];
        decode_frame(&out[..written], &mut decoded).unwrap();

        let cold = snr_db(&pcm[..40], &decoded[..40]);
        let warm = snr_db(&pcm[80..320], &decoded[80..]);
        assert!(cold < warm, "cold start must be the worse stretch");
        assert!(warm - cold > 15.0, "convergence should be visible: {cold} -> {warm}");
    }

    #[test]
    fn an_isolated_frame_decodes_at_full_quality() {
        // The claim the framing exists to make: a receiver that lost every
        // earlier frame decodes this one as well as a receiver that got them
        // all, because the frame carries the state it starts from.
        let pcm = signal(480);
        let mut encoder = Encoder::new();
        let mut frames = [[0u8; 83]; 3];
        for (i, frame) in frames.iter_mut().enumerate() {
            encoder
                .encode_frame(&pcm[i * 160..(i + 1) * 160], frame)
                .unwrap();
        }

        let mut whole_stream = [0i16; 480];
        for (i, frame) in frames.iter().enumerate() {
            decode_frame(frame, &mut whole_stream[i * 160..(i + 1) * 160]).unwrap();
        }
        let continuous = snr_db(&pcm[320..480], &whole_stream[320..]);

        let mut alone = [0i16; 160];
        decode_frame(&frames[2], &mut alone).unwrap();
        let isolated = snr_db(&pcm[320..480], &alone);

        assert!(isolated > 30.0, "isolated frame quality: {isolated} dB");
        assert!(
            (continuous - isolated).abs() < 0.5,
            "loss of earlier frames must not degrade this one: \
             {continuous} dB continuous vs {isolated} dB isolated"
        );
    }

    #[test]
    fn odd_sample_count_round_trips() {
        let pcm = signal(7);
        let mut encoder = Encoder::new();
        let mut out = [0u8; 16];
        let written = encoder.encode_frame(&pcm[..7], &mut out).unwrap();
        assert_eq!(written, frame_encoded_len(7));
        let mut decoded = [0i16; 7];
        assert_eq!(decode_frame(&out[..written], &mut decoded).unwrap(), 7);
    }

    #[test]
    fn short_output_buffer_is_refused() {
        let mut encoder = Encoder::new();
        let mut out = [0u8; 4];
        assert!(matches!(
            encoder.encode_frame(&[0i16; 160], &mut out),
            Err(Error::ShortBuffer)
        ));
    }

    #[test]
    fn hostile_frames_error_rather_than_panic() {
        let mut pcm = [0i16; 8];
        assert!(matches!(decode_frame(&[], &mut pcm), Err(Error::Truncated)));
        assert!(matches!(
            decode_frame(&[0, 0], &mut pcm),
            Err(Error::Truncated)
        ));
        // Step index past the end of the table.
        assert!(matches!(
            decode_frame(&[0, 0, 200, 0, 0, 0, 0], &mut pcm),
            Err(Error::InvalidState)
        ));
        // Header present, payload too short for the requested samples.
        assert!(matches!(
            decode_frame(&[0, 0, 0, 0x11], &mut pcm),
            Err(Error::Truncated)
        ));
    }

    #[test]
    fn extreme_input_stays_in_range() {
        let pcm = [i16::MAX, i16::MIN, i16::MAX, i16::MIN, 0, i16::MAX];
        let mut encoder = Encoder::new();
        let mut out = [0u8; 16];
        let written = encoder.encode_frame(&pcm, &mut out).unwrap();
        let mut decoded = [0i16; 6];
        // Clamping is the only guarantee at the rails; no panic, no wrap.
        decode_frame(&out[..written], &mut decoded).unwrap();
    }
}
