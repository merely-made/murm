//! The board's unsigned `applied` line (wall_node_management D20): the radio profile it is
//! actually running and, on a node, the relay flags in force. Diagnostics beside signed
//! Status, which carries no radio fields; nothing here is authority.

use std::fmt;
use std::future::Future;

use postilion::control::verified::{UsbControlError, UsbControlTransport};
use radio_hand::applied::{AppliedRadio, Relay};
use radio_hand::control::PublicConfigurationV1;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

const PROBE: &[u8] = b"applied\n";
const PREFIX: &str = "applied ";

/// A carrier that can also ask the board for its `applied` line between signed exchanges.
pub trait AppliedLineSource {
    type LineError: fmt::Display;

    /// The board's whole `applied` line, without its line ending.
    fn applied_line(&mut self) -> impl Future<Output = Result<String, Self::LineError>>;
}

impl<T> AppliedLineSource for UsbControlTransport<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type LineError = UsbControlError;

    async fn applied_line(&mut self) -> Result<String, UsbControlError> {
        self.diagnostic_line(PROBE, PREFIX).await
    }
}

/// What one `applied` line reported, field by field.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AppliedRecord {
    pub region: String,
    pub frequency_hz: u32,
    pub bandwidth_hz: u32,
    pub spreading_factor: u8,
    pub coding_rate_denominator: u8,
    pub preamble_symbols: u16,
    pub sync_word: String,
    pub sync_register: String,
    /// Antenna power after the board's region and hardware clamp.
    pub tx_power_dbm: i8,
    /// `not-served` (a modem image), `unclaimed` or `claimed`.
    pub relay: &'static str,
    pub relay_announces: Option<bool>,
    pub relay_packets: Option<bool>,
}

impl AppliedRecord {
    pub fn of(applied: &AppliedRadio) -> Self {
        let (relay, relay_announces, relay_packets) = match applied.relay {
            Relay::NotServed => ("not-served", None, None),
            Relay::Unclaimed => ("unclaimed", None, None),
            Relay::Claimed { announces, packets } => ("claimed", Some(announces), Some(packets)),
        };
        Self {
            region: applied.region.name().to_string(),
            frequency_hz: applied.frequency_hz,
            bandwidth_hz: applied.bandwidth_hz,
            spreading_factor: applied.spreading_factor,
            coding_rate_denominator: applied.coding_rate_denominator,
            preamble_symbols: applied.preamble_symbols,
            sync_word: format!("0x{:02x}", applied.sync_word),
            sync_register: format!("0x{:04x}", applied.sync_register()),
            tx_power_dbm: applied.tx_power_dbm,
            relay,
            relay_announces,
            relay_packets,
        }
    }
}

/// Whether the board runs what the candidate asked for. Power is left out: the board clamps
/// it to its region and hardware, and the record keeps the applied value. A modem image runs
/// no relay, so its flags are not compared.
pub fn matches_candidate(applied: &AppliedRadio, candidate: &PublicConfigurationV1) -> bool {
    let phy = candidate.requested_reticulum_phy();
    let transport = candidate.reticulum_transport();
    let relay = match applied.relay {
        Relay::NotServed => true,
        Relay::Unclaimed => false,
        Relay::Claimed { announces, packets } => {
            announces == transport.relay_announces && packets == transport.relay_packets
        }
    };
    relay
        && applied.region == candidate.region()
        && applied.frequency_hz == phy.frequency_hz
        && applied.bandwidth_hz == phy.bandwidth_hz
        && applied.spreading_factor == phy.spreading_factor
        && applied.coding_rate_denominator == phy.coding_rate_denominator
        && applied.preamble_symbols == phy.preamble_symbols
        && applied.sync_word == phy.sync_word
}
