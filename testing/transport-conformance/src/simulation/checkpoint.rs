// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Bounded signed snapshot of scheduler, captured delivery queue and receivers.
//! Decoder internals are reconstructed from authenticated collected packets;
//! resume then starts at next_slot, without rerunning earlier transmission slots.

use super::receiver::{Receiver, ReceiverSave};
use super::scheduler::{Scheduler, SchedulerSave};
use super::types::{DemandSample, MAX_PEER_BYTES, RunReport, Scenario};
use super::{Delivery, Engine, validate_pending, wire};
use crate::require;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const CHECKPOINT_SCHEMA: &str = "transport-conformance.simulation-checkpoint/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema: String,
    pub next_slot: u32,
    pub pending_frames: usize,
    pub state_hex: String,
    pub signature_hex: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    scenario: Scenario,
    next_slot: u32,
    scheduler: SchedulerSave,
    receivers: Vec<ReceiverSave>,
    pending: Vec<Delivery>,
    trace_hex: String,
    demand_by_slot: Vec<DemandSample>,
}

pub fn checkpoint_at(scenario: &Scenario, after_slot: u32) -> Result<Checkpoint, String> {
    require(
        after_slot < scenario.slots,
        "checkpoint slot outside scenario",
    )?;
    let mut engine = Engine::new(scenario.clone())?;
    engine.run_until(after_slot + 1)?;
    let snapshot = Snapshot {
        scenario: engine.scenario,
        next_slot: engine.next_slot,
        scheduler: engine.scheduler.save(),
        receivers: engine.receivers.iter().map(Receiver::save).collect(),
        pending: engine.pending,
        trace_hex: hex::encode(engine.trace),
        demand_by_slot: engine.demand,
    };
    let state = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    require(
        state.len() <= (MAX_PEER_BYTES - 2048) / 2,
        "checkpoint state exceeds bounded JSON peer profile",
    )?;
    Ok(Checkpoint {
        schema: CHECKPOINT_SCHEMA.into(),
        next_slot: snapshot.next_slot,
        pending_frames: snapshot.pending.len(),
        signature_hex: hex::encode(wire::sign_checkpoint(&state)),
        state_hex: hex::encode(state),
    })
}

pub fn resume(checkpoint: &Checkpoint) -> Result<RunReport, String> {
    require(
        checkpoint.schema == CHECKPOINT_SCHEMA
            && checkpoint.state_hex.len() <= MAX_PEER_BYTES - 2048
            && checkpoint.signature_hex.len() == 128,
        "checkpoint envelope schema/length budget",
    )?;
    let state = hex::decode(&checkpoint.state_hex).map_err(|e| e.to_string())?;
    let signature: [u8; 64] = hex::decode(&checkpoint.signature_hex)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "checkpoint signature length")?;
    // Authenticate the entire snapshot before parsing its source/receiver state.
    wire::verify_checkpoint(&state, &signature)?;
    let snapshot: Snapshot = serde_json::from_slice(&state).map_err(|e| e.to_string())?;
    snapshot.scenario.validate()?;
    require(
        snapshot.next_slot > 0
            && snapshot.next_slot <= snapshot.scenario.slots
            && snapshot.next_slot == checkpoint.next_slot
            && snapshot.pending.len() == checkpoint.pending_frames,
        "checkpoint cursor/envelope mismatch",
    )?;
    require(
        snapshot.receivers.len() == snapshot.scenario.receivers.len()
            && snapshot.demand_by_slot.len() == snapshot.next_slot as usize,
        "checkpoint receiver/demand history length",
    )?;
    validate_pending(&snapshot.pending, &snapshot.scenario, snapshot.next_slot)?;
    let trace: [u8; 32] = hex::decode(&snapshot.trace_hex)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "checkpoint trace digest length")?;
    let mut ids = BTreeSet::new();
    let mut validated = Vec::new();
    for (saved, spec) in snapshot
        .receivers
        .into_iter()
        .zip(&snapshot.scenario.receivers)
    {
        require(
            saved.id == spec.id && ids.insert(saved.id.clone()),
            "checkpoint receiver order/uniqueness",
        )?;
        validated.push(Receiver::validate_saved(
            saved,
            &snapshot.scenario,
            snapshot.next_slot,
        )?);
    }
    for (slot, sample) in snapshot.demand_by_slot.iter().enumerate() {
        require(
            sample.slot as usize == slot
                && sample.receiver_unmet_object_octets.len() == snapshot.scenario.receivers.len(),
            "checkpoint demand sample bounds/order",
        )?;
    }
    // Every receiver manifest and collected packet was validated before any decoder.
    let scheduler = Scheduler::restore(snapshot.scheduler, &snapshot.scenario, snapshot.next_slot)?;
    let mut receivers = Vec::new();
    let mut replay_work = 0;
    for saved in validated {
        let (receiver, work) = Receiver::restore(saved, &snapshot.scenario)?;
        receivers.push(receiver);
        replay_work += work;
    }
    let mut engine = Engine {
        scenario: snapshot.scenario,
        next_slot: snapshot.next_slot,
        scheduler,
        receivers,
        pending: snapshot.pending,
        trace,
        demand: snapshot.demand_by_slot,
        resumed_from: Some(snapshot.next_slot),
        replay_work,
    };
    engine.run_until(engine.scenario.slots)?;
    engine.report()
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum PeerRequest {
    Run { scenario: Scenario },
    Checkpoint { scenario: Scenario, after_slot: u32 },
    Resume { checkpoint: Checkpoint },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PeerResponse {
    Report { report: Box<RunReport> },
    Checkpoint { checkpoint: Checkpoint },
}

pub fn run_peer(request: &[u8]) -> Result<Vec<u8>, String> {
    require(
        request.len() <= MAX_PEER_BYTES,
        "simulation peer request budget",
    )?;
    let request: PeerRequest = serde_json::from_slice(request).map_err(|e| e.to_string())?;
    let response = match request {
        PeerRequest::Run { scenario } => PeerResponse::Report {
            report: Box::new(super::simulate(&scenario)?),
        },
        PeerRequest::Checkpoint {
            scenario,
            after_slot,
        } => PeerResponse::Checkpoint {
            checkpoint: checkpoint_at(&scenario, after_slot)?,
        },
        PeerRequest::Resume { checkpoint } => PeerResponse::Report {
            report: Box::new(resume(&checkpoint)?),
        },
    };
    let bytes = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
    require(
        bytes.len() <= MAX_PEER_BYTES,
        "simulation peer response budget",
    )?;
    Ok(bytes)
}
