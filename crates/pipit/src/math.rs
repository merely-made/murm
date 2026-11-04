// Copyright 2026 Mark AB (markik)
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The float math the vocoder needs, without `std` and without a dependency.
//!
//! `core` has no `sqrt`, `log2`, `exp2`, or `cos` for `f32`: those live in
//! `std` or in a libm crate. The vocoder needs exactly four, so they are
//! implemented here rather than taking a dependency for four functions.
//!
//! Accuracy is checked against `std` in the tests, which is the only reason
//! to trust them. None of these are general-purpose: they cover the ranges
//! speech coding actually uses and degrade gracefully outside them rather
//! than trapping.

/// Absolute value, by clearing the sign bit.
pub fn abs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// Largest integer not greater than `x`. Speech-range inputs only.
pub fn floor(x: f32) -> f32 {
    let truncated = x as i32 as f32;
    if truncated > x { truncated - 1.0 } else { truncated }
}

/// Square root by Newton-Raphson from an exponent-halving first guess.
///
/// Negative and zero inputs return zero: an energy is never negative here,
/// and returning NaN would propagate into the bitstream.
pub fn sqrt(x: f32) -> f32 {
    // NaN included: it must fall to the guard rather than through it.
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    // Halving the biased exponent lands within a factor of ~1.4, which four
    // Newton steps refine to f32 precision.
    let mut y = f32::from_bits((x.to_bits() + (127 << 23)) >> 1);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Base-2 logarithm: exponent from the bits, mantissa by series.
///
/// Non-positive inputs return a large negative value rather than infinity,
/// so a silent frame quantizes to the bottom of the gain range instead of
/// poisoning later arithmetic.
pub fn log2(x: f32) -> f32 {
    // NaN included: it must fall to the guard rather than through it.
    if x.is_nan() || x <= 0.0 {
        return -128.0;
    }
    let bits = x.to_bits();
    let exponent = (((bits >> 23) & 0xff) as i32 - 127) as f32;
    // Mantissa forced into [1, 2), where the atanh series converges fast.
    let m = f32::from_bits((bits & 0x007f_ffff) | (127 << 23));
    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let series = z * (1.0 + z2 * (1.0 / 3.0 + z2 * (1.0 / 5.0 + z2 * (1.0 / 7.0))));
    // 2 / ln(2), which is 2 log2(e).
    exponent + 2.0 * core::f32::consts::LOG2_E * series
}

/// Base-2 exponential: integer part by exponent bits, fraction by series.
pub fn exp2(x: f32) -> f32 {
    if x < -126.0 {
        return 0.0;
    }
    if x > 127.0 {
        return f32::MAX;
    }
    let whole = floor(x);
    let frac = x - whole;
    // Taylor of e^(f ln2) on [0, 1): coefficients are ln(2)^n / n!.
    const LN2: f32 = core::f32::consts::LN_2;
    let p = 1.0
        + frac
            * (LN2
                + frac
                    * (0.240_226_5
                        + frac
                            * (0.055_504_1
                                + frac * (0.009_618_1 + frac * (0.001_333_4 + frac * 0.000_154_0)))));
    let scale = f32::from_bits((((whole as i32) + 127) as u32) << 23);
    p * scale
}

/// Cosine by range reduction and a twelfth-order Taylor series.
pub fn cos(x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    const TAU: f32 = core::f32::consts::TAU;
    let mut x = x % TAU;
    if x > PI {
        x -= TAU;
    } else if x < -PI {
        x += TAU;
    }
    // cos is even, so only the magnitude matters from here.
    let x = abs(x);
    // Fold the second quadrant onto the first: a Taylor series centred at 0
    // is weakest near pi, and cos(x) = -cos(pi - x) keeps every evaluation
    // inside [0, pi/2] where it converges to well under f32 precision.
    let (x, sign) = if x > PI / 2.0 { (PI - x, -1.0) } else { (x, 1.0) };
    let x2 = x * x;
    let series = 1.0
        - x2 / 2.0
            * (1.0
                - x2 / 12.0
                    * (1.0
                        - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0 * (1.0 - x2 / 132.0)))));
    sign * series
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Every function here is only trustworthy insofar as it tracks `std`,
    /// so each test is a direct comparison across the range speech uses.
    #[test]
    fn sqrt_matches_std_across_the_energy_range() {
        let cases: Vec<f32> = (0..200)
            .map(|i| 1e-3 * 1.2f32.powi(i % 100))
            .chain([0.0, 1.0, 2.0, 1e9])
            .collect();
        for x in cases {
            let ours = sqrt(x);
            let theirs = x.sqrt();
            let tolerance = theirs.abs() * 1e-5 + 1e-9;
            assert!(
                (ours - theirs).abs() <= tolerance,
                "sqrt({x}): {ours} vs {theirs}"
            );
        }
        assert_eq!(sqrt(-1.0), 0.0, "negative energy must not become NaN");
        assert_eq!(sqrt(0.0), 0.0);
    }

    #[test]
    fn log2_matches_std_across_the_gain_range() {
        for i in 0..2000 {
            let x = 1e-4 * 1.01f32.powi(i);
            let ours = log2(x);
            let theirs = x.log2();
            assert!((ours - theirs).abs() < 1e-4, "log2({x}): {ours} vs {theirs}");
        }
        assert!(log2(0.0) < -100.0, "silence must not become -inf");
    }

    #[test]
    fn exp2_matches_std_and_inverts_log2() {
        for i in -300..300 {
            let x = i as f32 * 0.05;
            let ours = exp2(x);
            let theirs = x.exp2();
            assert!(
                (ours - theirs).abs() <= theirs.abs() * 1e-4,
                "exp2({x}): {ours} vs {theirs}"
            );
        }
        for i in 1..500 {
            let x = i as f32 * 7.3;
            let round_trip = exp2(log2(x));
            assert!((round_trip - x).abs() < x * 1e-3, "{x} -> {round_trip}");
        }
    }

    #[test]
    fn cos_matches_std_over_a_full_window() {
        for i in 0..1000 {
            let x = i as f32 * core::f32::consts::TAU / 500.0;
            let ours = cos(x);
            let theirs = x.cos();
            assert!((ours - theirs).abs() < 1e-4, "cos({x}): {ours} vs {theirs}");
        }
    }

    #[test]
    fn floor_and_abs_behave() {
        assert_eq!(floor(3.7), 3.0);
        assert_eq!(floor(-3.2), -4.0);
        assert_eq!(floor(5.0), 5.0);
        assert_eq!(abs(-2.5), 2.5);
        assert_eq!(abs(2.5), 2.5);
    }
}
