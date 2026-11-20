//! The unsigned `applied` line: the radio profile a board is actually running and, in a node
//! build, the relay flags in force (wall node D20).
//!
//! Diagnostics only. It carries no authority and changes no control or durable bytes; a
//! controller reads it beside signed Status, which says nothing about the radio. Boards render
//! it and hosts parse it with the same code, so the two cannot drift.

use core::fmt::Write as _;

use selvage::PhyProfile;

use crate::region::Region;

/// The probe that asks for the line, without its line ending.
pub const QUERY: &[u8] = b"applied";

/// Room for the line with every value at its widest, line ending included.
pub const LINE_CAPACITY: usize = 160;

/// The relay flags a board reports beside its profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relay {
    /// This image runs no relay (the modem builds): the field is left out.
    NotServed,
    /// A node with no applied configuration, running its own defaults.
    Unclaimed,
    /// The flags of the configuration the board last applied.
    Claimed { announces: bool, packets: bool },
}

/// What the line reports. The profile fields are the ones a controller chooses; power is the
/// antenna power the board applied after its region and hardware clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedRadio {
    pub region: Region,
    pub frequency_hz: u32,
    pub bandwidth_hz: u32,
    pub spreading_factor: u8,
    pub coding_rate_denominator: u8,
    pub preamble_symbols: u16,
    pub sync_word: u8,
    pub tx_power_dbm: i8,
    pub relay: Relay,
}

/// The two SX126x sync-word register bytes for a canonical LoRa sync word, as the vendored
/// `lora-phy` writes them (`0x2b` is register value `0x24b4`).
pub const fn sx126x_sync_register(sync_word: u8) -> u16 {
    let high = (sync_word & 0xf0) | 0x04;
    let low = ((sync_word & 0x0f) << 4) | 0x04;
    ((high as u16) << 8) | low as u16
}

/// Whether one host line is the query.
pub fn is_query(packet: &[u8]) -> bool {
    packet
        .strip_suffix(b"\n")
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        == Some(QUERY)
}

impl AppliedRadio {
    pub fn new(region: Region, profile: &PhyProfile, relay: Relay) -> Self {
        Self {
            region,
            frequency_hz: profile.frequency_hz,
            bandwidth_hz: profile.bandwidth_hz,
            spreading_factor: profile.spreading_factor,
            coding_rate_denominator: profile.coding_rate_denominator,
            preamble_symbols: profile.preamble_symbols,
            sync_word: profile.sync_word,
            tx_power_dbm: profile.tx_power_dbm,
            relay,
        }
    }

    pub const fn sync_register(&self) -> u16 {
        sx126x_sync_register(self.sync_word)
    }

    /// The line, ending in CRLF.
    pub fn render(&self) -> radio_face::Text<LINE_CAPACITY> {
        let mut out = radio_face::Text::<LINE_CAPACITY>::empty();
        let _ = write!(
            &mut out,
            "applied region={} freq={} bw={} sf={} cr={} preamble={} sync={:02x} reg={:04x} \
             power={}",
            self.region.name(),
            self.frequency_hz,
            self.bandwidth_hz,
            self.spreading_factor,
            self.coding_rate_denominator,
            self.preamble_symbols,
            self.sync_word,
            self.sync_register(),
            self.tx_power_dbm,
        );
        let _ = match self.relay {
            Relay::NotServed => Ok(()),
            Relay::Unclaimed => write!(&mut out, " relay=unclaimed"),
            Relay::Claimed { announces, packets } => write!(
                &mut out,
                " relay=claimed announces={} packets={}",
                u8::from(announces),
                u8::from(packets)
            ),
        };
        let _ = write!(&mut out, "\r\n");
        out
    }

    /// Read one line back. `None` for anything that is not a whole, consistent `applied`
    /// line: a missing or repeated field, an unknown region, or a register that does not match
    /// the sync word.
    pub fn parse(line: &str) -> Option<Self> {
        let mut fields = line.trim_end().strip_prefix("applied ")?.split(' ');
        let mut next = |key: &str| {
            fields
                .next()
                .and_then(|field| field.strip_prefix(key))
                .and_then(|rest| rest.strip_prefix('='))
        };
        let region = next("region").and_then(region_named)?;
        let frequency_hz = next("freq")?.parse().ok()?;
        let bandwidth_hz = next("bw")?.parse().ok()?;
        let spreading_factor = next("sf")?.parse().ok()?;
        let coding_rate_denominator = next("cr")?.parse().ok()?;
        let preamble_symbols = next("preamble")?.parse().ok()?;
        let sync_word = u8::from_str_radix(next("sync")?, 16).ok()?;
        let register = u16::from_str_radix(next("reg")?, 16).ok()?;
        let tx_power_dbm = next("power")?.parse().ok()?;
        let relay = match (fields.next(), fields.next(), fields.next(), fields.next()) {
            (None, ..) => Relay::NotServed,
            (Some("relay=unclaimed"), None, ..) => Relay::Unclaimed,
            (Some("relay=claimed"), Some(announces), Some(packets), None) => Relay::Claimed {
                announces: flag(announces.strip_prefix("announces=")?)?,
                packets: flag(packets.strip_prefix("packets=")?)?,
            },
            _ => return None,
        };
        if register != sx126x_sync_register(sync_word) {
            return None;
        }
        Some(Self {
            region,
            frequency_hz,
            bandwidth_hz,
            spreading_factor,
            coding_rate_denominator,
            preamble_symbols,
            sync_word,
            tx_power_dbm,
            relay,
        })
    }
}

fn region_named(name: &str) -> Option<Region> {
    if name == Region::Unset.name() {
        return Some(Region::Unset);
    }
    Region::choices().find(|region| region.name() == name)
}

fn flag(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widest(relay: Relay) -> AppliedRadio {
        AppliedRadio {
            region: Region::Anz915,
            frequency_hz: u32::MAX,
            bandwidth_hz: u32::MAX,
            spreading_factor: u8::MAX,
            coding_rate_denominator: u8::MAX,
            preamble_symbols: u16::MAX,
            sync_word: 0xff,
            tx_power_dbm: i8::MIN,
            relay,
        }
    }

    #[test]
    fn the_register_is_what_lora_phy_writes() {
        assert_eq!(sx126x_sync_register(0x2b), 0x24b4);
        assert_eq!(sx126x_sync_register(0x12), 0x1424);
        assert_eq!(sx126x_sync_register(0x00), 0x0404);
    }

    #[test]
    fn the_widest_line_fits_with_its_ending_and_reads_back() {
        for relay in [
            Relay::NotServed,
            Relay::Unclaimed,
            Relay::Claimed {
                announces: true,
                packets: true,
            },
        ] {
            let applied = widest(relay);
            let line = applied.render();
            assert!(line.as_str().ends_with("\r\n"), "{}", line.as_str());
            assert_eq!(AppliedRadio::parse(line.as_str()), Some(applied));
        }
    }

    #[test]
    fn a_relay_on_profile_renders_as_expected() {
        let mut profile = PhyProfile::meshtastic_long_fast(908_125_000);
        profile.sync_word = 0x12;
        let applied = AppliedRadio::new(
            Region::Us915,
            &profile,
            Relay::Claimed {
                announces: true,
                packets: false,
            },
        );
        assert_eq!(
            applied.render().as_str(),
            "applied region=US915 freq=908125000 bw=250000 sf=11 cr=5 preamble=16 sync=12 \
             reg=1424 power=17 relay=claimed announces=1 packets=0\r\n"
        );
    }

    #[test]
    fn inconsistent_or_partial_lines_are_refused() {
        let good = "applied region=US915 freq=1 bw=2 sf=7 cr=5 preamble=8 sync=2b reg=24b4 power=0";
        assert!(AppliedRadio::parse(good).is_some());
        assert!(AppliedRadio::parse(&good.replace("24b4", "24b5")).is_none());
        assert!(AppliedRadio::parse(&good.replace("US915", "MARS")).is_none());
        assert!(AppliedRadio::parse(&good.replace(" bw=2", "")).is_none());
        assert!(AppliedRadio::parse(&std::format!("{good} extra=1")).is_none());
        assert!(AppliedRadio::parse("air uptime=1ms").is_none());
    }

    #[test]
    fn the_query_takes_either_line_ending() {
        assert!(is_query(b"applied\n"));
        assert!(is_query(b"applied\r\n"));
        assert!(!is_query(b"applied"));
        assert!(!is_query(b"appliedx\n"));
    }
}
