// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Half-rate superframe mode. Shares the parent module's signal
//! generators and spectral-distortion measure, so the two rates are
//! judged by exactly the same instrument.

use super::*;
use crate::lpc10::half::{self, SUPERFRAME_BYTES, SUPERFRAME_SAMPLES};

fn round_trip_half(pcm: &[i16]) -> Vec<i16> {
    let mut encoder = half::Encoder::new();
    let mut decoder = half::Decoder::new();
    let mut out = Vec::new();
    let mut frame = [0u8; SUPERFRAME_BYTES];
    let mut decoded = [0i16; SUPERFRAME_SAMPLES];
    for chunk in pcm.chunks_exact(SUPERFRAME_SAMPLES) {
        encoder.encode_superframe(chunk, &mut frame).unwrap();
        decoder.decode_superframe(&frame, &mut decoded).unwrap();
        out.extend_from_slice(&decoded);
    }
    out
}

#[test]
fn a_superframe_is_nine_bytes_at_sixteen_hundred_bits_per_second() {
    let pcm = voiced(64, SUPERFRAME_SAMPLES, 6000.0);
    let mut encoder = half::Encoder::new();
    let mut frame = [0u8; 16];
    assert_eq!(encoder.encode_superframe(&pcm, &mut frame).unwrap(), 9);

    let bits_per_second = SUPERFRAME_BYTES as f32 * 8.0 * 8000.0 / SUPERFRAME_SAMPLES as f32;
    assert!(
        (bits_per_second - 1600.0).abs() < 1.0,
        "declared bitrate: {bits_per_second}"
    );

    // The whole point: a third less airtime than full rate.
    let full = FRAME_BYTES as f32 * 8.0 * 8000.0 / FRAME_SAMPLES as f32;
    let saving = 1.0 - bits_per_second / full;
    assert!(saving > 0.35, "expected a third off, got {saving}");
}

#[test]
fn pitch_and_voicing_are_untouched_by_halving_the_rate() {
    // Both are still sent per frame, so they should be as good as full
    // rate. Only the spectrum was thinned.
    for period in [64usize, 50, 100] {
        let pcm = voiced(period, SUPERFRAME_SAMPLES * 4, 6000.0);
        let mut encoder = half::Encoder::new();
        let mut frame = [0u8; SUPERFRAME_BYTES];
        for chunk in pcm.chunks_exact(SUPERFRAME_SAMPLES).skip(1) {
            encoder.encode_superframe(chunk, &mut frame).unwrap();
            let (first, second) = crate::lpc10::half::tests_unpack(&frame);
            for params in [first, second] {
                assert!(params.voiced, "period {period} should read as voiced");
                let ratio = params.pitch as f32 / period as f32;
                assert!(
                    (0.95..1.05).contains(&ratio),
                    "period {period} came back as {}",
                    params.pitch
                );
            }
        }
    }
}

#[test]
fn the_interpolated_spectrum_is_always_stable() {
    // The first frame's filter is never transmitted, so its stability
    // rests on interpolation staying inside (-1, 1). Hostile input is
    // the case that matters.
    let mut state = 0x51ed_c0deu32;
    let mut decoder = half::Decoder::new();
    let mut pcm = [0i16; SUPERFRAME_SAMPLES];
    for _ in 0..3000 {
        let mut frame = [0u8; SUPERFRAME_BYTES];
        for byte in &mut frame {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        decoder.decode_superframe(&frame, &mut pcm).unwrap();
    }

    // And it still renders speech afterwards.
    let speech = voiced(64, SUPERFRAME_SAMPLES * 4, 6000.0);
    let mut encoder = half::Encoder::new();
    let mut frame = [0u8; SUPERFRAME_BYTES];
    let mut last = [0i16; SUPERFRAME_SAMPLES];
    for chunk in speech.chunks_exact(SUPERFRAME_SAMPLES) {
        encoder.encode_superframe(chunk, &mut frame).unwrap();
        decoder.decode_superframe(&frame, &mut last).unwrap();
    }
    let level = rms(&last);
    assert!(level > 500.0 && level < 20000.0, "did not recover: {level}");
}

#[test]
fn steady_speech_costs_little_spectral_accuracy() {
    // On sustained sound the untransmitted spectrum is nearly the same as
    // its neighbours, so interpolation should cost very little. This is
    // the case half rate is designed for.
    let pcm = voiced(64, SUPERFRAME_SAMPLES * 6, 6000.0);
    let full = round_trip(&pcm);
    let half = round_trip_half(&pcm);

    let mut a_an = Encoder::new();
    let mut b_an = Encoder::new();
    let mut c_an = Encoder::new();
    let mut full_distortion = Vec::new();
    let mut half_distortion = Vec::new();
    for ((orig, f), h) in pcm
        .chunks_exact(FRAME_SAMPLES)
        .zip(full.chunks_exact(FRAME_SAMPLES))
        .zip(half.chunks_exact(FRAME_SAMPLES))
    {
        let po = a_an.analyze(orig).unwrap();
        full_distortion.push(spectral_distortion(&po.rc, &b_an.analyze(f).unwrap().rc));
        half_distortion.push(spectral_distortion(&po.rc, &c_an.analyze(h).unwrap().rc));
    }
    let mean = |v: &[f32]| v[4..].iter().sum::<f32>() / v[4..].len() as f32;
    let (full_mean, half_mean) = (mean(&full_distortion), mean(&half_distortion));
    assert!(
        half_mean < full_mean + 1.5,
        "half rate should stay close on steady speech: \
         {full_mean} dB full vs {half_mean} dB half"
    );
}


/// A tract that sweeps hard on a five-frame cycle: fast enough that
/// interpolation has real work to do, and on an odd period so it cannot
/// sit in lockstep with the two-frame superframe.
fn sweeping_vowel(frames: usize, level: f32) -> Vec<i16> {
    let mut out = Vec::new();
    let mut phase = 0usize;
    for frame in 0..frames {
        let mut buf = vec![0.0f32; FRAME_SAMPLES];
        for (n, slot) in buf.iter_mut().enumerate() {
            if (phase + n).is_multiple_of(64) {
                *slot = 1.0;
            }
        }
        phase += FRAME_SAMPLES;
        // Triangle sweep, period five frames.
        let cycle = (frame % 5) as f32 / 5.0;
        let t = if cycle < 0.5 { cycle * 2.0 } else { 2.0 - cycle * 2.0 };
        resonate(&mut buf, 700.0 + t * (350.0 - 700.0), 90.0);
        resonate(&mut buf, 1220.0 + t * (2300.0 - 1220.0), 120.0);
        scale_to_rms(&mut buf, level);
        out.extend(buf.iter().map(|s| *s as i16));
    }
    out
}

#[test]
fn a_fast_moving_tract_stays_in_range_of_full_rate() {
    // The counterweight to the steady-speech test, comparing full and
    // half on *the same signal*. Comparing across different signals
    // would conflate how hard a spectrum is to quantize with what
    // interpolation costs, which is a mistake this test used to make.
    //
    // The direction is not asserted either: half rate samples the
    // spectrum on even frames only, so articulation that moves in step
    // with the superframe can be flattered by that sampling. An earlier
    // version alternated vowels every frame and had half rate "winning"
    // purely from the lock. What is asserted is that half rate stays in
    // the same range as full rate when the tract is genuinely moving.
    let pcm = sweeping_vowel(40, 6000.0);
    let usable = pcm.len() / SUPERFRAME_SAMPLES * SUPERFRAME_SAMPLES;
    let full = round_trip(&pcm[..usable]);
    let half = round_trip_half(&pcm[..usable]);

    let mut a_an = Encoder::new();
    let mut b_an = Encoder::new();
    let mut c_an = Encoder::new();
    let (mut full_d, mut half_d) = (Vec::new(), Vec::new());
    for ((orig, f), h) in pcm[..usable]
        .chunks_exact(FRAME_SAMPLES)
        .zip(full.chunks_exact(FRAME_SAMPLES))
        .zip(half.chunks_exact(FRAME_SAMPLES))
    {
        let po = a_an.analyze(orig).unwrap();
        full_d.push(spectral_distortion(&po.rc, &b_an.analyze(f).unwrap().rc));
        half_d.push(spectral_distortion(&po.rc, &c_an.analyze(h).unwrap().rc));
    }
    let mean = |v: &[f32]| v[4..].iter().sum::<f32>() / v[4..].len() as f32;
    let (full_mean, half_mean) = (mean(&full_d), mean(&half_d));

    assert!(
        half_mean < full_mean * 1.6,
        "half rate must stay in range of full rate on a moving tract:              {full_mean} dB full vs {half_mean} dB half"
    );
}

#[test]
fn silence_still_decodes_to_silence() {
    let decoded = round_trip_half(&[0i16; SUPERFRAME_SAMPLES * 3]);
    assert!(decoded.iter().all(|s| *s == 0));
}

#[test]
fn wrong_buffer_sizes_are_refused() {
    let mut encoder = half::Encoder::new();
    let mut out = [0u8; SUPERFRAME_BYTES];
    assert!(encoder.encode_superframe(&[0i16; 100], &mut out).is_err());
    assert!(
        encoder
            .encode_superframe(&[0i16; SUPERFRAME_SAMPLES], &mut [0u8; 4])
            .is_err()
    );
    let mut decoder = half::Decoder::new();
    let mut pcm = [0i16; SUPERFRAME_SAMPLES];
    assert!(decoder.decode_superframe(&[0u8; 4], &mut pcm).is_err());
    assert!(
        decoder
            .decode_superframe(&[0u8; SUPERFRAME_BYTES], &mut [0i16; 10])
            .is_err()
    );
}
