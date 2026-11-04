// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

fn header() -> ClipHeader {
    ClipHeader {
        codec: Codec::ImaAdpcm,
        mode: 0,
        sample_rate: 8_000,
        frame_samples: 160,
        sample_count: 8_000,
    }
}

#[test]
fn header_round_trips_through_bytes() {
    let original = header();
    let mut bytes = [0u8; HEADER_LEN];
    assert_eq!(original.write(&mut bytes).unwrap(), HEADER_LEN);
    assert_eq!(ClipHeader::parse(&bytes).unwrap(), original);
}

#[test]
fn header_reports_geometry() {
    let header = header();
    assert_eq!(header.frame_count(), 50);
    assert_eq!(header.duration_ms(), 1000);
    assert_eq!(header.encoded_len(), HEADER_LEN + 50 * 83);

    // A trailing partial frame still counts as a frame.
    let partial = ClipHeader {
        sample_count: 8_001,
        ..header
    };
    assert_eq!(partial.frame_count(), 51);
}

#[test]
fn malformed_headers_are_refused() {
    let mut bytes = [0u8; HEADER_LEN];
    header().write(&mut bytes).unwrap();

    assert!(matches!(
        ClipHeader::parse(&bytes[..HEADER_LEN - 1]),
        Err(Error::Truncated)
    ));

    let mut bad_magic = bytes;
    bad_magic[0] = b'X';
    assert!(matches!(ClipHeader::parse(&bad_magic), Err(Error::BadMagic)));

    let mut bad_version = bytes;
    bad_version[5] = 9;
    assert!(matches!(
        ClipHeader::parse(&bad_version),
        Err(Error::UnsupportedVersion(9))
    ));

    let mut bad_codec = bytes;
    bad_codec[6] = 200;
    assert!(matches!(
        ClipHeader::parse(&bad_codec),
        Err(Error::UnknownCodec(200))
    ));

    let mut bad_mode = bytes;
    bad_mode[7] = 3;
    assert!(matches!(
        ClipHeader::parse(&bad_mode),
        Err(Error::UnsupportedMode(3))
    ));

    let mut zero_rate = bytes;
    zero_rate[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(
        ClipHeader::parse(&zero_rate),
        Err(Error::InvalidHeader)
    ));

    // A frame length past the cap is refused rather than sized into a buffer.
    let mut huge_frame = bytes;
    huge_frame[12..14].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(matches!(
        ClipHeader::parse(&huge_frame),
        Err(Error::InvalidHeader)
    ));
}

#[cfg(feature = "alloc")]
mod clips {
    use super::*;

    fn speech(len: usize) -> alloc::vec::Vec<i16> {
        (0..len)
            .map(|i| {
                let t = i as f32 / 8000.0;
                let envelope = 0.3 + 0.7 * (t * 6.0).sin().abs();
                let v = (t * 220.0 * core::f32::consts::TAU).sin() * 0.6
                    + (t * 440.0 * core::f32::consts::TAU).sin() * 0.25;
                (v * envelope * 12000.0) as i16
            })
            .collect()
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
        (10.0 * (sig / noise).log10()) as f32
    }

    #[test]
    fn clip_round_trips_with_speech_band_snr() {
        let pcm = speech(8_000);
        let clip = encode_clip(&pcm, ClipParams::default()).unwrap();

        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.sample_count, 8_000);
        assert_eq!(header.duration_ms(), 1000);
        assert_eq!(decoded.len(), pcm.len());
        // A second of audio dwarfs the 10 ms cold-start transient, so a whole
        // clip lands near the codec's steady state.
        let snr = snr_db(&pcm, &decoded);
        assert!(snr > 30.0, "clip round trip should clear 30 dB, got {snr}");
    }

    #[test]
    fn clip_size_matches_the_declared_bitrate() {
        // One second of 8 kHz speech at 4 bits per sample, plus 3 header
        // bytes per 20 ms frame: the airtime number the scoping doc quotes.
        let clip = encode_clip(&speech(8_000), ClipParams::default()).unwrap();
        assert_eq!(clip.len(), HEADER_LEN + 50 * 83);
        assert!(clip.len() < 4_300, "~33 kbps, not more: {}", clip.len());
    }

    #[test]
    fn partial_final_frame_is_padded_and_trimmed() {
        // 170 samples is one full frame plus ten.
        let pcm = speech(170);
        let clip = encode_clip(&pcm, ClipParams::default()).unwrap();
        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.frame_count(), 2);
        assert_eq!(decoded.len(), 170, "padding must not survive decode");
    }

    #[test]
    fn empty_input_produces_a_headerless_clip() {
        let clip = encode_clip(&[], ClipParams::default()).unwrap();
        assert_eq!(clip.len(), HEADER_LEN);
        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.sample_count, 0);
        assert_eq!(header.duration_ms(), 0);
        assert!(decoded.is_empty());
    }

    #[test]
    fn truncated_clip_is_refused_not_allocated() {
        let clip = encode_clip(&speech(8_000), ClipParams::default()).unwrap();
        // A header promising 50 frames with only a few present must fail,
        // rather than sizing a buffer from the declared count.
        assert!(matches!(
            decode_clip(&clip[..HEADER_LEN + 83]),
            Err(Error::Truncated)
        ));
    }

    #[test]
    fn vocoder_clip_round_trips_and_fits_a_lora_frame() {
        // The number Rung 1 exists for: ten seconds of speech small enough
        // to carry over LoRa, where the same memo in ADPCM is 40 KB.
        let pcm = speech(80_000);
        let clip = encode_clip(&pcm, ClipParams::lpc10()).unwrap();

        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.codec, Codec::Lpc10);
        assert_eq!(header.duration_ms(), 10_000);
        assert_eq!(decoded.len(), pcm.len());
        assert!(
            clip.len() < 3_200,
            "ten seconds should fit in ~3 KB, got {}",
            clip.len()
        );

        // Resynthesised speech, so the level should be in the same country
        // as the original even though the waveform is not.
        let energy = |x: &[i16]| {
            let sum: f64 = x.iter().map(|s| (*s as f64) * (*s as f64)).sum();
            (sum / x.len() as f64).sqrt()
        };
        let ratio = energy(&decoded) / energy(&pcm);
        assert!((0.2..5.0).contains(&ratio), "level ratio {ratio}");
    }

    #[test]
    fn vocoder_refuses_a_frame_length_it_cannot_code() {
        let params = ClipParams {
            frame_samples: 160,
            ..ClipParams::lpc10()
        };
        assert!(matches!(
            encode_clip(&speech(1_000), params),
            Err(Error::InvalidHeader)
        ));
    }

    #[test]
    fn vocoder_partial_final_frame_is_trimmed() {
        // 200 samples is one 180-sample frame plus twenty.
        let pcm = speech(200);
        let clip = encode_clip(&pcm, ClipParams::lpc10()).unwrap();
        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.frame_count(), 2);
        assert_eq!(decoded.len(), 200);
    }


    #[test]
    fn half_rate_clip_round_trips_at_a_third_less_airtime() {
        // The number Rung 2 exists for: the same ten seconds that costs
        // 40 KB as ADPCM and 3.1 KB through the vocoder.
        let pcm = speech(80_000);
        let full = encode_clip(&pcm, ClipParams::lpc10()).unwrap();
        let clip = encode_clip(&pcm, ClipParams::lpc10_half()).unwrap();

        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.codec, Codec::Lpc10Half);
        assert_eq!(header.duration_ms(), 10_000);
        assert_eq!(decoded.len(), pcm.len());
        assert!(clip.len() < 2_100, "ten seconds in ~2 KB, got {}", clip.len());

        let saving = 1.0 - clip.len() as f32 / full.len() as f32;
        assert!(saving > 0.33, "expected about a third off, got {saving}");

        let energy = |x: &[i16]| {
            let sum: f64 = x.iter().map(|s| (*s as f64) * (*s as f64)).sum();
            (sum / x.len() as f64).sqrt()
        };
        let ratio = energy(&decoded) / energy(&pcm);
        assert!((0.2..5.0).contains(&ratio), "level ratio {ratio}");
    }

    #[test]
    fn half_rate_refuses_a_frame_length_it_cannot_code() {
        let params = ClipParams {
            frame_samples: 180,
            ..ClipParams::lpc10_half()
        };
        assert!(matches!(
            encode_clip(&speech(1_000), params),
            Err(Error::InvalidHeader)
        ));
    }

    #[test]
    fn each_codec_keeps_its_own_identifier() {
        for (params, codec, id) in [
            (ClipParams::adpcm(), Codec::ImaAdpcm, 1u8),
            (ClipParams::lpc10(), Codec::Lpc10, 2),
            (ClipParams::lpc10_half(), Codec::Lpc10Half, 3),
        ] {
            let clip = encode_clip(&speech(4_000), params).unwrap();
            assert_eq!(clip[6], id, "codec id byte");
            let (header, _) = decode_clip(&clip).unwrap();
            assert_eq!(header.codec, codec);
        }
    }

    #[test]
    fn frame_geometry_is_configurable() {
        let params = ClipParams {
            frame_samples: 320,
            ..ClipParams::default()
        };
        let pcm = speech(1_000);
        let clip = encode_clip(&pcm, params).unwrap();
        let (header, decoded) = decode_clip(&clip).unwrap();
        assert_eq!(header.frame_samples, 320);
        assert_eq!(decoded.len(), 1_000);
    }
}
