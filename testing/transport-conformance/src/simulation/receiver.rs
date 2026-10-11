// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::scheduler::block_counts;
use super::types::{
    GenerationRxReport, MAX_ACTIVE, MAX_DECODE_WORK, MAX_PACKETS, ReceiverReport, ReceiverSpec,
    Scenario,
};
use super::wire::{self, AuthenticatedFrame, Manifest, WireError};
use crate::{checked_packet, require};
use raptorq::Decoder;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollectedPacket {
    pub slot: u32,
    pub wire_hex: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiveSave {
    pub manifest_hex: String,
    pub packets: Vec<CollectedPacket>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiverSave {
    pub id: String,
    pub report: ReceiverReport,
    pub active: Vec<ReceiveSave>,
}

struct ReceiveState {
    manifest: Manifest,
    saved: ReceiveSave,
    seen: BTreeSet<(u8, u32)>,
    decoder: Option<Decoder>,
    completed: bool,
}

pub(crate) struct Receiver {
    pub spec: ReceiverSpec,
    pub report: ReceiverReport,
    active: BTreeMap<u64, ReceiveState>,
}

pub(crate) struct ValidatedReceiver {
    saved: ReceiverSave,
    manifests: Vec<Manifest>,
}

impl Receiver {
    pub fn new(spec: &ReceiverSpec, scenario: &Scenario) -> Result<Self, String> {
        let generations = scenario
            .generations
            .iter()
            .map(|g| {
                Ok(GenerationRxReport {
                    id: g.id,
                    object_octets: g.object_octets,
                    required_source_symbols: block_counts(scenario.oti(g)?).iter().sum(),
                    ..Default::default()
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            spec: spec.clone(),
            report: ReceiverReport {
                id: spec.id.clone(),
                radios: spec.radios,
                generations,
                ..Default::default()
            },
            active: BTreeMap::new(),
        })
    }

    pub fn tuned_channels(&self, slot: u32, scenario: &Scenario, drift: bool) -> Vec<u8> {
        if slot < self.spec.join_slot || self.spec.sleep.iter().any(|range| range.contains(slot)) {
            return Vec::new();
        }
        // Enough radios can stay assigned one per channel; no synthetic retune loss.
        if self.spec.radios == scenario.channels {
            return (0..scenario.channels).collect();
        }
        let elapsed = slot - self.spec.join_slot;
        let steps = if drift && self.spec.drift_every_slots > 0 {
            elapsed / self.spec.drift_every_slots
        } else {
            0
        };
        let clock = i64::from(elapsed)
            + i64::from(self.spec.phase)
            + i64::from(steps) * i64::from(self.spec.drift_slots);
        let dwell = i64::from(self.spec.dwell_slots);
        // Nominal dwell guards are an authored opportunity model. Anchor
        // pre-tuning and individual synthesizer settle transitions are not modeled.
        if clock.rem_euclid(dwell) < i64::from(self.spec.retune_guard_slots) {
            return Vec::new();
        }
        let anchor_clock = i64::from(slot) + i64::from(steps) * i64::from(self.spec.drift_slots);
        let base = if anchor_clock.rem_euclid(i64::from(scenario.manifest_interval)) == 0 {
            0
        } else {
            clock.div_euclid(dwell)
        };
        (0..self.spec.radios)
            .map(|radio| (base + i64::from(radio)).rem_euclid(i64::from(scenario.channels)) as u8)
            .collect()
    }

    pub fn tick(&mut self, slot: u32, scenario: &Scenario) {
        self.report.symbol_opportunity_upper_bound +=
            self.tuned_channels(slot, scenario, true).len() as u64;
        self.active.retain(|_, state| slot < state.manifest.expiry);
        for (spec, report) in scenario
            .generations
            .iter()
            .zip(&mut self.report.generations)
        {
            if spec.publish_slot <= slot && spec.expires_slot <= slot {
                report.expired = true;
                report.incomplete_expired = report.completion_slot.is_none();
            }
        }
    }

    // Return a stable event code for the deterministic trace, including losses.
    pub fn capture(
        &mut self,
        slot: u32,
        channel: u8,
        bytes: &[u8],
        erased: bool,
        scenario: &Scenario,
    ) -> u8 {
        self.report.offered_frames += 1;
        if slot < self.spec.join_slot || self.spec.sleep.iter().any(|range| range.contains(slot)) {
            self.report.late_or_sleep_misses += 1;
            return 0;
        }
        let tuned = self.tuned_channels(slot, scenario, true);
        if tuned.is_empty() {
            self.report.retune_misses += 1;
            return 1;
        }
        if !tuned.contains(&channel) {
            self.report.channel_misses += 1;
            if self
                .tuned_channels(slot, scenario, false)
                .contains(&channel)
            {
                self.report.drift_misses += 1;
            }
            return 2;
        }
        self.report.tuned_frames += 1;
        if erased {
            self.report.erased_frames += 1;
            return 3;
        }
        self.report.wire_octets += bytes.len() as u64;
        15
    }

    pub fn deliver(&mut self, slot: u32, bytes: &[u8], scenario: &Scenario) -> Result<u8, String> {
        match wire::verify(bytes, &wire::namespace(scenario.namespace)) {
            Err(WireError::InvalidSignature) => {
                self.report.invalid_signatures += 1;
                Ok(4)
            }
            Err(_) => {
                self.report.malformed_frames += 1;
                Ok(5)
            }
            Ok(AuthenticatedFrame::Manifest(manifest)) => {
                self.manifest(slot, bytes, manifest, scenario)
            }
            Ok(AuthenticatedFrame::Symbol {
                generation,
                manifest_hash,
                packet,
            }) => self.symbol(slot, bytes, generation, manifest_hash, &packet),
        }
    }

    fn manifest(
        &mut self,
        slot: u32,
        bytes: &[u8],
        manifest: Manifest,
        scenario: &Scenario,
    ) -> Result<u8, String> {
        if slot < manifest.publish || slot >= manifest.expiry {
            self.report.expired_frames += 1;
            return Ok(6);
        }
        if !wire::matches_scenario(&manifest, scenario) {
            self.report.malformed_frames += 1;
            return Ok(5);
        }
        if let Some(state) = self.active.get(&manifest.generation) {
            if state.manifest.body_hash != manifest.body_hash {
                self.report.malformed_frames += 1;
                return Ok(5);
            }
        } else {
            if self.active.len() >= MAX_ACTIVE {
                self.report.budget_rejections += 1;
                return Ok(7);
            }
            self.active.insert(
                manifest.generation,
                ReceiveState {
                    manifest: manifest.clone(),
                    saved: ReceiveSave {
                        manifest_hex: hex::encode(bytes),
                        packets: Vec::new(),
                    },
                    seen: BTreeSet::new(),
                    decoder: None,
                    completed: false,
                },
            );
        }
        self.report.accepted_manifests += 1;
        self.report
            .generations
            .iter_mut()
            .find(|g| g.id == manifest.generation)
            .ok_or("manifest accounting missing")?
            .manifest_seen = true;
        Ok(8)
    }

    fn symbol(
        &mut self,
        slot: u32,
        bytes: &[u8],
        generation: u64,
        hash: [u8; 32],
        packet_bytes: &[u8],
    ) -> Result<u8, String> {
        let Some(state) = self.active.get_mut(&generation) else {
            if self
                .report
                .generations
                .iter()
                .any(|g| g.id == generation && g.expired)
            {
                self.report.expired_frames += 1;
                return Ok(6);
            }
            self.report.unknown_generation_symbols += 1;
            return Ok(9);
        };
        if slot < state.manifest.publish || slot >= state.manifest.expiry {
            self.report.expired_frames += 1;
            return Ok(6);
        }
        if state.manifest.body_hash != hash {
            self.report.malformed_frames += 1;
            return Ok(5);
        }
        let packet = match checked_packet(packet_bytes, state.manifest.oti) {
            Ok(packet) => packet,
            Err(_) => {
                self.report.malformed_frames += 1;
                return Ok(5);
            }
        };
        let id = packet.payload_id();
        let key = (id.source_block_number(), id.encoding_symbol_id());
        if state.seen.contains(&key) {
            self.report.duplicate_symbols += 1;
            return Ok(10);
        }
        if state.completed {
            self.report.completed_generation_symbols += 1;
            return Ok(11);
        }
        if self.report.decode_work >= MAX_DECODE_WORK || state.saved.packets.len() >= MAX_PACKETS {
            self.report.budget_rejections += 1;
            return Ok(7);
        }
        if state.decoder.is_none() {
            state.decoder = Some(Decoder::new(state.manifest.oti.to_crate()?));
            self.report.decoder_allocations += 1;
        }
        state.seen.insert(key);
        state.saved.packets.push(CollectedPacket {
            slot,
            wire_hex: hex::encode(bytes),
        });
        self.report.accepted_symbols += 1;
        self.report.decode_work += 1;
        let report = self
            .report
            .generations
            .iter_mut()
            .find(|g| g.id == generation)
            .ok_or("symbol accounting missing")?;
        report.unique_symbols += 1;
        if let Some(object) = state
            .decoder
            .as_mut()
            .ok_or("decoder missing")?
            .decode(packet)
        {
            if object.len() as u64 != state.manifest.oti.transfer_length
                || blake3::hash(&object).as_bytes() != &state.manifest.digest
            {
                self.report.reconstructed_digest_mismatches += 1;
                return Ok(12);
            }
            report.completion_slot = Some(slot);
            report.reconstructed_blake3 = Some(hex::encode(state.manifest.digest));
            state.completed = true;
            state.decoder = None;
            return Ok(13);
        }
        Ok(14)
    }

    pub fn update_demand(&mut self, slot: u32, scenario: &Scenario) {
        self.report.produced_object_octets = scenario
            .generations
            .iter()
            .filter(|g| g.publish_slot <= slot)
            .map(|g| u64::from(g.object_octets))
            .sum();
        self.report.completed_object_octets = self
            .report
            .generations
            .iter()
            .filter(|g| g.completion_slot.is_some())
            .map(|g| u64::from(g.object_octets))
            .sum();
        self.report.unmet_object_octets = self
            .report
            .produced_object_octets
            .saturating_sub(self.report.completed_object_octets);
    }

    pub fn save(&self) -> ReceiverSave {
        ReceiverSave {
            id: self.spec.id.clone(),
            report: self.report.clone(),
            active: self.active.values().map(|g| g.saved.clone()).collect(),
        }
    }

    pub fn validate_saved(
        saved: ReceiverSave,
        scenario: &Scenario,
        next_slot: u32,
    ) -> Result<ValidatedReceiver, String> {
        require(
            scenario.receivers.iter().any(|r| r.id == saved.id) && saved.report.id == saved.id,
            "checkpoint receiver identity",
        )?;
        require(
            saved.active.len() <= MAX_ACTIVE
                && saved.report.generations.len() == scenario.generations.len()
                && saved.report.decode_work <= MAX_DECODE_WORK
                && saved.report.accepted_symbols == saved.report.decode_work
                && saved.report.decoder_allocations <= saved.report.decode_work,
            "checkpoint receiver work/accounting bounds",
        )?;
        let mut manifests = Vec::new();
        let mut generations = BTreeSet::new();
        let mut packets = 0usize;
        for state in &saved.active {
            require(
                state.manifest_hex.len() <= 1024 && state.packets.len() <= MAX_PACKETS,
                "checkpoint receiver packet/manifest budget",
            )?;
            packets += state.packets.len();
            require(
                packets <= MAX_PACKETS,
                "checkpoint collected packet total budget",
            )?;
            let bytes = hex::decode(&state.manifest_hex).map_err(|e| e.to_string())?;
            let AuthenticatedFrame::Manifest(manifest) =
                wire::verify(&bytes, &wire::namespace(scenario.namespace))
                    .map_err(|_| "checkpoint receiver manifest authentication")?
            else {
                return Err("checkpoint receiver manifest kind".into());
            };
            require(
                wire::matches_scenario(&manifest, scenario)
                    && generations.insert(manifest.generation)
                    && manifest.publish < next_slot
                    && next_slot <= manifest.expiry,
                "checkpoint receiver manifest context",
            )?;
            let mut seen = BTreeSet::new();
            for collected in &state.packets {
                require(
                    collected.slot < next_slot
                        && collected.slot >= manifest.publish
                        && collected.slot < manifest.expiry
                        && collected.wire_hex.len() <= 1024,
                    "checkpoint collected packet time/length",
                )?;
                let raw = hex::decode(&collected.wire_hex).map_err(|e| e.to_string())?;
                let AuthenticatedFrame::Symbol {
                    generation,
                    manifest_hash,
                    packet,
                } = wire::verify(&raw, &wire::namespace(scenario.namespace))
                    .map_err(|_| "checkpoint symbol authentication")?
                else {
                    return Err("checkpoint collected symbol kind".into());
                };
                require(
                    generation == manifest.generation && manifest_hash == manifest.body_hash,
                    "checkpoint symbol generation binding",
                )?;
                let checked = checked_packet(&packet, manifest.oti)?;
                let id = checked.payload_id();
                require(
                    seen.insert((id.source_block_number(), id.encoding_symbol_id())),
                    "checkpoint duplicate collected symbol",
                )?;
            }
            manifests.push(manifest);
        }
        Ok(ValidatedReceiver { saved, manifests })
    }

    pub fn restore(
        validated: ValidatedReceiver,
        scenario: &Scenario,
    ) -> Result<(Self, u64), String> {
        let saved = validated.saved;
        let spec = scenario
            .receivers
            .iter()
            .find(|r| r.id == saved.id)
            .ok_or("receiver restore spec")?
            .clone();
        let mut active = BTreeMap::new();
        let mut replay_work = 0;
        for (state, manifest) in saved.active.into_iter().zip(validated.manifests) {
            let mut seen = BTreeSet::new();
            let mut decoder = if state.packets.is_empty() {
                None
            } else {
                Some(Decoder::new(manifest.oti.to_crate()?))
            };
            let mut completion = None;
            let mut digest = None;
            for collected in &state.packets {
                let bytes = hex::decode(&collected.wire_hex).map_err(|e| e.to_string())?;
                let AuthenticatedFrame::Symbol { packet, .. } =
                    wire::verify(&bytes, &wire::namespace(scenario.namespace))
                        .map_err(|_| "validated checkpoint symbol changed")?
                else {
                    return Err("validated checkpoint kind changed".into());
                };
                let checked = checked_packet(&packet, manifest.oti)?;
                let id = checked.payload_id();
                seen.insert((id.source_block_number(), id.encoding_symbol_id()));
                replay_work += 1;
                if let Some(object) = decoder
                    .as_mut()
                    .ok_or("replay decoder missing")?
                    .decode(checked)
                {
                    require(
                        object.len() as u64 == manifest.oti.transfer_length
                            && blake3::hash(&object).as_bytes() == &manifest.digest,
                        "checkpoint replay object digest",
                    )?;
                    completion = Some(collected.slot);
                    digest = Some(hex::encode(manifest.digest));
                }
            }
            let report = saved
                .report
                .generations
                .iter()
                .find(|g| g.id == manifest.generation)
                .ok_or("checkpoint generation report missing")?;
            require(
                report.unique_symbols as usize == state.packets.len()
                    && report.completion_slot == completion
                    && report.reconstructed_blake3 == digest,
                "checkpoint replay differs from recorded terminal state",
            )?;
            let completed = completion.is_some();
            if completed {
                decoder = None;
            }
            active.insert(
                manifest.generation,
                ReceiveState {
                    manifest,
                    saved: state,
                    seen,
                    decoder,
                    completed,
                },
            );
        }
        Ok((
            Self {
                spec,
                report: saved.report,
                active,
            },
            replay_work,
        ))
    }
}
