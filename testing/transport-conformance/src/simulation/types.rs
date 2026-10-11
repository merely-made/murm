// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{Oti, require};
use serde::{Deserialize, Serialize};

pub const SCENARIO_SCHEMA: &str = "transport-conformance.simulation/v1";
pub const MAX_PEER_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = 512;
pub(crate) const MAX_ACTIVE: usize = 8;
pub(crate) const MAX_DECODE_WORK: u64 = 4096;
pub(crate) const MAX_PACKETS: usize = 4096;
pub(crate) const MAX_GENERATED: u64 = 16_384;

fn default_symbol_size() -> u16 {
    128
}
fn default_source_blocks() -> u8 {
    1
}
fn default_slot_budget() -> u16 {
    512
}
fn default_dwell() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema: String,
    pub id: String,
    pub namespace: u64,
    pub slots: u32,
    pub channels: u8,
    #[serde(default = "default_symbol_size")]
    pub symbol_size: u16,
    #[serde(default = "default_source_blocks")]
    pub source_blocks: u8,
    #[serde(default = "default_slot_budget")]
    pub slot_octet_budget: u16,
    pub manifest_interval: u32,
    pub repair_floor: u8,
    pub generations: Vec<GenerationSpec>,
    pub receivers: Vec<ReceiverSpec>,
    #[serde(default)]
    pub impairments: Impairments,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSpec {
    pub id: u64,
    pub publish_slot: u32,
    pub expires_slot: u32,
    pub object_octets: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverSpec {
    pub id: String,
    pub radios: u8,
    #[serde(default)]
    pub join_slot: u32,
    #[serde(default = "default_dwell")]
    pub dwell_slots: u32,
    #[serde(default)]
    pub retune_guard_slots: u32,
    #[serde(default)]
    pub phase: u32,
    #[serde(default)]
    pub sleep: Vec<SlotRange>,
    #[serde(default)]
    pub drift_every_slots: u32,
    #[serde(default)]
    pub drift_slots: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlotRange {
    pub start: u32,
    pub end: u32,
}

impl SlotRange {
    pub(crate) fn contains(self, slot: u32) -> bool {
        self.start <= slot && slot < self.end
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Impairments {
    pub loss_seed: u64,
    pub loss_per_million: u32,
    pub erase_every: u32,
    pub bursts: Vec<SlotRange>,
    pub duplicate_every: u32,
    pub reorder_delay_slots: u32,
    pub tamper_every: u32,
    pub malformed_every: u32,
}

impl Scenario {
    pub fn validate(&self) -> Result<(), String> {
        require(self.schema == SCENARIO_SCHEMA, "unknown simulation schema")?;
        require(
            !self.id.is_empty() && self.id.len() <= 128,
            "scenario ID budget",
        )?;
        require((1..=4096).contains(&self.slots), "slot count budget")?;
        require((1..=8).contains(&self.channels), "channel count budget")?;
        require(
            u64::from(self.slots) * u64::from(self.channels) <= MAX_GENERATED,
            "generated frame work budget",
        )?;
        require(
            (128..=512).contains(&self.slot_octet_budget),
            "slot octet budget",
        )?;
        require(
            self.manifest_interval > 0 && self.manifest_interval <= self.slots,
            "manifest interval budget",
        )?;
        require(
            self.repair_floor <= self.channels,
            "retained repair floor must be <=channels; zero is the disabled-floor counterfactual",
        )?;
        require(
            !self.generations.is_empty() && self.generations.len() <= 32,
            "generation count budget",
        )?;
        require(
            !self.receivers.is_empty() && self.receivers.len() <= 8,
            "receiver count budget",
        )?;
        let mut ids = std::collections::BTreeSet::new();
        for generation in &self.generations {
            require(ids.insert(generation.id), "duplicate generation namespace")?;
            require(
                generation.publish_slot < generation.expires_slot
                    && generation.expires_slot <= 8192,
                "generation lifetime budget",
            )?;
            self.oti(generation)?.bounded()?;
        }
        for slot in 0..self.slots {
            let active: Vec<_> = self
                .generations
                .iter()
                .filter(|g| g.publish_slot <= slot && slot < g.expires_slot)
                .collect();
            require(active.len() <= MAX_ACTIVE, "active generation budget")?;
            require(
                active
                    .iter()
                    .map(|g| u64::from(g.object_octets))
                    .sum::<u64>()
                    <= 512 * 1024,
                "retained-original byte budget",
            )?;
        }
        let mut receivers = std::collections::BTreeSet::new();
        for receiver in &self.receivers {
            require(
                !receiver.id.is_empty()
                    && receiver.id.len() <= 128
                    && receivers.insert(&receiver.id),
                "receiver ID budget or duplicate",
            )?;
            require(
                receiver.radios > 0 && receiver.radios <= self.channels,
                "receiver radio count budget",
            )?;
            require(
                receiver.dwell_slots > 0
                    && receiver.dwell_slots <= 4096
                    && receiver.retune_guard_slots < receiver.dwell_slots,
                "receiver dwell/retune guard budget",
            )?;
            require(
                receiver.phase <= 4096 && receiver.join_slot <= 8192,
                "receiver phase/join budget",
            )?;
            require(
                receiver.drift_slots.unsigned_abs() <= 8 && receiver.drift_every_slots <= 8192,
                "receiver drift budget",
            )?;
            validate_ranges(&receiver.sleep)?;
        }
        validate_ranges(&self.impairments.bursts)?;
        require(
            self.impairments.loss_per_million <= 1_000_000,
            "seeded erasure probability budget",
        )?;
        require(
            self.impairments.reorder_delay_slots <= 16,
            "reorder queue delay budget",
        )?;
        Ok(())
    }

    pub(crate) fn oti(&self, generation: &GenerationSpec) -> Result<Oti, String> {
        Oti {
            transfer_length: u64::from(generation.object_octets),
            symbol_size: self.symbol_size,
            source_blocks: self.source_blocks,
            sub_blocks: 1,
            alignment: 4,
        }
        .bounded()
    }
}

fn validate_ranges(ranges: &[SlotRange]) -> Result<(), String> {
    require(ranges.len() <= 32, "slot range count budget")?;
    for range in ranges {
        require(
            range.start < range.end && range.end <= 8192,
            "invalid half-open slot range",
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GenerationTxReport {
    pub id: u64,
    pub object_octets: u32,
    pub total_source_symbols: u32,
    pub fresh_sent: u32,
    pub repairs_sent: u32,
    pub fresh_backlog: u32,
    pub expired: bool,
    pub esi_exhausted: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TransmitterReport {
    pub manifest_frames: u64,
    pub fresh_frames: u64,
    pub repair_frames: u64,
    pub wire_octets: u64,
    pub channel_wire_octets: Vec<u64>,
    pub generations: Vec<GenerationTxReport>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GenerationRxReport {
    pub id: u64,
    pub object_octets: u32,
    pub required_source_symbols: u32,
    pub unique_symbols: u32,
    pub manifest_seen: bool,
    pub completion_slot: Option<u32>,
    pub reconstructed_blake3: Option<String>,
    pub expired: bool,
    pub incomplete_expired: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReceiverReport {
    pub id: String,
    pub radios: u8,
    pub offered_frames: u64,
    pub tuned_frames: u64,
    pub late_or_sleep_misses: u64,
    pub retune_misses: u64,
    pub channel_misses: u64,
    pub drift_misses: u64,
    pub erased_frames: u64,
    pub wire_octets: u64,
    pub accepted_manifests: u64,
    pub accepted_symbols: u64,
    pub duplicate_symbols: u64,
    pub completed_generation_symbols: u64,
    pub unknown_generation_symbols: u64,
    pub invalid_signatures: u64,
    pub malformed_frames: u64,
    pub expired_frames: u64,
    pub budget_rejections: u64,
    pub decode_work: u64,
    pub decoder_allocations: u64,
    pub reconstructed_digest_mismatches: u64,
    pub symbol_opportunity_upper_bound: u64,
    pub produced_object_octets: u64,
    pub completed_object_octets: u64,
    pub unmet_object_octets: u64,
    pub generations: Vec<GenerationRxReport>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunReport {
    pub schema: String,
    pub id: String,
    pub slots_executed: u32,
    pub namespace_blake3: String,
    pub authority_public_key_hex: String,
    pub transmitter: TransmitterReport,
    pub receivers: Vec<ReceiverReport>,
    pub demand_by_slot: Vec<DemandSample>,
    pub semantic_terminal_blake3: String,
    pub trace_blake3: String,
    pub resumed_from_slot: Option<u32>,
    pub checkpoint_replay_decode_work: u64,
    pub pending_delivery_frames: usize,
    pub physical_throughput_claim: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DemandSample {
    pub slot: u32,
    pub produced_object_octets: u64,
    pub receiver_unmet_object_octets: Vec<u64>,
}
