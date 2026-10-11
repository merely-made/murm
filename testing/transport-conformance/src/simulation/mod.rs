// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Exploratory retained-repair design tool. Slots and byte costs are authored
//! units; these experiments establish no RF timing, throughput or application admission.

pub mod campaign;
mod checkpoint;
mod receiver;
mod scheduler;
mod types;
mod wire;

use crate::require;
pub use checkpoint::{Checkpoint, checkpoint_at, resume, run_peer};
use receiver::Receiver;
use scheduler::Scheduler;
use serde::{Deserialize, Serialize};
use types::{DemandSample, MAX_FRAME_BYTES};
pub use types::{
    GenerationSpec, Impairments, MAX_PEER_BYTES, MAX_RETURNED_OBJECT_OCTETS, ReceiverSpec,
    RunReport, Scenario, SlotRange,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Delivery {
    pub receiver: usize,
    pub captured_slot: u32,
    pub due: u32,
    pub channel: u8,
    pub ordinal: u64,
    pub duplicate: bool,
    pub wire_hex: String,
}

pub(crate) struct Engine {
    pub scenario: Scenario,
    pub next_slot: u32,
    pub scheduler: Scheduler,
    pub receivers: Vec<Receiver>,
    pub pending: Vec<Delivery>,
    pub trace: [u8; 32],
    pub demand: Vec<DemandSample>,
    pub resumed_from: Option<u32>,
    pub replay_work: u64,
}

pub fn simulate(scenario: &Scenario) -> Result<RunReport, String> {
    let mut engine = Engine::new(scenario.clone())?;
    engine.run_until(scenario.slots)?;
    engine.report()
}

impl Engine {
    fn new(scenario: Scenario) -> Result<Self, String> {
        scenario.validate()?;
        let scheduler = Scheduler::new(&scenario)?;
        let receivers = scenario
            .receivers
            .iter()
            .map(|r| Receiver::new(r, &scenario))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            scenario,
            next_slot: 0,
            scheduler,
            receivers,
            pending: Vec::new(),
            trace: [0; 32],
            demand: Vec::new(),
            resumed_from: None,
            replay_work: 0,
        })
    }

    pub fn run_until(&mut self, next_slot: u32) -> Result<(), String> {
        require(
            next_slot >= self.next_slot && next_slot <= self.scenario.slots,
            "run cursor outside scenario",
        )?;
        while self.next_slot < next_slot {
            self.tick()?;
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<(), String> {
        let slot = self.next_slot;
        let frames = self.scheduler.tick(slot, &self.scenario)?;
        for receiver in &mut self.receivers {
            receiver.tick(slot, &self.scenario);
        }
        for index in 0..self.receivers.len() {
            let channels = self.receivers[index].tuned_channels(slot, &self.scenario, true);
            self.event(
                b"tune",
                &[
                    &slot.to_be_bytes(),
                    &(index as u64).to_be_bytes(),
                    &channels,
                ],
            );
        }
        let ordinal_start = self.scheduler.ordinal - frames.len() as u64;
        for (offset, frame) in frames.into_iter().enumerate() {
            let ordinal = ordinal_start + offset as u64 + 1;
            self.event(
                b"transmit",
                &[
                    &slot.to_be_bytes(),
                    &[frame.channel],
                    &frame.generation.to_be_bytes(),
                    &frame.wire,
                ],
            );
            let loss = &self.scenario.impairments;
            let erased = hit(ordinal, loss.erase_every)
                || loss.bursts.iter().any(|range| range.contains(slot))
                || seeded_loss(loss.loss_seed, ordinal, loss.loss_per_million);
            let mut bytes = frame.wire;
            if hit(ordinal, loss.tamper_every)
                && let Some(byte) = bytes.last_mut()
            {
                *byte ^= 1;
            }
            if hit(ordinal, loss.malformed_every) {
                bytes.truncate(bytes.len() / 2);
            }
            let delay =
                if self.scenario.impairments.reorder_delay_slots > 0 && ordinal.is_multiple_of(2) {
                    self.scenario.impairments.reorder_delay_slots
                } else {
                    0
                };
            for index in 0..self.receivers.len() {
                let result = self.receivers[index].capture(
                    slot,
                    frame.channel,
                    &bytes,
                    erased,
                    &self.scenario,
                );
                self.event(
                    b"capture",
                    &[
                        &slot.to_be_bytes(),
                        &(index as u64).to_be_bytes(),
                        &ordinal.to_be_bytes(),
                        &[result],
                    ],
                );
                if result != 15 {
                    continue;
                }
                let delivery = Delivery {
                    receiver: index,
                    captured_slot: slot,
                    due: slot + delay,
                    channel: frame.channel,
                    ordinal,
                    duplicate: false,
                    wire_hex: hex::encode(&bytes),
                };
                if hit(ordinal, self.scenario.impairments.duplicate_every) {
                    self.pending.push(Delivery {
                        duplicate: true,
                        ..delivery.clone()
                    });
                }
                self.pending.push(delivery);
            }
        }
        require(
            self.pending.len() <= 4096,
            "pending captured-delivery queue budget",
        )?;
        self.pending.sort_by(|a, b| {
            let order = a.due.cmp(&b.due);
            let order = order.then_with(|| {
                if self.scenario.impairments.reorder_delay_slots > 0 {
                    b.ordinal.cmp(&a.ordinal)
                } else {
                    a.ordinal.cmp(&b.ordinal)
                }
            });
            order
                .then(a.receiver.cmp(&b.receiver))
                .then(a.duplicate.cmp(&b.duplicate))
        });
        let ready = self.pending.partition_point(|d| d.due <= slot);
        let deliveries: Vec<_> = self.pending.drain(..ready).collect();
        for delivery in deliveries {
            let bytes = hex::decode(&delivery.wire_hex).map_err(|e| e.to_string())?;
            let outcome =
                self.receivers[delivery.receiver].deliver(slot, &bytes, &self.scenario)?;
            self.event(
                b"deliver",
                &[
                    &slot.to_be_bytes(),
                    &(delivery.receiver as u64).to_be_bytes(),
                    &delivery.ordinal.to_be_bytes(),
                    &[u8::from(delivery.duplicate), outcome],
                    &bytes,
                ],
            );
        }
        for receiver in &mut self.receivers {
            receiver.update_demand(slot, &self.scenario);
        }
        self.demand.push(DemandSample {
            slot,
            produced_object_octets: self.receivers[0].report.produced_object_octets,
            receiver_unmet_object_octets: self
                .receivers
                .iter()
                .map(|r| r.report.unmet_object_octets)
                .collect(),
        });
        self.next_slot += 1;
        Ok(())
    }

    fn event(&mut self, tag: &[u8], parts: &[&[u8]]) {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.trace);
        hasher.update(&(tag.len() as u64).to_be_bytes());
        hasher.update(tag);
        for part in parts {
            hasher.update(&(part.len() as u64).to_be_bytes());
            hasher.update(part);
        }
        self.trace = *hasher.finalize().as_bytes();
    }

    pub fn report(&self) -> Result<RunReport, String> {
        let receivers: Vec<_> = self.receivers.iter().map(|r| r.report.clone()).collect();
        let semantic = serde_json::to_vec(&(
            &self.scenario.id,
            self.next_slot,
            &self.scheduler.report,
            &receivers,
            &self.demand,
            self.pending.len(),
        ))
        .map_err(|e| e.to_string())?;
        Ok(RunReport {
            schema: "transport-conformance.simulation-report/v1".into(),
            id: self.scenario.id.clone(),
            slots_executed: self.next_slot,
            namespace_blake3: hex::encode(wire::namespace(self.scenario.namespace)),
            authority_public_key_hex: hex::encode(wire::authority().to_bytes()),
            transmitter: self.scheduler.report.clone(),
            receivers,
            demand_by_slot: self.demand.clone(),
            semantic_terminal_blake3: blake3::hash(&semantic).to_hex().to_string(),
            trace_blake3: hex::encode(self.trace),
            resumed_from_slot: self.resumed_from,
            checkpoint_replay_decode_work: self.replay_work,
            pending_delivery_frames: self.pending.len(),
            physical_throughput_claim: false,
        })
    }
}

fn hit(ordinal: u64, every: u32) -> bool {
    every > 0 && ordinal.is_multiple_of(u64::from(every))
}

fn seeded_loss(seed: u64, ordinal: u64, per_million: u32) -> bool {
    if per_million == 0 {
        return false;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"retinue.simulation.erasure.v1");
    hasher.update(&seed.to_be_bytes());
    hasher.update(&ordinal.to_be_bytes());
    let bytes = hasher.finalize();
    let sample = u64::from_be_bytes(
        bytes.as_bytes()[..8]
            .try_into()
            .expect("fixed digest prefix"),
    );
    // Integer threshold without modulo bias, in authored opportunities rather than RF probability.
    u128::from(sample) * 1_000_000 < u128::from(per_million) * (u128::from(u64::MAX) + 1)
}

pub(crate) fn validate_pending(
    pending: &[Delivery],
    scenario: &Scenario,
    next_slot: u32,
) -> Result<(), String> {
    require(pending.len() <= 4096, "checkpoint pending queue budget")?;
    for delivery in pending {
        require(
            delivery.receiver < scenario.receivers.len()
                && delivery.channel < scenario.channels
                && delivery.captured_slot < next_slot
                && delivery.due >= next_slot
                && delivery.due <= delivery.captured_slot + 16
                && delivery.ordinal > 0
                && delivery.ordinal <= types::MAX_GENERATED
                && delivery.wire_hex.len() <= MAX_FRAME_BYTES * 2,
            "checkpoint pending delivery bounds",
        )?;
        hex::decode(&delivery.wire_hex).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn external_original_is_returned_byte_exact_after_decode_and_resume() {
        // Opaque binary bytes deliberately have no application serialization meaning.
        let original: Vec<u8> = (0..301).map(|i| (i * 73 % 256) as u8).collect();
        let hexadecimal = hex::encode(&original);
        let mut scenario: super::Scenario = serde_json::from_value(serde_json::json!({
            "schema": "transport-conformance.simulation/v1", "id": "external-original-smoke",
            "namespace": 900, "slots": 24, "channels": 4, "manifest_interval": 4,
            "repair_floor": 1, "generations": [{"id": 1, "publish_slot": 0,
            "expires_slot": 24, "object_octets": original.len(), "original_hex": hexadecimal}],
            "receivers": [{"id": "owner", "radios": 4}],
            "impairments": {"erase_every": 3, "duplicate_every": 5, "reorder_delay_slots": 2}
        }))
        .unwrap();
        let run = super::simulate(&scenario).unwrap();
        assert_eq!(
            run.receivers[0].generations[0]
                .recovered_object_hex
                .as_deref(),
            Some(hexadecimal.as_str())
        );
        let checkpoint = super::checkpoint_at(&scenario, 2).unwrap();
        let resumed = super::resume(&checkpoint).unwrap();
        assert_eq!(
            resumed.receivers[0].generations[0]
                .recovered_object_hex
                .as_deref(),
            Some(hexadecimal.as_str())
        );
        assert_eq!(
            run.semantic_terminal_blake3,
            resumed.semantic_terminal_blake3
        );
        assert_eq!(run.trace_blake3, resumed.trace_blake3);
        scenario.generations[0].expires_slot = 12;
        let mut expired = super::Engine::new(scenario.clone()).unwrap();
        expired.run_until(14).unwrap();
        let saved = expired.receivers[0].save();
        assert!(saved.active.is_empty());
        assert!(super::receiver::Receiver::validate_saved(saved.clone(), &scenario, 14).is_ok());
        let mut malformed = saved;
        malformed.report.generations[0].recovered_object_hex = Some("zz".repeat(original.len()));
        assert!(super::receiver::Receiver::validate_saved(malformed, &scenario, 14).is_err());
        scenario.generations[0]
            .original_hex
            .as_mut()
            .unwrap()
            .push_str("00");
        assert!(scenario.validate().is_err());
    }
}
