//! The run record a lifecycle invocation writes for its receipt.
//!
//! Public facts only: no commit token, no key material, and nothing derived from either.
//! Counters, sequences, generations, the change id and transaction ids are public: they are
//! what the board checks and journals, not secrets.

use radio_hand::control::{
    ControlStatusBootFact, ControlStatusEvidence, ControlStatusV1, PublicConfigurationV1,
};
use serde::Serialize;

/// The configuration this run sent, field by field.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CandidateRecord {
    pub region: String,
    pub frequency_hz: u32,
    pub bandwidth_hz: u32,
    pub spreading_factor: u8,
    pub coding_rate_denominator: u8,
    pub preamble_symbols: u16,
    pub sync_word: String,
    pub explicit_header: bool,
    pub crc: bool,
    pub invert_iq: bool,
    pub tx_power_dbm: i8,
    pub relay_announces: bool,
    pub relay_packets: bool,
    pub max_hops: u8,
    pub management_carriers_mask: u8,
}

impl CandidateRecord {
    pub fn of(candidate: &PublicConfigurationV1) -> Self {
        let phy = candidate.requested_reticulum_phy();
        let transport = candidate.reticulum_transport();
        Self {
            region: candidate.region().name().to_string(),
            frequency_hz: phy.frequency_hz,
            bandwidth_hz: phy.bandwidth_hz,
            spreading_factor: phy.spreading_factor,
            coding_rate_denominator: phy.coding_rate_denominator,
            preamble_symbols: phy.preamble_symbols,
            sync_word: format!("0x{:02x}", phy.sync_word),
            explicit_header: phy.explicit_header,
            crc: phy.crc,
            invert_iq: phy.invert_iq,
            tx_power_dbm: phy.tx_power_dbm,
            relay_announces: transport.relay_announces,
            relay_packets: transport.relay_packets,
            max_hops: transport.max_hops,
            management_carriers_mask: candidate.enabled_management_carriers().mask(),
        }
    }
}

/// One signed command: what it was, the values it spent, and how it ended.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    pub step: &'static str,
    pub counter: u64,
    pub sequence: Option<u64>,
    pub transaction: Option<String>,
    pub outcome: String,
}

/// The public status fields the board returned.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusRecord {
    pub known_good_generation: u64,
    pub generation_watermark: u64,
    pub control: &'static str,
    pub pending: &'static str,
    pub boot: &'static str,
}

impl StatusRecord {
    pub fn of(status: &ControlStatusV1) -> Self {
        Self {
            known_good_generation: status.known_good_generation().0,
            generation_watermark: status.generation_watermark().0,
            control: evidence(status.control()),
            pending: evidence(status.pending()),
            boot: match status.boot() {
                ControlStatusBootFact::KnownGoodApplied => "known-good-applied",
                ControlStatusBootFact::RecoveredRollback => "recovered-rollback",
            },
        }
    }
}

const fn evidence(value: ControlStatusEvidence) -> &'static str {
    match value {
        ControlStatusEvidence::Blank => "blank",
        ControlStatusEvidence::Valid => "valid",
        ControlStatusEvidence::Corrupt => "corrupt",
    }
}

/// Everything a lifecycle receipt needs, built up as the run goes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct LifecycleRecord {
    pub schema: &'static str,
    pub node: String,
    pub change: String,
    /// `commit`, `revert` or `expire`.
    pub action: &'static str,
    pub candidate: CandidateRecord,
    pub lifetime_ms: u64,
    pub steps: Vec<StepRecord>,
    pub status_before: Option<StatusRecord>,
    pub candidate_generation: Option<u64>,
    pub deadline_ms: Option<u64>,
    /// The configuration the board reported as applied. The V4's provisional answer carries
    /// no configuration, so this stays empty and says so in `notes`.
    pub applied_reported: Option<CandidateRecord>,
    pub status_during: Option<StatusRecord>,
    pub status_after: Option<StatusRecord>,
    pub known_good_after: Option<u64>,
    pub outcome: String,
    pub notes: Vec<String>,
}
