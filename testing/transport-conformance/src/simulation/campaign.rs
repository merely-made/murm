// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Authored design experiments. Observations describe the current model shape.

use super::types::{MAX_ACTIVE, MAX_DECODE_WORK, MAX_FRAME_BYTES, MAX_GENERATED, MAX_PACKETS};
use super::{MAX_PEER_BYTES, RunReport, Scenario, checkpoint_at, resume, simulate};
use crate::require;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const CONFIG: &str = include_str!("../../scenarios/retained-repair-v1.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Campaign {
    schema: String,
    id: String,
    assumptions: Vec<String>,
    experiments: Vec<Experiment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Experiment {
    id: String,
    question: String,
    #[serde(default)]
    compare_to: Option<String>,
    #[serde(default)]
    checkpoint_after_slot: Option<u32>,
    scenario: Scenario,
}

fn receiver<'a>(
    report: &'a RunReport,
    id: &str,
) -> Result<&'a super::types::ReceiverReport, String> {
    report
        .receivers
        .iter()
        .find(|receiver| receiver.id == id)
        .ok_or_else(|| format!("missing prototype receiver {id}"))
}

fn collection(report: &RunReport, receiver_id: &str, generation_id: u64) -> Result<Value, String> {
    let generation = receiver(report, receiver_id)?
        .generations
        .iter()
        .find(|generation| generation.id == generation_id)
        .ok_or("missing prototype generation")?;
    Ok(
        json!({"generation":generation.id,"unique_symbols":generation.unique_symbols,"required_source_symbols":generation.required_source_symbols,"completion_slot":generation.completion_slot,"reconstructed_blake3":generation.reconstructed_blake3,"incomplete_expired":generation.incomplete_expired}),
    )
}

fn observations(
    experiment: &Experiment,
    report: &RunReport,
    previous: &BTreeMap<String, RunReport>,
) -> Result<Value, String> {
    match experiment.id.as_str() {
        "finite-burst" => {
            let one = receiver(report, "one")?;
            let four = receiver(report, "four")?;
            let comparisons: Vec<Value> = one.generations.iter().map(|slow| {
                let fast = four.generations.iter().find(|generation| generation.id == slow.id);
                json!({"generation":slow.id,"one_radio_completion_slot":slow.completion_slot,"four_radio_completion_slot":fast.and_then(|generation| generation.completion_slot),"four_finished_earlier":fast.and_then(|generation| generation.completion_slot).zip(slow.completion_slot).map(|(fast,slow)|fast<slow)})
            }).collect();
            Ok(
                json!({"one_radio_unmet_object_octets":one.unmet_object_octets,"four_radio_unmet_object_octets":four.unmet_object_octets,"completion_comparison":comparisons,"interpretation":"Catch-up is conditional on this finite production tail, retention window and modeled receive opportunities. Completion uses actual RaptorQ reconstruction."}),
            )
        }
        "fresh-priority-no-floor" | "fresh-priority-repair-floor" => {
            let current = collection(report, "late", 1)?;
            let baseline = experiment
                .compare_to
                .as_ref()
                .map(|id| {
                    previous
                        .get(id)
                        .ok_or("missing earlier floor counterfactual")
                })
                .transpose()?;
            let before = baseline
                .map(|report| collection(report, "late", 1))
                .transpose()?;
            let improved = before.as_ref().map(|before| {
                current["unique_symbols"].as_u64().unwrap_or(0)
                    > before["unique_symbols"].as_u64().unwrap_or(0)
            });
            Ok(
                json!({"retained_old_generation":current,"baseline_old_generation":before,"old_collection_improved":improved,"fresh_frames":report.transmitter.fresh_frames,"repair_frames":report.transmitter.repair_frames,"interpretation":"These paired inputs change only the reserved repair floor. The late receiver misses the original burst while four large new objects keep fresh demand available. Observe both old progress and the fresh capacity consumed by repairs."}),
            )
        }
        "sustained-overload" => {
            let one = receiver(report, "one")?;
            let offered: u64 = report
                .transmitter
                .generations
                .iter()
                .map(|generation| u64::from(generation.total_source_symbols))
                .sum();
            let expired_unmet: u64 = one
                .generations
                .iter()
                .filter(|generation| generation.incomplete_expired)
                .map(|generation| u64::from(generation.object_octets))
                .sum();
            let fresh_backlog: u64 = report
                .transmitter
                .generations
                .iter()
                .map(|generation| u64::from(generation.fresh_backlog))
                .sum();
            Ok(
                json!({"offered_source_symbols":offered,"one_radio_symbol_opportunity_upper_bound":one.symbol_opportunity_upper_bound,"offered_exceeds_even_opportunity_upper_bound":offered>one.symbol_opportunity_upper_bound,"cumulative_produced_object_octets":one.produced_object_octets,"completed_object_octets":one.completed_object_octets,"cumulative_unmet_object_octets":one.unmet_object_octets,"expired_unmet_object_octets":expired_unmet,"sender_fresh_backlog_symbols":fresh_backlog,"offered_symbols_per_slot":{"numerator":offered,"denominator":experiment.scenario.slots},"interpretation":"Steady offered demand spans this finite modeled horizon. Expiring retained state does not clear cumulative undelivered work. Extending the same offered rate above receive capacity cannot provide eventual delivery of every object."}),
            )
        }
        _ => Err("unregistered prototype design question".into()),
    }
}

fn restart_observation(
    experiment: &Experiment,
    uninterrupted: &RunReport,
) -> Result<Value, String> {
    let Some(after_slot) = experiment.checkpoint_after_slot else {
        return Ok(Value::Null);
    };
    let checkpoint = checkpoint_at(&experiment.scenario, after_slot)?;
    let bytes = serde_json::to_vec(&checkpoint).map_err(|error| error.to_string())?;
    require(
        bytes.len() <= MAX_PEER_BYTES,
        "prototype checkpoint byte budget",
    )?;
    let restored = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let resumed = resume(&restored)?;
    Ok(
        json!({"kind":"in_process_saved_state_design_observation","after_slot":after_slot,"pending_frames":checkpoint.pending_frames,"snapshot_octets":bytes.len(),"snapshot_blake3":blake3::hash(&bytes).to_hex().to_string(),"terminal_hash_equal":resumed.semantic_terminal_blake3==uninterrupted.semantic_terminal_blake3,"trace_hash_equal":resumed.trace_blake3==uninterrupted.trace_blake3,"reconstruction_decode_work":resumed.checkpoint_replay_decode_work,"fresh_process":"Available through --simulation-peer; not exercised by this frontier campaign."}),
    )
}

pub fn run_campaign() -> Result<Value, String> {
    let campaign: Campaign = toml::from_str(CONFIG).map_err(|error| error.to_string())?;
    require(
        campaign.schema == "transport-conformance.simulation-experiments/v1",
        "unknown prototype experiment schema",
    )?;
    require(
        !campaign.experiments.is_empty() && campaign.experiments.len() <= 32,
        "prototype experiment count budget",
    )?;
    let mut ids = BTreeSet::new();
    let mut previous = BTreeMap::new();
    let mut results = Vec::new();
    for experiment in &campaign.experiments {
        require(
            ids.insert(&experiment.id) && experiment.id == experiment.scenario.id,
            "duplicate or mismatched experiment ID",
        )?;
        let report = simulate(&experiment.scenario)
            .map_err(|error| format!("{}: {error}", experiment.id))?;
        let observed = observations(experiment, &report, &previous)?;
        let restart = restart_observation(experiment, &report)?;
        results.push(json!({"id":experiment.id,"question":experiment.question,"compare_to":experiment.compare_to,"scenario":experiment.scenario,"observations":observed,"restart_observation":restart,"run":report}));
        previous.insert(experiment.id.clone(), report);
    }
    Ok(
        json!({"schema":"transport-conformance.simulation-exploration/v1","id":campaign.id,"kind":"prototype_design_exploration","scenario_asset_blake3":blake3::hash(CONFIG.as_bytes()).to_hex().to_string(),"assumptions":campaign.assumptions,"limits":{"slots":4096,"channels":8,"receivers":8,"generations":32,"active_generations":MAX_ACTIVE,"retained_original_octets":524288,"object_octets":65536,"symbol_octets":256,"source_symbols_per_block":256,"esi":4095,"generated_symbols":MAX_GENERATED,"receiver_packet_budget":MAX_PACKETS,"receiver_decode_work":MAX_DECODE_WORK,"frame_octets":MAX_FRAME_BYTES,"checkpoint_peer_octets":MAX_PEER_BYTES,"reorder_delay_slots":16},"scope":{"actual_raptorq_codec":true,"modeled_slots_and_bytes":true,"physical_mbps":false,"qualification_matrix":false,"standards_conformance":false,"fresh_process_resume_exercised":false},"experiments":results}),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn model_smoke_reconstructs_a_small_object() {
        let campaign: super::Campaign = toml::from_str(super::CONFIG).unwrap();
        let mut scenario = campaign.experiments[0].scenario.clone();
        scenario.slots = 32;
        scenario.source_blocks = 1;
        scenario.generations.truncate(1);
        scenario.generations[0].object_octets = 1281;
        scenario.generations[0].expires_slot = 32;
        scenario.receivers.retain(|receiver| receiver.id == "four");
        let run = super::simulate(&scenario).expect("prototype smoke run");
        let generation = &run.receivers[0].generations[0];
        assert!(generation.completion_slot.is_some());
        assert!(generation.reconstructed_blake3.is_some());
        assert!(!run.physical_throughput_claim);
    }
}
