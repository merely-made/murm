//! What sits between the radio chip and the antenna.
//!
//! A profile's `tx_power_dbm` is power at the antenna, the quantity regions regulate. A board
//! with a front end turns that into a chip setting and a PA choice here, so the executive
//! never drives a chip level the front end cannot take.

/// The SX126x's settable output range.
pub const CHIP_MIN_DBM: i8 = -9;
/// The SX126x's settable output range.
pub const CHIP_MAX_DBM: i8 = 22;

/// The board's transmit chain.
#[derive(Clone, Copy, Debug)]
pub enum FrontEnd {
    /// The chip drives the antenna directly.
    Direct,
    /// A fixed pad into a module with a PA and a low-loss bypass (Heltec V4.2: 17 dB pad
    /// into a GC1109).
    PadAndPa {
        pad_db: i8,
        pa_gain_db: i8,
        bypass_loss_db: i8,
        /// Board cap on antenna power with the PA engaged.
        cap_dbm: i8,
        /// Largest chip setting the module's PA input tolerates after the pad.
        pa_input_max_dbm: i8,
        /// Select the PA (`true`) or the bypass (`false`) for the next transmissions.
        select_pa: fn(bool),
    },
}

/// One transmit setting the executive can apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxPlan {
    /// Chip output setting.
    pub chip_dbm: i8,
    /// Whether the front end's PA is in the path.
    pub pa: bool,
    /// Power at the antenna this plan produces.
    pub antenna_dbm: i8,
}

impl FrontEnd {
    /// The most antenna power this board will plan.
    pub fn max_antenna_dbm(&self) -> i8 {
        match *self {
            Self::Direct => CHIP_MAX_DBM,
            Self::PadAndPa { cap_dbm, .. } => cap_dbm,
        }
    }

    /// The chip setting and path for `antenna_dbm`, clamped to what this chain can produce.
    /// Requests the bypass can reach use it; the PA serves only those above.
    pub fn plan(&self, antenna_dbm: i8) -> TxPlan {
        match *self {
            Self::Direct => {
                let chip_dbm = antenna_dbm.clamp(CHIP_MIN_DBM, CHIP_MAX_DBM);
                TxPlan {
                    chip_dbm,
                    pa: false,
                    antenna_dbm: chip_dbm,
                }
            }
            Self::PadAndPa {
                pad_db,
                pa_gain_db,
                bypass_loss_db,
                cap_dbm,
                pa_input_max_dbm,
                ..
            } => {
                let bypass_loss = pad_db + bypass_loss_db;
                let wanted = antenna_dbm.min(cap_dbm);
                if wanted <= CHIP_MAX_DBM - bypass_loss {
                    let chip_dbm = (wanted + bypass_loss).clamp(CHIP_MIN_DBM, CHIP_MAX_DBM);
                    TxPlan {
                        chip_dbm,
                        pa: false,
                        antenna_dbm: chip_dbm - bypass_loss,
                    }
                } else {
                    let pa_chip_max = CHIP_MAX_DBM.min(pa_input_max_dbm + pad_db);
                    let chip_dbm = (wanted + pad_db - pa_gain_db).clamp(CHIP_MIN_DBM, pa_chip_max);
                    TxPlan {
                        chip_dbm,
                        pa: true,
                        antenna_dbm: chip_dbm - pad_db + pa_gain_db,
                    }
                }
            }
        }
    }

    /// Put the PA in or out of the path. A direct chain has nothing to switch.
    pub fn select(&self, pa: bool) {
        if let Self::PadAndPa { select_pa, .. } = *self {
            select_pa(pa);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: bool) {}

    /// Heltec V4.2 figures: 17 dB pad, GC1109 30 dB gain and 1 dB bypass loss, +5 dBm PA
    /// input absolute maximum, owner's +20 dBm cap.
    fn v4() -> FrontEnd {
        FrontEnd::PadAndPa {
            pad_db: 17,
            pa_gain_db: 30,
            bypass_loss_db: 1,
            cap_dbm: 20,
            pa_input_max_dbm: 5,
            select_pa: none,
        }
    }

    #[test]
    fn low_requests_use_the_bypass() {
        assert_eq!(
            v4().plan(0),
            TxPlan {
                chip_dbm: 18,
                pa: false,
                antenna_dbm: 0
            }
        );
        assert!(!v4().plan(4).pa);
        assert_eq!(v4().plan(4).chip_dbm, 22);
        // Below what the chip can go, the bypass reports what it really radiates.
        assert_eq!(v4().plan(-40).antenna_dbm, CHIP_MIN_DBM - 18);
    }

    #[test]
    fn high_requests_use_the_pa_under_the_cap() {
        assert_eq!(
            v4().plan(17),
            TxPlan {
                chip_dbm: 4,
                pa: true,
                antenna_dbm: 17
            }
        );
        assert_eq!(v4().plan(30), v4().plan(20));
        assert_eq!(v4().plan(30).antenna_dbm, 20);
    }

    #[test]
    fn no_plan_exceeds_the_cap_or_the_pa_input_rating() {
        for request in -60..=60 {
            let plan = v4().plan(request);
            assert!(plan.antenna_dbm <= 20, "{request}: {plan:?}");
            assert!((CHIP_MIN_DBM..=CHIP_MAX_DBM).contains(&plan.chip_dbm));
            if plan.pa {
                assert!(plan.chip_dbm - 17 <= 5, "{request}: {plan:?}");
            }
        }
    }

    #[test]
    fn antenna_power_never_falls_as_the_request_rises() {
        let mut last = i8::MIN;
        for request in -60..=60 {
            let now = v4().plan(request).antenna_dbm;
            assert!(now >= last, "{request}: {now} < {last}");
            last = now;
        }
    }

    #[test]
    fn a_direct_chain_is_the_chip() {
        assert_eq!(FrontEnd::Direct.plan(17).chip_dbm, 17);
        assert_eq!(FrontEnd::Direct.plan(30).antenna_dbm, CHIP_MAX_DBM);
        assert_eq!(FrontEnd::Direct.max_antenna_dbm(), CHIP_MAX_DBM);
    }
}
