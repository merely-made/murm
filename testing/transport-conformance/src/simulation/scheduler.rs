// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::types::{GenerationTxReport, MAX_ACTIVE, MAX_GENERATED, Scenario, TransmitterReport};
use super::wire::{self, AuthenticatedFrame, Manifest};
use crate::{Oti, checked_packet, checked_payload_id, interop, require};
use raptorq::Encoder;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum Lane {
    Manifest,
    Fresh,
    Repair,
}

#[derive(Clone, Debug)]
pub(crate) struct Transmission {
    pub channel: u8,
    pub lane: Lane,
    pub generation: u64,
    pub wire: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceSave {
    pub generation: u64,
    pub original_hex: String,
    pub manifest_hex: String,
    pub fresh_cursor: u32,
    pub repair_next: Vec<u32>,
    pub repair_block: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SchedulerSave {
    pub active: Vec<SourceSave>,
    pub manifest_order: Vec<u64>,
    pub repair_order: Vec<u64>,
    pub ordinal: u64,
    pub report: TransmitterReport,
}

struct Source {
    saved: SourceSave,
    manifest: Manifest,
    encoder: Encoder,
    block_k: Vec<u32>,
}

pub(crate) struct Scheduler {
    active: BTreeMap<u64, Source>,
    manifest_order: VecDeque<u64>,
    repair_order: VecDeque<u64>,
    pub ordinal: u64,
    pub report: TransmitterReport,
}

pub(crate) fn block_counts(oti: Oti) -> Vec<u32> {
    let symbols = oti.transfer_length.div_ceil(u64::from(oti.symbol_size));
    let blocks = u64::from(oti.source_blocks);
    (0..blocks)
        .map(|i| (symbols / blocks + u64::from(i < symbols % blocks)) as u32)
        .collect()
}

impl Scheduler {
    pub fn new(scenario: &Scenario) -> Result<Self, String> {
        // Check signed frame costs for every declared generation before any encoder.
        for spec in &scenario.generations {
            let object = wire::original(spec, &wire::namespace(scenario.namespace));
            let (manifest, bytes) = wire::manifest(spec, scenario, &object)?;
            let packet = vec![0; 4 + usize::from(scenario.symbol_size)];
            let symbol = wire::symbol(
                spec.id,
                &wire::namespace(scenario.namespace),
                &manifest.body_hash,
                &packet,
            );
            require(
                bytes.len() <= usize::from(scenario.slot_octet_budget)
                    && symbol.len() <= usize::from(scenario.slot_octet_budget),
                "signed frame exceeds per-channel slot byte budget",
            )?;
        }
        Ok(Self {
            active: BTreeMap::new(),
            manifest_order: VecDeque::new(),
            repair_order: VecDeque::new(),
            ordinal: 0,
            report: TransmitterReport {
                channel_wire_octets: vec![0; usize::from(scenario.channels)],
                ..Default::default()
            },
        })
    }

    pub fn tick(&mut self, slot: u32, scenario: &Scenario) -> Result<Vec<Transmission>, String> {
        let expired: Vec<_> = self
            .active
            .iter()
            .filter_map(|(id, source)| (slot >= source.manifest.expiry).then_some(*id))
            .collect();
        for id in expired {
            self.active.remove(&id);
            self.manifest_order.retain(|candidate| *candidate != id);
            self.repair_order.retain(|candidate| *candidate != id);
            if let Some(report) = self.report.generations.iter_mut().find(|g| g.id == id) {
                report.expired = true;
            }
        }
        for spec in scenario
            .generations
            .iter()
            .filter(|g| g.publish_slot == slot)
        {
            let original = wire::original(spec, &wire::namespace(scenario.namespace));
            let (manifest, manifest_wire) = wire::manifest(spec, scenario, &original)?;
            let block_k = block_counts(manifest.oti);
            let total = block_k.iter().sum();
            let saved = SourceSave {
                generation: spec.id,
                original_hex: hex::encode(&original),
                manifest_hex: hex::encode(&manifest_wire),
                fresh_cursor: 0,
                repair_next: block_k.clone(),
                repair_block: 0,
            };
            let source = Source {
                encoder: Encoder::new(&original, manifest.oti.to_crate()?),
                saved,
                manifest,
                block_k,
            };
            require(self.active.len() < MAX_ACTIVE, "active encoder budget")?;
            self.active.insert(spec.id, source);
            self.manifest_order.push_back(spec.id);
            self.repair_order.push_back(spec.id);
            self.report.generations.push(GenerationTxReport {
                id: spec.id,
                object_octets: spec.object_octets,
                total_source_symbols: total,
                fresh_backlog: total,
                ..Default::default()
            });
        }
        let mut frames = Vec::new();
        let mut channels: Vec<_> = (0..scenario.channels).collect();
        if slot.is_multiple_of(scenario.manifest_interval) && !self.active.is_empty() {
            let id = self
                .manifest_order
                .pop_front()
                .ok_or("manifest round-robin queue empty")?;
            self.manifest_order.push_back(id);
            let source = self.active.get(&id).ok_or("manifest generation vanished")?;
            frames.push(Transmission {
                channel: 0,
                lane: Lane::Manifest,
                generation: id,
                wire: hex::decode(&source.saved.manifest_hex).map_err(|e| e.to_string())?,
            });
            channels.remove(0);
        }
        for (index, channel) in channels.into_iter().enumerate() {
            let prefer_repair = index < usize::from(scenario.repair_floor);
            let selected = if prefer_repair {
                match self.next_repair(scenario)? {
                    Some(frame) => Some(frame),
                    None => self.next_fresh(scenario)?,
                }
            } else {
                match self.next_fresh(scenario)? {
                    Some(frame) => Some(frame),
                    None => self.next_repair(scenario)?,
                }
            };
            if let Some(mut frame) = selected {
                frame.channel = channel;
                frames.push(frame);
            }
        }
        for frame in &frames {
            require(
                frame.wire.len() <= usize::from(scenario.slot_octet_budget),
                "frame exceeds slot budget",
            )?;
            require(self.ordinal < MAX_GENERATED, "generated frame budget")?;
            self.ordinal += 1;
            self.report.wire_octets += frame.wire.len() as u64;
            self.report.channel_wire_octets[usize::from(frame.channel)] += frame.wire.len() as u64;
            match frame.lane {
                Lane::Manifest => self.report.manifest_frames += 1,
                Lane::Fresh => self.report.fresh_frames += 1,
                Lane::Repair => self.report.repair_frames += 1,
            }
        }
        Ok(frames)
    }

    fn next_fresh(&mut self, scenario: &Scenario) -> Result<Option<Transmission>, String> {
        let Some(id) = self
            .active
            .iter()
            .filter(|(_, s)| s.saved.fresh_cursor < s.block_k.iter().sum())
            .min_by_key(|(id, s)| (s.manifest.publish, **id))
            .map(|(id, _)| *id)
        else {
            return Ok(None);
        };
        let source = self
            .active
            .get_mut(&id)
            .ok_or("fresh generation vanished")?;
        let mut remaining = source.saved.fresh_cursor;
        let mut block = 0;
        while remaining >= source.block_k[block] {
            remaining -= source.block_k[block];
            block += 1;
        }
        source.saved.fresh_cursor += 1;
        let report = self
            .report
            .generations
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or("missing generation accounting")?;
        report.fresh_sent += 1;
        report.fresh_backlog -= 1;
        Self::frame(source, scenario, block, remaining, Lane::Fresh).map(Some)
    }

    fn next_repair(&mut self, scenario: &Scenario) -> Result<Option<Transmission>, String> {
        for _ in 0..self.repair_order.len() {
            let id = self
                .repair_order
                .pop_front()
                .ok_or("repair round-robin queue empty")?;
            self.repair_order.push_back(id);
            let source = self
                .active
                .get_mut(&id)
                .ok_or("repair generation vanished")?;
            for _ in 0..source.block_k.len() {
                let block = source.saved.repair_block % source.block_k.len();
                source.saved.repair_block = (block + 1) % source.block_k.len();
                let esi = source.saved.repair_next[block];
                if esi > 4095 {
                    continue;
                }
                source.saved.repair_next[block] += 1;
                self.report
                    .generations
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or("repair accounting missing")?
                    .repairs_sent += 1;
                return Self::frame(source, scenario, block, esi, Lane::Repair).map(Some);
            }
            self.report
                .generations
                .iter_mut()
                .find(|g| g.id == id)
                .ok_or("repair accounting missing")?
                .esi_exhausted = true;
        }
        Ok(None)
    }

    fn frame(
        source: &Source,
        scenario: &Scenario,
        block: usize,
        esi: u32,
        lane: Lane,
    ) -> Result<Transmission, String> {
        let id = checked_payload_id(block as u8, esi)?;
        // Encoder is derived from the retained original, never accumulated repair bytes.
        let packet = interop::encode_ids(&source.encoder, &[id])?
            .pop()
            .ok_or("encoder returned no symbol")?
            .serialize();
        checked_packet(&packet, source.manifest.oti)?;
        Ok(Transmission {
            channel: 0,
            lane,
            generation: source.saved.generation,
            wire: wire::symbol(
                source.saved.generation,
                &wire::namespace(scenario.namespace),
                &source.manifest.body_hash,
                &packet,
            ),
        })
    }

    pub fn save(&self) -> SchedulerSave {
        SchedulerSave {
            active: self.active.values().map(|s| s.saved.clone()).collect(),
            manifest_order: self.manifest_order.iter().copied().collect(),
            repair_order: self.repair_order.iter().copied().collect(),
            ordinal: self.ordinal,
            report: self.report.clone(),
        }
    }

    pub fn restore(
        saved: SchedulerSave,
        scenario: &Scenario,
        next_slot: u32,
    ) -> Result<Self, String> {
        require(
            saved.active.len() <= MAX_ACTIVE && saved.ordinal <= MAX_GENERATED,
            "checkpoint scheduler bounds",
        )?;
        require(
            saved.report.channel_wire_octets.len() == usize::from(scenario.channels)
                && saved.report.generations.len() <= 32,
            "checkpoint transmitter accounting bounds",
        )?;
        let mut validated = Vec::new();
        let mut ids = BTreeSet::new();
        for source in saved.active {
            require(
                ids.insert(source.generation)
                    && source.original_hex.len() <= 131072
                    && source.manifest_hex.len() <= 1024,
                "checkpoint original/manifest bounds",
            )?;
            let original = hex::decode(&source.original_hex).map_err(|e| e.to_string())?;
            let bytes = hex::decode(&source.manifest_hex).map_err(|e| e.to_string())?;
            let AuthenticatedFrame::Manifest(manifest) =
                wire::verify(&bytes, &wire::namespace(scenario.namespace))
                    .map_err(|_| "checkpoint source manifest authentication")?
            else {
                return Err("checkpoint source manifest kind".into());
            };
            require(
                wire::matches_scenario(&manifest, scenario)
                    && source.generation == manifest.generation
                    && manifest.publish < next_slot
                    && next_slot <= manifest.expiry
                    && original.len() as u64 == manifest.oti.transfer_length
                    && blake3::hash(&original).as_bytes() == &manifest.digest,
                "checkpoint retained original differs from signed manifest",
            )?;
            let k = block_counts(manifest.oti);
            require(
                source.fresh_cursor <= k.iter().sum()
                    && source.repair_next.len() == k.len()
                    && source.repair_block < k.len(),
                "checkpoint source cursor bounds",
            )?;
            for (next, count) in source.repair_next.iter().zip(&k) {
                require(
                    *next >= *count && *next <= 4096,
                    "checkpoint repair ESI bounds",
                )?;
            }
            validated.push((source, original, manifest, k));
        }
        for order in [&saved.manifest_order, &saved.repair_order] {
            require(
                order.len() == ids.len() && order.iter().copied().collect::<BTreeSet<_>>() == ids,
                "checkpoint round-robin membership",
            )?;
        }
        // Validate every retained original first; only then allocate codecs.
        let mut active = BTreeMap::new();
        for (source, original, manifest, block_k) in validated {
            let encoder = Encoder::new(&original, manifest.oti.to_crate()?);
            active.insert(
                source.generation,
                Source {
                    saved: source,
                    manifest,
                    encoder,
                    block_k,
                },
            );
        }
        Ok(Self {
            active,
            manifest_order: saved.manifest_order.into(),
            repair_order: saved.repair_order.into(),
            ordinal: saved.ordinal,
            report: saved.report,
        })
    }
}
