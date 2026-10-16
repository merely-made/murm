//! RNode telemetry reports, read as RNS reads them (`RNodeInterface.py` 124-131, 698-704,
//! 882-1069). Pure parsers over unescaped frame payloads; the [`super::RNode`] keeps the
//! latest of each.

use super::config::{RSSI_OFFSET, cmd};

/// `STAT_CHTM` (0x25, 11 bytes): airtime and channel load as percent, signal levels in dBm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelStats {
    pub airtime_short: f64,
    pub airtime_long: f64,
    pub channel_load_short: f64,
    pub channel_load_long: f64,
    pub current_rssi_dbm: i16,
    pub noise_floor_dbm: i16,
    /// `None` when the device reports none (0xFF).
    pub interference_dbm: Option<i16>,
}

/// `STAT_PHYPRM` (0x26, 12 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhyStats {
    pub symbol_time_ms: f64,
    pub symbol_rate: u16,
    pub preamble_symbols: u16,
    pub preamble_ms: u16,
    pub csma_slot_ms: u16,
    pub csma_difs_ms: u16,
}

/// `STAT_CSMA` (0x28, 3 bytes): the contention window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CsmaStats {
    pub band: u8,
    pub cw_min: u8,
    pub cw_max: u8,
}

/// `STAT_BAT` (0x27, 2 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// 0 unknown, 1 discharging, 2 charging, 3 charged (`RNodeInterface.py` 128-131).
    pub state: u8,
    /// Clamped to 100.
    pub percent: u8,
}

impl Battery {
    /// RNS's name for the state (`RNodeInterface.py` 1217-1225).
    pub fn state_name(&self) -> &'static str {
        match self.state {
            1 => "discharging",
            2 => "charging",
            3 => "charged",
            _ => "unknown",
        }
    }
}

/// The latest report of each kind, and the on-air bitrate of the echoed settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Telemetry {
    pub channel: Option<ChannelStats>,
    pub phy: Option<PhyStats>,
    pub csma: Option<CsmaStats>,
    pub battery: Option<Battery>,
    /// MCU temperature in °C; `None` until reported or when outside -30..=90.
    pub temperature_c: Option<i16>,
    /// From the echoed SF, CR and bandwidth, as RNS's `updateBitrate`.
    pub bitrate: Option<f64>,
}

impl Telemetry {
    /// Record one report. False if `command` is not a telemetry report or `payload` is short.
    pub fn record(&mut self, command: u8, payload: &[u8]) -> bool {
        let u16_at = |i: usize| u16::from_be_bytes([payload[i], payload[i + 1]]);
        let dbm = |raw: u8| i16::from(raw) - RSSI_OFFSET;
        let percent = |i: usize| f64::from(u16_at(i)) / 100.0;
        match (command, payload.len()) {
            (cmd::STAT_CHTM, 11..) => {
                self.channel = Some(ChannelStats {
                    airtime_short: percent(0),
                    airtime_long: percent(2),
                    channel_load_short: percent(4),
                    channel_load_long: percent(6),
                    current_rssi_dbm: dbm(payload[8]),
                    noise_floor_dbm: dbm(payload[9]),
                    interference_dbm: (payload[10] != 0xFF).then(|| dbm(payload[10])),
                })
            }
            (cmd::STAT_PHYPRM, 12..) => {
                self.phy = Some(PhyStats {
                    symbol_time_ms: f64::from(u16_at(0)) / 1000.0,
                    symbol_rate: u16_at(2),
                    preamble_symbols: u16_at(4),
                    preamble_ms: u16_at(6),
                    csma_slot_ms: u16_at(8),
                    csma_difs_ms: u16_at(10),
                })
            }
            (cmd::STAT_CSMA, 3..) => {
                self.csma = Some(CsmaStats {
                    band: payload[0],
                    cw_min: payload[1],
                    cw_max: payload[2],
                })
            }
            (cmd::STAT_BAT, 2..) => {
                self.battery = Some(Battery {
                    state: payload[0],
                    percent: payload[1].min(100),
                })
            }
            (cmd::STAT_TEMP, 1..) => {
                let celsius = i16::from(payload[0]) - 120;
                self.temperature_c = (-30..=90).contains(&celsius).then_some(celsius);
            }
            _ => return false,
        }
        true
    }
}

/// Link quality in percent from a packet's SNR at `sf`: -9 dB (2 dB lower per SF above 7)
/// maps to 0, +6 dB to 100, rounded to 0.1 (`RNodeInterface.py` 124-126, 882-893).
pub fn quality(snr_db: f32, sf: u8) -> f32 {
    let min = -9.0 - 2.0 * (f64::from(sf) - 7.0);
    let q = (f64::from(snr_db) - min) / (6.0 - min) * 100.0;
    // Python's round() takes ties to even.
    ((q * 10.0).round_ties_even() / 10.0).clamp(0.0, 100.0) as f32
}

/// RNS's on-air bitrate in bits per second: `sf * (4 / cr) / (2^sf / bw_khz) * 1000`, with
/// `cr` the denominator 5..=8 (`RNodeInterface.py` 698-704).
pub fn bitrate(sf: u8, cr: u8, bandwidth_hz: u32) -> f64 {
    let symbol_ms = f64::from(1_u32 << sf.min(31)) / (f64::from(bandwidth_hz) / 1000.0);
    f64::from(sf) * ((4.0 / f64::from(cr)) / symbol_ms) * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_report_follows_rns_layout() {
        let mut t = Telemetry::default();
        let chtm = [0x04, 0xD2, 0x00, 0x7B, 0x27, 0x10, 0x00, 0x01, 57, 40, 0xFF];
        assert!(t.record(cmd::STAT_CHTM, &chtm));
        let c = t.channel.unwrap();
        assert_eq!((c.airtime_short, c.airtime_long), (12.34, 1.23));
        assert_eq!((c.channel_load_short, c.channel_load_long), (100.0, 0.01));
        assert_eq!((c.current_rssi_dbm, c.noise_floor_dbm), (-100, -117));
        assert_eq!(c.interference_dbm, None);
        assert!(t.record(cmd::STAT_CHTM, &[[0; 10].as_slice(), &[77]].concat()));
        assert_eq!(t.channel.unwrap().interference_dbm, Some(-80));
        assert!(!t.record(cmd::STAT_CHTM, &[0; 10]), "short");
    }

    #[test]
    fn phy_csma_battery_and_temperature() {
        let mut t = Telemetry::default();
        let phy = [0x04, 0x00, 0x03, 0xD1, 0, 18, 0, 25, 0, 24, 0, 60];
        assert!(t.record(cmd::STAT_PHYPRM, &phy));
        let p = t.phy.unwrap();
        assert_eq!((p.symbol_time_ms, p.symbol_rate), (1.024, 977));
        assert_eq!((p.preamble_symbols, p.preamble_ms), (18, 25));
        assert_eq!((p.csma_slot_ms, p.csma_difs_ms), (24, 60));
        assert!(t.record(cmd::STAT_CSMA, &[2, 3, 8]));
        assert_eq!(t.csma.unwrap().cw_max, 8);
        assert!(t.record(cmd::STAT_BAT, &[2, 140]));
        assert_eq!(t.battery.unwrap().percent, 100);
        assert_eq!(t.battery.unwrap().state_name(), "charging");
        for (raw, want) in [
            (145, Some(25)),
            (90, Some(-30)),
            (210, Some(90)),
            (89, None),
        ] {
            assert!(t.record(cmd::STAT_TEMP, &[raw]));
            assert_eq!(t.temperature_c, want);
        }
        assert!(!t.record(cmd::STAT_RSSI, &[0]));
    }

    #[test]
    fn quality_spans_the_sf_window() {
        assert_eq!(quality(6.0, 7), 100.0);
        assert_eq!(quality(-9.0, 7), 0.0);
        assert_eq!(quality(-1.5, 7), 50.0);
        // SF12 widens the floor to -19 dB.
        assert_eq!(quality(-19.0, 12), 0.0);
        assert_eq!(quality(-6.5, 12), 50.0);
        assert_eq!(quality(20.0, 9), 100.0);
    }

    #[test]
    fn bitrate_matches_rns() {
        assert_eq!(bitrate(7, 5, 500_000), 21_875.0);
        assert!((bitrate(12, 8, 125_000) - 183.105_468_75).abs() < 1e-9);
    }
}
