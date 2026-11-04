// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A vocoder cannot be tested by comparing waveforms. It throws the
//! waveform away and builds a new one that carries the same message, so
//! sample-by-sample error is expected to be enormous and says nothing.
//! These tests measure what the codec actually claims to preserve: the
//! spectral envelope, the pitch, the voiced/unvoiced decision, and the
//! energy envelope. Plus the property that matters over the air, which is
//! that no bit pattern can make the synthesiser misbehave.

use super::*;
use std::{vec, vec::Vec};

const PI: f32 = core::f32::consts::PI;

/// A two-pole resonator, the building block of a synthetic vowel.
fn resonate(x: &mut [f32], freq: f32, bandwidth: f32) {
    let fs = 8000.0;
    let r = (-PI * bandwidth / fs).exp();
    let theta = 2.0 * PI * freq / fs;
    let a1 = 2.0 * r * theta.cos();
    let a2 = -r * r;
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    for s in x.iter_mut() {
        let y = *s + a1 * y1 + a2 * y2;
        y2 = y1;
        y1 = y;
        *s = y;
    }
}

fn scale_to_rms(x: &mut [f32], target: f32) {
    let energy: f32 = x.iter().map(|s| s * s).sum();
    let rms = (energy / x.len() as f32).sqrt();
    if rms > 0.0 {
        let g = target / rms;
        for s in x.iter_mut() {
            *s *= g;
        }
    }
}

/// A synthetic vowel: a glottal pulse train through two formants. Voiced,
/// with a pitch we know exactly.
fn voiced(period: usize, len: usize, rms: f32) -> Vec<i16> {
    let mut buf = vec![0.0f32; len];
    for n in (0..len).step_by(period) {
        buf[n] = 1.0;
    }
    resonate(&mut buf, 700.0, 90.0);
    resonate(&mut buf, 1220.0, 110.0);
    scale_to_rms(&mut buf, rms);
    buf.iter().map(|s| *s as i16).collect()
}

/// Fricative-like noise: no periodicity, broad spectrum.
fn unvoiced(len: usize, rms: f32) -> Vec<i16> {
    let mut state = 0x1234_5678u32;
    let mut buf: Vec<f32> = (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect();
    resonate(&mut buf, 3000.0, 800.0);
    scale_to_rms(&mut buf, rms);
    buf.iter().map(|s| *s as i16).collect()
}

/// Log power spectrum of an all-pole filter, in dB, at 64 points.
fn log_spectrum(rc: &[f32; ORDER]) -> Vec<f32> {
    let a = super::analysis::reflection_to_predictor(rc);
    (0..64)
        .map(|i| {
            let w = PI * i as f32 / 64.0;
            let mut re = 1.0f32;
            let mut im = 0.0f32;
            for (j, &coefficient) in a.iter().enumerate().skip(1) {
                re -= coefficient * (w * j as f32).cos();
                im += coefficient * (w * j as f32).sin();
            }
            -10.0 * (re * re + im * im).max(1e-12).log10()
        })
        .collect()
}

/// Root-mean-square log spectral distortion in dB, the standard measure of
/// how well a vocoder preserved the vocal tract shape. Mean offset is
/// removed because overall level is carried separately by the gain.
fn spectral_distortion(a: &[f32; ORDER], b: &[f32; ORDER]) -> f32 {
    let (sa, sb) = (log_spectrum(a), log_spectrum(b));
    let mean_a: f32 = sa.iter().sum::<f32>() / sa.len() as f32;
    let mean_b: f32 = sb.iter().sum::<f32>() / sb.len() as f32;
    let sum: f32 = sa
        .iter()
        .zip(&sb)
        .map(|(x, y)| {
            let d = (x - mean_a) - (y - mean_b);
            d * d
        })
        .sum();
    (sum / sa.len() as f32).sqrt()
}

fn rms(x: &[i16]) -> f32 {
    let energy: f64 = x.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (energy / x.len() as f64).sqrt() as f32
}

/// Run PCM through encode and decode, returning the resynthesised audio.
fn round_trip(pcm: &[i16]) -> Vec<i16> {
    let mut encoder = Encoder::new();
    let mut decoder = Decoder::new();
    let mut out = Vec::new();
    let mut frame = [0u8; FRAME_BYTES];
    let mut decoded = [0i16; FRAME_SAMPLES];
    for chunk in pcm.chunks_exact(FRAME_SAMPLES) {
        encoder.encode_frame(chunk, &mut frame).unwrap();
        decoder.decode_frame(&frame, &mut decoded).unwrap();
        out.extend_from_slice(&decoded);
    }
    out
}

#[test]
fn frame_is_seven_bytes_at_the_declared_rate() {
    let pcm = voiced(64, FRAME_SAMPLES, 6000.0);
    let mut encoder = Encoder::new();
    let mut frame = [0u8; 16];
    assert_eq!(encoder.encode_frame(&pcm, &mut frame).unwrap(), 7);

    // 7 bytes per 22.5 ms.
    let bits_per_second = FRAME_BYTES as f32 * 8.0 * 8000.0 / FRAME_SAMPLES as f32;
    assert!(
        (bits_per_second - 2489.0).abs() < 1.0,
        "declared bitrate: {bits_per_second}"
    );
}

#[test]
fn pitch_is_recovered_for_known_fundamentals() {
    // 125 Hz, 160 Hz, and 80 Hz: a mid male voice, a higher one, and a low
    // one, spanning most of the coder's range.
    for period in [64usize, 50, 100] {
        let pcm = voiced(period, FRAME_SAMPLES * 6, 6000.0);
        let mut encoder = Encoder::new();
        // Skip the first frames: the analyser has no history yet.
        let mut measured = Vec::new();
        for chunk in pcm.chunks_exact(FRAME_SAMPLES) {
            measured.push(encoder.analyze(chunk).unwrap());
        }
        for params in &measured[2..] {
            assert!(params.voiced, "period {period} should read as voiced");
            let error = (params.pitch as i32 - period as i32).abs();
            assert!(
                error <= 2,
                "period {period}: measured {} (error {error})",
                params.pitch
            );
        }
    }
}

#[test]
fn pitch_survives_quantization() {
    for period in [64usize, 50, 100] {
        let pcm = voiced(period, FRAME_SAMPLES * 4, 6000.0);
        let mut encoder = Encoder::new();
        let mut frame = [0u8; FRAME_BYTES];
        for chunk in pcm.chunks_exact(FRAME_SAMPLES).skip(2) {
            encoder.encode_frame(chunk, &mut frame).unwrap();
            let decoded = super::quant::unpack(&frame);
            // Six bits over a 20..156 log range is about 3% per step.
            let ratio = decoded.pitch as f32 / period as f32;
            assert!(
                (0.95..1.05).contains(&ratio),
                "period {period} came back as {}",
                decoded.pitch
            );
        }
    }
}

#[test]
fn voicing_distinguishes_buzz_from_hiss_and_silence() {
    let mut encoder = Encoder::new();
    let voiced_pcm = voiced(64, FRAME_SAMPLES * 5, 6000.0);
    for chunk in voiced_pcm.chunks_exact(FRAME_SAMPLES).skip(2) {
        assert!(encoder.analyze(chunk).unwrap().voiced, "buzz is voiced");
    }

    let mut encoder = Encoder::new();
    let noise = unvoiced(FRAME_SAMPLES * 5, 6000.0);
    for chunk in noise.chunks_exact(FRAME_SAMPLES).skip(2) {
        assert!(!encoder.analyze(chunk).unwrap().voiced, "hiss is unvoiced");
    }

    let mut encoder = Encoder::new();
    let silence = [0i16; FRAME_SAMPLES * 3];
    for chunk in silence.chunks_exact(FRAME_SAMPLES) {
        let params = encoder.analyze(chunk).unwrap();
        assert!(!params.voiced, "silence is not voiced");
        assert_eq!(params.gain, 0.0);
    }
}

#[test]
fn spectral_envelope_survives_the_round_trip() {
    // The measure that matters: does the resynthesised audio have the same
    // vocal tract shape as the original? Compared through the analyser, in
    // the log spectral domain, which is how vocoders are graded.
    let pcm = voiced(64, FRAME_SAMPLES * 8, 6000.0);
    let decoded = round_trip(&pcm);

    let mut original_analyzer = Encoder::new();
    let mut decoded_analyzer = Encoder::new();
    let mut distortions = Vec::new();
    for (a, b) in pcm
        .chunks_exact(FRAME_SAMPLES)
        .zip(decoded.chunks_exact(FRAME_SAMPLES))
    {
        let pa = original_analyzer.analyze(a).unwrap();
        let pb = decoded_analyzer.analyze(b).unwrap();
        distortions.push(spectral_distortion(&pa.rc, &pb.rc));
    }

    // Skip the onset frames, where the decoder is ramping out of silence.
    let steady = &distortions[3..];
    let mean = steady.iter().sum::<f32>() / steady.len() as f32;
    // Measured 1.97 dB when written, tightly clustered frame to frame. The
    // threshold leaves headroom for tuning without letting a real
    // regression through: much above 3 dB and formants are moving.
    assert!(
        mean < 3.0,
        "mean spectral distortion {mean} dB over {steady:?}"
    );
}

#[test]
fn energy_envelope_is_tracked() {
    for level in [1000.0f32, 6000.0, 15000.0] {
        let pcm = voiced(64, FRAME_SAMPLES * 6, level);
        let decoded = round_trip(&pcm);
        // Compare steady-state frames only.
        let from = FRAME_SAMPLES * 3;
        let original = rms(&pcm[from..]);
        let output = rms(&decoded[from..]);
        let ratio = output / original;
        assert!(
            (0.35..3.0).contains(&ratio),
            "level {level}: {original} in, {output} out (ratio {ratio})"
        );
    }
}

#[test]
fn silence_decodes_to_silence() {
    let decoded = round_trip(&[0i16; FRAME_SAMPLES * 4]);
    assert!(
        decoded.iter().all(|s| *s == 0),
        "silent input must not hiss"
    );
}

#[test]
fn unvoiced_speech_resynthesises_as_noise_at_the_right_level() {
    let pcm = unvoiced(FRAME_SAMPLES * 6, 5000.0);
    let decoded = round_trip(&pcm);
    let from = FRAME_SAMPLES * 3;
    let ratio = rms(&decoded[from..]) / rms(&pcm[from..]);
    assert!((0.35..3.0).contains(&ratio), "unvoiced level ratio {ratio}");
}

#[test]
fn every_bit_pattern_is_a_safe_frame() {
    // The property that matters over the air. A frame arrives from a
    // stranger, possibly corrupted; it must never produce a filter that
    // explodes, a NaN, or a panic.
    let mut state = 0xdead_beefu32;
    let mut decoder = Decoder::new();
    let mut pcm = [0i16; FRAME_SAMPLES];
    for _ in 0..4000 {
        let mut frame = [0u8; FRAME_BYTES];
        for byte in &mut frame {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        decoder.decode_frame(&frame, &mut pcm).unwrap();
    }

    // All-ones and all-zeros are the patterns a broken link produces.
    for pattern in [0x00u8, 0xff] {
        for _ in 0..50 {
            decoder
                .decode_frame(&[pattern; FRAME_BYTES], &mut pcm)
                .unwrap();
        }
    }

    // The real proof is recovery: after all that, the same decoder, with
    // whatever state the garbage left in it, must still render real speech.
    // A filter that had diverged to NaN would emit silence forever, and one
    // stuck at the rails would emit saturation forever.
    let speech = voiced(64, FRAME_SAMPLES * 6, 6000.0);
    let mut encoder = Encoder::new();
    let mut frame = [0u8; FRAME_BYTES];
    let mut last = [0i16; FRAME_SAMPLES];
    for chunk in speech.chunks_exact(FRAME_SAMPLES) {
        encoder.encode_frame(chunk, &mut frame).unwrap();
        decoder.decode_frame(&frame, &mut last).unwrap();
    }
    let level = rms(&last);
    assert!(
        level > 500.0 && level < 20000.0,
        "decoder did not recover after hostile input: rms {level}"
    );
}

#[test]
fn unpacked_filters_are_always_stable() {
    let mut state = 0x0badc0deu32;
    for _ in 0..5000 {
        let mut frame = [0u8; FRAME_BYTES];
        for byte in &mut frame {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let params = super::quant::unpack(&frame);
        for k in params.rc {
            assert!(k.abs() < 1.0, "reflection coefficient {k} is unstable");
            assert!(k.is_finite());
        }
        assert!(params.gain.is_finite() && params.gain >= 0.0);
        assert!((PITCH_MIN as u8..=PITCH_MAX as u8).contains(&params.pitch));
    }
}

#[test]
fn a_lost_frame_costs_only_itself() {
    // Frames describe themselves, so a decoder that misses one recovers on
    // the next rather than desynchronising the way a waveform coder would.
    let pcm = voiced(64, FRAME_SAMPLES * 8, 6000.0);
    let mut encoder = Encoder::new();
    let mut frames = Vec::new();
    let mut frame = [0u8; FRAME_BYTES];
    for chunk in pcm.chunks_exact(FRAME_SAMPLES) {
        encoder.encode_frame(chunk, &mut frame).unwrap();
        frames.push(frame);
    }

    let mut complete = Decoder::new();
    let mut lossy = Decoder::new();
    let mut a = [0i16; FRAME_SAMPLES];
    let mut b = [0i16; FRAME_SAMPLES];
    for (i, frame) in frames.iter().enumerate() {
        complete.decode_frame(frame, &mut a).unwrap();
        // The lossy decoder never sees frame 4.
        if i != 4 {
            lossy.decode_frame(frame, &mut b).unwrap();
        }
    }
    // Two frames after the loss, both decoders are producing comparable
    // energy again.
    let ratio = rms(&b) / rms(&a);
    assert!(
        (0.5..2.0).contains(&ratio),
        "decoder should recover after a lost frame, ratio {ratio}"
    );
}

#[test]
fn encoding_is_deterministic() {
    let pcm = voiced(64, FRAME_SAMPLES * 3, 6000.0);
    let first = round_trip(&pcm);
    let second = round_trip(&pcm);
    assert_eq!(first, second);
}

#[test]
fn wrong_buffer_sizes_are_refused() {
    let mut encoder = Encoder::new();
    let mut out = [0u8; FRAME_BYTES];
    assert!(encoder.encode_frame(&[0i16; 100], &mut out).is_err());
    assert!(
        encoder
            .encode_frame(&[0i16; FRAME_SAMPLES], &mut [0u8; 3])
            .is_err()
    );

    let mut decoder = Decoder::new();
    let mut pcm = [0i16; FRAME_SAMPLES];
    assert!(decoder.decode_frame(&[0u8; 3], &mut pcm).is_err());
    assert!(
        decoder
            .decode_frame(&[0u8; FRAME_BYTES], &mut [0i16; 10])
            .is_err()
    );
}

// Half-rate superframe tests live in `tests/half_rate.rs`, keeping this
// file under the 600-line ceiling.
mod half_rate;
