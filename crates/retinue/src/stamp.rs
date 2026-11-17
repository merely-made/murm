//! Reticulum's quantized proof-of-work costs and digest thresholds.
//!
//! Adapted from RNS 1.6.0 `RNS/Stamps.py`, Copyright (c) 2016-2026 Mark Qvist,
//! under the Reticulum License reproduced in this crate's LICENSE. The integer
//! lookup table is a protocol constant: deriving it with platform logarithms would
//! change acceptance at exact thresholds.

use sha2::{Digest, Sha256};

/// A finite stamp cost quantized to 1/256, clamped to stock's global cap of 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StampCost(u16);

impl StampCost {
    pub const SCALE: u16 = 256;
    pub const MAX: Self = Self(100 * Self::SCALE);
    pub const ZERO: Self = Self(0);

    pub const fn from_fp(value: u16) -> Self {
        Self(if value > Self::MAX.0 {
            Self::MAX.0
        } else {
            value
        })
    }

    /// Stock's half-up quantization. Non-finite inputs are refused.
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let scaled = (value * f64::from(Self::SCALE) + 0.5) as u16;
        Some(Self::from_fp(scaled))
    }

    pub const fn from_integer(value: u16) -> Self {
        Self::from_fp(if value > 100 {
            Self::MAX.0
        } else {
            value * Self::SCALE
        })
    }
    pub const fn floor(self) -> u16 {
        self.0 / Self::SCALE
    }
    pub const fn ceil(self) -> u16 {
        self.0.div_ceil(Self::SCALE)
    }
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
    pub const fn to_fp(self) -> u16 {
        self.0
    }
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / f64::from(Self::SCALE)
    }

    /// The stock threshold comparison is inclusive. It is deliberately independent
    /// of the displayed stamp value, which floors a normalized mantissa.
    pub fn accepts_digest(self, digest: &[u8; 32]) -> bool {
        if self.0 == 0 {
            return true;
        }
        digest <= &self.target_digest()
    }

    pub fn target_digest(self) -> [u8; 32] {
        if self.0 == 0 {
            return [u8::MAX; 32];
        }
        let inverse = 256 * 256 - u32::from(self.0);
        let exponent = (inverse / 256) as i32;
        let mantissa = TABLE[(inverse % 256) as usize];
        let mut out = [0_u8; 32];
        for bit in 0..34_i32 {
            let position = bit + exponent - 33;
            if position >= 0 && position < 256 && mantissa & (1_u64 << bit) != 0 {
                out[31 - position as usize / 8] |= 1 << (position as usize % 8);
            }
        }
        out
    }
}

impl core::fmt::Display for StampCost {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.to_f64().fmt(out)
    }
}

/// Stock's floored value on the 1/256 grid. The all-zero digest has value 256.
pub fn value_of_digest(digest: &[u8; 32]) -> f64 {
    // Count full zero bytes, followed by the first nonzero byte's leading zeros.
    let full = digest.iter().take_while(|byte| **byte == 0).count();
    if full == 32 {
        return 256.0;
    }
    let leading = full as u32 * 8 + digest[full].leading_zeros();
    let bits = 256 - leading;
    let mut mantissa = 0_u64;
    for offset in 0..34 {
        mantissa <<= 1;
        if offset < bits {
            let position = leading + offset;
            mantissa |= u64::from((digest[position as usize / 8] >> (7 - position % 8)) & 1);
        }
    }
    let climbed = TABLE.partition_point(|entry| *entry <= mantissa) as u32;
    f64::from((leading + 1) * 256 - climbed) / 256.0
}

pub fn detached_valid(material: &[u8], stamp: &[u8], cost: StampCost) -> bool {
    let digest: [u8; 32] = Sha256::new()
        .chain_update(material)
        .chain_update(stamp)
        .finalize()
        .into();
    cost.accepts_digest(&digest)
}

const TABLE: [u64; 256] = [
    0x200000000,
    0x201635f4b,
    0x202c7b53f,
    0x2042d0287,
    0x2059347ce,
    0x206fa85c2,
    0x20862bd0d,
    0x209cbee5e,
    0x20b361a62,
    0x20ca141c7,
    0x20e0d653b,
    0x20f7a856e,
    0x210e8a30e,
    0x21257becc,
    0x213c7d958,
    0x21538f363,
    0x216ab0d9f,
    0x2181e28bc,
    0x21992456e,
    0x21b076467,
    0x21c7d865a,
    0x21df4abfb,
    0x21f6cd5ff,
    0x220e6051a,
    0x222603a02,
    0x223db756b,
    0x22557b80d,
    0x226d5029e,
    0x2285355d5,
    0x229d2b269,
    0x22b531914,
    0x22cd48a8e,
    0x22e57078f,
    0x22fda90d2,
    0x2315f2711,
    0x232e4cb06,
    0x2346b7d6d,
    0x235f33f02,
    0x2377c1080,
    0x23905f2a5,
    0x23a90e62d,
    0x23c1cebd6,
    0x23daa045f,
    0x23f383087,
    0x240c7710c,
    0x24257c6af,
    0x243e9322f,
    0x2457bb44f,
    0x2470f4dce,
    0x248a3ff70,
    0x24a39c9f6,
    0x24bd0ae23,
    0x24d68acbc,
    0x24f01c683,
    0x2509bfc3e,
    0x252374eb2,
    0x253d3bea3,
    0x255714cda,
    0x2570ffa1b,
    0x258afc72e,
    0x25a50b4dc,
    0x25bf2c3ec,
    0x25d95f527,
    0x25f3a4957,
    0x260dfc146,
    0x262865dbd,
    0x2642e1f89,
    0x265d70775,
    0x26781164c,
    0x2692c4cdc,
    0x26ad8abf2,
    0x26c86345b,
    0x26e34e6e7,
    0x26fe4c463,
    0x27195cda0,
    0x27348036e,
    0x274fb669c,
    0x276aff7fd,
    0x27865b862,
    0x27a1ca89d,
    0x27bd4c982,
    0x27d8e1be3,
    0x27f48a095,
    0x28104586c,
    0x282c1443e,
    0x2847f64e1,
    0x2863ebb2a,
    0x287ff47f0,
    0x289c10c0c,
    0x28b840854,
    0x28d483da3,
    0x28f0dacd1,
    0x290d456b8,
    0x2929c3c32,
    0x294655e1a,
    0x2962fbd4d,
    0x297fb5aa6,
    0x299c83702,
    0x29b96533f,
    0x29d65b03b,
    0x29f364ed3,
    0x2a1082fe8,
    0x2a2db5459,
    0x2a4afbd07,
    0x2a6856ad3,
    0x2a85c5e9e,
    0x2aa34994b,
    0x2ac0e1bbd,
    0x2ade8e6d6,
    0x2afc4fb7c,
    0x2b1a25a92,
    0x2b38104ff,
    0x2b560fba9,
    0x2b7423f75,
    0x2b924d14b,
    0x2bb08b213,
    0x2bcede2b5,
    0x2bed4641b,
    0x2c0bc372e,
    0x2c2a55cd9,
    0x2c48fd607,
    0x2c67ba3a3,
    0x2c868c699,
    0x2ca573fd7,
    0x2cc47104a,
    0x2ce3838e1,
    0x2d02aba89,
    0x2d21e9633,
    0x2d413cccf,
    0x2d60a5f4e,
    0x2d8024ea1,
    0x2d9fb9bba,
    0x2dbf6478c,
    0x2ddf2530b,
    0x2dfefbf2a,
    0x2e1ee8cde,
    0x2e3eebd1d,
    0x2e5f050dd,
    0x2e7f34914,
    0x2e9f7a6ba,
    0x2ebfd6ac8,
    0x2ee049635,
    0x2f00d29fb,
    0x2f2172715,
    0x2f4228e7d,
    0x2f62f612e,
    0x2f83da026,
    0x2fa4d4c5f,
    0x2fc5e66d9,
    0x2fe70f092,
    0x30084ea87,
    0x3029a55ba,
    0x304b13329,
    0x306c983d7,
    0x308e348c4,
    0x30afe82f3,
    0x30d1b3368,
    0x30f395b26,
    0x31158fb31,
    0x3137a148f,
    0x3159ca845,
    0x317c0b75a,
    0x319e642d6,
    0x31c0d4bc1,
    0x31e35d322,
    0x3205fda05,
    0x3228b6172,
    0x324b86a75,
    0x326e6f619,
    0x32917056b,
    0x32b489979,
    0x32d7bb34e,
    0x32fb053fb,
    0x331e67c8f,
    0x3341e2e19,
    0x3365769aa,
    0x338923054,
    0x33ace8329,
    0x33d0c633c,
    0x33f4bd1a0,
    0x3418ccf6b,
    0x343cf5db1,
    0x346137d89,
    0x348593008,
    0x34aa07647,
    0x34ce9515e,
    0x34f33c266,
    0x3517fca78,
    0x353cd6aaf,
    0x3561ca426,
    0x3586d77fa,
    0x35abfe747,
    0x35d13f32b,
    0x35f699cc4,
    0x361c0e531,
    0x36419cd93,
    0x366745709,
    0x368d082b6,
    0x36b2e51bc,
    0x36d8dc53e,
    0x36feede5f,
    0x372519e44,
    0x374b60614,
    0x3771c16f3,
    0x37983d209,
    0x37bed387e,
    0x37e584b7a,
    0x380c50c28,
    0x383337bb0,
    0x385a39b3f,
    0x388156bff,
    0x38a88ef1f,
    0x38cfe25ca,
    0x38f751131,
    0x391edb280,
    0x394680aea,
    0x396e41b9d,
    0x39961e5cd,
    0x39be16aab,
    0x39e62ab6b,
    0x3a0e5a940,
    0x3a36a6561,
    0x3a5f0e101,
    0x3a8791d59,
    0x3ab031b9f,
    0x3ad8edd0c,
    0x3b01c62d9,
    0x3b2abae3f,
    0x3b53cc07b,
    0x3b7cf9ac7,
    0x3ba643e60,
    0x3bcfaac83,
    0x3bf92e66f,
    0x3c22ced63,
    0x3c4c8c29e,
    0x3c7666762,
    0x3ca05dcf1,
    0x3cca7248c,
    0x3cf4a3f78,
    0x3d1ef2ef9,
    0x3d495f454,
    0x3d73e90cf,
    0x3d9e905b1,
    0x3dc955443,
    0x3df437dcc,
    0x3e1f38396,
    0x3e4a566ed,
    0x3e759291b,
    0x3ea0ecb6d,
    0x3ecc64f31,
    0x3ef7fb5b3,
    0x3f23b0044,
    0x3f4f83033,
    0x3f7b746d2,
    0x3fa784571,
    0x3fd3b2d65,
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantization_is_half_up_and_nonfinite_is_refused() {
        assert_eq!(
            StampCost::from_f64(8.0 + 1.0 / 512.0).unwrap().to_fp(),
            2049
        );
        assert_eq!(StampCost::from_f64(-0.5), Some(StampCost::ZERO));
        assert_eq!(StampCost::from_f64(101.0), Some(StampCost::MAX));
        assert_eq!(StampCost::from_f64(f64::NAN), None);
        assert_eq!(StampCost::from_f64(f64::INFINITY), None);
    }
    #[test]
    fn thresholds_are_inclusive() {
        for fp in [1, 128, 256, 2048, 2049, 2176, 25599, 25600] {
            let cost = StampCost::from_fp(fp);
            let mut digest = cost.target_digest();
            assert!(cost.accepts_digest(&digest));
            for byte in digest.iter_mut().rev() {
                let (next, overflow) = byte.overflowing_add(1);
                *byte = next;
                if !overflow {
                    break;
                }
            }
            assert!(!cost.accepts_digest(&digest));
        }
    }
    #[test]
    fn integer_cost_threshold_matches_stock_power_of_two() {
        let mut digest = [0_u8; 32];
        digest[1] = 1;
        assert!(StampCost::from_f64(16.0).unwrap().accepts_digest(&digest));
        assert!(
            !StampCost::from_f64(16.0 + 1.0 / 256.0)
                .unwrap()
                .accepts_digest(&digest)
        );
        assert_eq!(value_of_digest(&[0; 32]), 256.0);
        assert_eq!(value_of_digest(&[255; 32]), 0.0);
    }
}
