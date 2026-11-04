// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Quantization and bit packing: parameters to seven bytes and back.
//!
//! Bit budget per 22.5 ms frame, following LPC-10e's published allocation:
//!
//! | Field | Bits |
//! |---|---|
//! | voicing | 1 |
//! | pitch period | 6 |
//! | residual gain | 5 |
//! | reflection coefficients 1-4 | 5 each |
//! | reflection coefficients 5-8 | 4 each |
//! | reflection coefficient 9 | 3 |
//! | reflection coefficient 10 | 2 |
//!
//! That is 53 bits, carried in 7 bytes. The spare 3 bits are written as
//! zero and ignored on read, which leaves room to say something later
//! without moving any existing field.
//!
//! The quantizer *tables* are this crate's own. The structure above is the
//! standard's; the levels are not, so a Pipit frame is not interchangeable
//! with a FIPS-137 frame.

use super::{FRAME_BYTES, ORDER, PITCH_MAX, PITCH_MIN, Params};
use crate::math;

/// Bits given to each reflection coefficient, lowest order first. Early
/// coefficients carry the formants that decide intelligibility, so they get
/// the resolution.
pub(super) const RC_BITS: [u32; ORDER] = [5, 5, 5, 5, 4, 4, 4, 4, 3, 2];

/// The first two coefficients are quantized as log area ratios, where equal
/// steps matter equally to the ear. This bounds that domain.
const LAR_LIMIT: f32 = 8.0;

/// Direct-quantized coefficients stay inside this, which keeps every
/// reconstructed filter stable.
const RC_LIMIT: f32 = 0.99;

/// Gain is logarithmic across this many powers of two, starting here. Index
/// zero is reserved for silence.
pub(super) const PITCH_BITS: u32 = 6;
pub(super) const GAIN_BITS: u32 = 5;
const GAIN_MIN_LOG2: f32 = -1.0;
const GAIN_MAX_LOG2: f32 = 14.0;

/// Log area ratio of a reflection coefficient.
fn to_lar(k: f32) -> f32 {
    let k = k.clamp(-0.999, 0.999);
    math::log2((1.0 + k) / (1.0 - k))
}

/// Back from a log area ratio. The result is always inside (-1, 1).
fn from_lar(lar: f32) -> f32 {
    let p = math::exp2(lar);
    (p - 1.0) / (p + 1.0)
}

/// Quantize `value` in `[lo, hi]` to `bits`, returning the code.
fn quantize(value: f32, lo: f32, hi: f32, bits: u32) -> u32 {
    let levels = (1u32 << bits) - 1;
    let normalized = (value - lo) / (hi - lo);
    let scaled = normalized * levels as f32 + 0.5;
    (scaled.clamp(0.0, levels as f32)) as u32
}

/// The inverse of [`quantize`].
fn dequantize(code: u32, lo: f32, hi: f32, bits: u32) -> f32 {
    let levels = (1u32 << bits) - 1;
    lo + (code.min(levels) as f32 / levels as f32) * (hi - lo)
}

/// Writes fields most significant bit first into an `N`-byte frame.
pub(super) struct BitWriter<const N: usize> {
    pub(super) bytes: [u8; N],
    position: usize,
}

impl<const N: usize> BitWriter<N> {
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; N],
            position: 0,
        }
    }

    pub(super) fn write(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            if (value >> i) & 1 == 1 {
                self.bytes[self.position / 8] |= 0x80 >> (self.position % 8);
            }
            self.position += 1;
        }
    }
}

/// Reads fields back in the same order.
pub(super) struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(super) fn read(&mut self, bits: u32) -> u32 {
        let mut value = 0;
        for _ in 0..bits {
            let byte = self.bytes[self.position / 8];
            let bit = (byte >> (7 - self.position % 8)) & 1;
            value = (value << 1) | bit as u32;
            self.position += 1;
        }
        value
    }
}


// ---------------------------------------------------------------------------
// Field codecs. Lifted out of `pack`/`unpack` so the half-rate superframe
// layout quantizes identically rather than growing a second set of tables.
// ---------------------------------------------------------------------------

/// Pitch is logarithmic: a semitone matters the same at the top of the range
/// as at the bottom.
pub(super) fn pitch_code(pitch: u8) -> u32 {
    let period = (pitch as f32).clamp(PITCH_MIN as f32, PITCH_MAX as f32);
    quantize(
        math::log2(period),
        math::log2(PITCH_MIN as f32),
        math::log2(PITCH_MAX as f32),
        PITCH_BITS,
    )
}

pub(super) fn pitch_from_code(code: u32) -> u8 {
    let period = math::exp2(dequantize(
        code,
        math::log2(PITCH_MIN as f32),
        math::log2(PITCH_MAX as f32),
        PITCH_BITS,
    ));
    (period + 0.5).clamp(PITCH_MIN as f32, PITCH_MAX as f32) as u8
}

/// Level zero means silence, so a real gain never quantizes below one.
pub(super) fn gain_code(gain: f32) -> u32 {
    if gain <= 0.0 {
        return 0;
    }
    quantize(math::log2(gain), GAIN_MIN_LOG2, GAIN_MAX_LOG2, GAIN_BITS).max(1)
}

pub(super) fn gain_from_code(code: u32) -> f32 {
    if code == 0 {
        0.0
    } else {
        math::exp2(dequantize(code, GAIN_MIN_LOG2, GAIN_MAX_LOG2, GAIN_BITS))
    }
}

/// Reflection coefficient `index`, quantized as a log area ratio for the
/// first two and directly for the rest.
pub(super) fn rc_code(index: usize, k: f32) -> u32 {
    let bits = RC_BITS[index];
    if index < 2 {
        quantize(to_lar(k), -LAR_LIMIT, LAR_LIMIT, bits)
    } else {
        quantize(k, -RC_LIMIT, RC_LIMIT, bits)
    }
}

pub(super) fn rc_from_code(index: usize, code: u32) -> f32 {
    let bits = RC_BITS[index];
    if index < 2 {
        from_lar(dequantize(code, -LAR_LIMIT, LAR_LIMIT, bits))
    } else {
        dequantize(code, -RC_LIMIT, RC_LIMIT, bits)
    }
}

/// Pack analysed parameters into one frame.
pub fn pack(params: &Params) -> [u8; FRAME_BYTES] {
    let mut writer = BitWriter::<FRAME_BYTES>::new();
    writer.write(u32::from(params.voiced), 1);
    writer.write(pitch_code(params.pitch), PITCH_BITS);
    writer.write(gain_code(params.gain), GAIN_BITS);
    for (i, &k) in params.rc.iter().enumerate() {
        writer.write(rc_code(i, k), RC_BITS[i]);
    }
    writer.bytes
}

/// Unpack one frame.
///
/// Every bit pattern is a valid frame: fields are clamped to their declared
/// ranges and reflection coefficients land strictly inside (-1, 1), so a
/// corrupted or hostile frame produces a stable filter rather than an
/// exploding one. `bytes` must be at least [`FRAME_BYTES`] long.
pub fn unpack(bytes: &[u8]) -> Params {
    debug_assert!(bytes.len() >= FRAME_BYTES);
    let mut reader = BitReader::new(bytes);
    let voiced = reader.read(1) == 1;
    let pitch = pitch_from_code(reader.read(PITCH_BITS));
    let gain = gain_from_code(reader.read(GAIN_BITS));
    let mut rc = [0.0f32; ORDER];
    for (i, slot) in rc.iter_mut().enumerate() {
        *slot = rc_from_code(i, reader.read(RC_BITS[i]));
    }
    Params {
        voiced,
        pitch,
        gain,
        rc,
    }
}
