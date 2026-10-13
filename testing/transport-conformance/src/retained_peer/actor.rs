// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use ed25519_dalek::SigningKey;
use retained_repair::{
    Collector, Decision, FrameToken, Lane, MAX_EPOCH_TICK, Producer, Profile, PublishOutcome,
    Settlement,
};
use serde_json::{Value, json};

use super::format;
use super::protocol::{
    Command, MAX_FRAME_BYTES, MAX_LINE_BYTES, MAX_ORIGINAL_BYTES, MAX_RECOVERY_FRAMES,
    MAX_REQUESTS, MAX_TOTAL_BYTES, Refusal, Request, SCHEMA, WireDecision, decode_hex, encode_line,
    fixed_hex,
};

pub(crate) struct Actor {
    implementation: Value,
    state: Option<State>,
    last_request_id: Option<u32>,
}

struct State {
    profile: Profile,
    producer: Producer,
    collector: Collector,
    now_slot: u32,
    command_expiry: Vec<Value>,
}

impl Actor {
    pub(crate) fn new() -> Result<Self, String> {
        Ok(Self::with_identity(super::provenance::capture()?))
    }

    pub(crate) fn with_identity(implementation: Value) -> Self {
        Self {
            implementation,
            state: None,
            last_request_id: None,
        }
    }

    pub(crate) fn hello(&self) -> Result<Vec<u8>, String> {
        encode_line(
            &json!({"schema":SCHEMA,"kind":"hello","implementation":self.implementation,
            "limits":{"encoded_line_octets":MAX_LINE_BYTES,"input_octets":MAX_TOTAL_BYTES,
                "output_octets":MAX_TOTAL_BYTES,"requests":MAX_REQUESTS,"lifetime_seconds":60,
                "idle_seconds":5,"request_seconds":10,"retained_objects":4,
                "original_octets":MAX_ORIGINAL_BYTES,"retained_original_octets":65536,
                "recovery_frames":MAX_RECOVERY_FRAMES,"epoch_slot":MAX_EPOCH_TICK},
            "scope":{"persistent_process":true,"actual_raptorq":true,"application_admission":false,
                "physical_rf":false,"fixture_authority_only":true},
            "time":"Caller-supplied monotonic logical slots; idle wall time does not advance expiry. The process has independent finite wall-clock deadlines."}),
        )
    }

    pub(crate) fn handle_line(&mut self, line: &[u8]) -> Result<Vec<u8>, String> {
        if line.len() > MAX_LINE_BYTES {
            return self.response(
                None,
                None,
                Value::Null,
                Err(Refusal::new(
                    "line_too_large",
                    "encoded line exceeds 64KiB before JSON parsing",
                )),
            );
        }
        let request: Request = match serde_json::from_slice(line) {
            Ok(request) => request,
            Err(error) => {
                return self.response(
                    None,
                    None,
                    Value::Null,
                    Err(Refusal::new("invalid_request", error.to_string())),
                );
            }
        };
        let id = request.request_id;
        let requested_now = request.now_slot;
        if request.schema != SCHEMA {
            return self.response(
                Some(id),
                Some(requested_now),
                Value::Null,
                Err(Refusal::new(
                    "unsupported_schema",
                    "unknown peer request schema",
                )),
            );
        }
        if self.last_request_id.is_some_and(|last| id <= last) {
            return self.response(
                Some(id),
                Some(requested_now),
                Value::Null,
                Err(Refusal::new(
                    "request_id_reused",
                    "request_id must strictly increase",
                )),
            );
        }
        self.last_request_id = Some(id);
        let (expiry, result) = match request.command {
            command @ Command::Init { .. } => {
                let result = self.init(requested_now, command);
                (Value::Null, result)
            }
            command => match self.state.as_mut() {
                None => (
                    Value::Null,
                    Err(Refusal::new(
                        "not_initialized",
                        "init must precede object commands",
                    )),
                ),
                Some(state) => match state.advance(requested_now) {
                    Err(refusal) => (Value::Null, Err(refusal)),
                    Ok(mut expiry) => {
                        let result = state.execute(command);
                        expiry["during_command"] = json!(std::mem::take(&mut state.command_expiry));
                        (expiry, result)
                    }
                },
            },
        };
        self.response(Some(id), Some(requested_now), expiry, result)
    }

    fn init(&mut self, now: u32, command: Command) -> Result<Value, Refusal> {
        let Command::Init {
            public_fixture,
            namespace_hex,
            authority_seed_hex,
            channels,
            manifest_interval,
            repair_floor,
        } = command
        else {
            return Err(Refusal::new("invalid_request", "expected init command"));
        };
        if self.state.is_some() {
            return Err(Refusal::new(
                "already_initialized",
                "one namespace and authority per process",
            ));
        }
        if !public_fixture {
            return Err(Refusal::new(
                "fixture_required",
                "this probe accepts explicitly public fixture authority material only",
            ));
        }
        if now > MAX_EPOCH_TICK {
            return Err(format::refusal(retained_repair::Error::EpochExhausted));
        }
        let key = SigningKey::from_bytes(&fixed_hex::<32>(&authority_seed_hex)?);
        let profile = Profile::new(
            fixed_hex::<32>(&namespace_hex)?,
            key.verifying_key(),
            channels,
            manifest_interval,
            repair_floor,
        )
        .map_err(format::refusal)?;
        let producer = Producer::new(profile.clone(), key).map_err(format::refusal)?;
        let collector = Collector::new(profile.clone()).map_err(format::refusal)?;
        let mut state = State {
            profile,
            producer,
            collector,
            now_slot: now,
            command_expiry: Vec::new(),
        };
        state.advance(now)?;
        let result = json!({"namespace_hex":hex::encode(state.profile.namespace),
            "authority_key_hex":hex::encode(state.profile.authority.to_bytes()),
            "public_fixture":true,"channels":channels,"manifest_interval":manifest_interval,
            "repair_floor":repair_floor,"oti_profile":{"symbol_octets":128,"source_blocks":1,
                "sub_blocks":1,"alignment":4},"inventory":[]});
        self.state = Some(state);
        Ok(result)
    }

    fn response(
        &self,
        request_id: Option<u32>,
        requested_now: Option<u32>,
        expiry: Value,
        result: Result<Value, Refusal>,
    ) -> Result<Vec<u8>, String> {
        let mut response = json!({"schema":SCHEMA,"kind":"response","request_id":request_id,
            "requested_now_slot":requested_now,"now_slot":self.state.as_ref().map(|s|s.now_slot),
            "expiry":expiry,"implementation":self.implementation});
        if let Some(state) = &self.state {
            response["producer_stats"] = format::producer_stats(state.producer.stats());
            response["collector_stats"] = format::collector_stats(state.collector.stats());
            response["pending_token"] = json!(state.producer.pending().map(|p| p.token.0));
        }
        match result {
            Ok(result) => {
                response["status"] = json!("ok");
                response["result"] = result;
            }
            Err(refusal) => {
                response["status"] = json!("refused");
                response["refusal"] =
                    serde_json::to_value(refusal).map_err(|error| error.to_string())?;
            }
        }
        encode_line(&response)
    }
}

impl State {
    fn advance(&mut self, now: u32) -> Result<Value, Refusal> {
        if now < self.now_slot {
            return Err(format::refusal(retained_repair::Error::ClockRegression));
        }
        if now > MAX_EPOCH_TICK {
            return Err(format::refusal(retained_repair::Error::EpochExhausted));
        }
        let producer = self.producer.advance(now).map_err(format::refusal)?;
        let collector = self.collector.advance(now).map_err(format::refusal)?;
        self.now_slot = now;
        Ok(json!({"producer":format::expiry(&producer),"collector":format::expiry(&collector)}))
    }

    fn execute(&mut self, command: Command) -> Result<Value, Refusal> {
        self.command_expiry.clear();
        match command {
            Command::Publish {
                generation,
                original_bytes_hex,
                expires_at,
            } => {
                let original = decode_hex(&original_bytes_hex, MAX_ORIGINAL_BYTES)?;
                let outcome = self
                    .producer
                    .publish(self.now_slot, generation, &original, expires_at)
                    .map_err(format::refusal)?;
                let (outcome, metadata) = match outcome {
                    PublishOutcome::Inserted(metadata) => ("inserted", metadata),
                    PublishOutcome::AlreadyRetained(metadata) => ("already_retained", metadata),
                };
                Ok(json!({"outcome":outcome,"metadata":format::metadata(&metadata)}))
            }
            Command::Snapshot => Ok(
                json!({"inventory":format::inventory(&self.producer.inventory()),
                "retained_original_octets":self.producer.retained_original_octets(),
                "collector_inventory":format::inventory(&self.collector.inventory()),
                "pending":self.producer.pending().map(format::prepared)}),
            ),
            Command::LookupMetadata { generation } => self
                .producer
                .lookup(generation)
                .map(|metadata| json!({"metadata":format::metadata(&metadata)}))
                .ok_or_else(|| format::refusal(retained_repair::Error::UnknownGeneration)),
            Command::Prepare {
                channel,
                requested_generation,
            } => {
                let prepared = self
                    .producer
                    .prepare(self.now_slot, channel, requested_generation)
                    .map_err(format::refusal)?;
                Ok(json!({"prepared":prepared.as_ref().map(format::prepared)}))
            }
            Command::Settle { token, decision } => {
                let decision = match decision {
                    WireDecision::Accept => Decision::Accept,
                    WireDecision::Defer => Decision::Defer,
                };
                let settled = self
                    .producer
                    .settle(self.now_slot, FrameToken(token), decision)
                    .map_err(format::refusal)?;
                Ok(
                    json!({"settlement":match settled { Settlement::Queued => "accepted", Settlement::Deferred => "deferred" },
                    "meaning":"Local sink custody only; neither RF delivery nor application admission."}),
                )
            }
            Command::Receive { frame_bytes_hex } => {
                let bytes = decode_hex(&frame_bytes_hex, MAX_FRAME_BYTES)?;
                let outcome = self
                    .collector
                    .ingest(self.now_slot, &bytes)
                    .map_err(format::refusal)?;
                Ok(format::received(&outcome))
            }
            Command::Recovered { generation } => self
                .collector
                .recovered(generation)
                .map(format::recovered)
                .ok_or_else(|| {
                    Refusal::new(
                        "not_recovered",
                        "collector has no complete retained decoded object for this generation",
                    )
                }),
            Command::Recover { generation } => self.recover(generation),
            Command::Init { .. } => Err(Refusal::new(
                "already_initialized",
                "init is only available once",
            )),
        }
    }

    fn recover(&mut self, generation: u64) -> Result<Value, Refusal> {
        if self.producer.pending().is_some() {
            return Err(format::refusal(retained_repair::Error::PendingFrame));
        }
        let expected = self
            .producer
            .lookup(generation)
            .ok_or_else(|| format::refusal(retained_repair::Error::UnknownGeneration))?;
        let start = self.now_slot;
        let collector_before = self.collector.inventory();
        let preexisting_selected_object = collector_before
            .iter()
            .any(|metadata| metadata.generation == generation);
        let collector_stats_before = format::collector_stats(self.collector.stats());
        let mut frames = 0usize;
        let mut dropped_fresh = 0usize;
        let mut repair_frames = 0usize;
        let mut manifest_frames = 0usize;
        loop {
            if let Some(recovered) = self.collector.recovered(generation) {
                if recovered.metadata != expected {
                    return Err(format::refusal(retained_repair::Error::ConflictingObject));
                }
                return Ok(json!({"recovered":format::recovered(recovered),
                    "recovery_kind":if frames==0 {"retained_decoder_output"} else {"repair_only_delivery_loopback"},"start_slot":start,"end_slot":self.now_slot,
                    "settled_frames":frames,"dropped_fresh_frames":dropped_fresh,
                    "delivered_repair_frames":repair_frames,"delivered_manifest_frames":manifest_frames,
                    "expiry_during_recovery":self.command_expiry,"physical_rf":false,"application_admission":false,
                    "preexisting_selected_object":preexisting_selected_object,
                    "collector_inventory_before":format::inventory(&collector_before),
                    "collector_stats_before":collector_stats_before,
                    "decoder_input_provenance":if preexisting_selected_object {"may_include_preexisting_symbols"} else {"new_object_state_from_authenticated_manifest_and_repair_frames"}}));
            }
            if self.producer.lookup(generation).is_none() {
                return Err(format::refusal(retained_repair::Error::ExpiredFrame));
            }
            for channel in 0..self.profile.channels {
                if frames == MAX_RECOVERY_FRAMES {
                    return Err(Refusal::new(
                        "recovery_budget",
                        "finite helper frame budget exhausted",
                    ));
                }
                let Some(frame) = self
                    .producer
                    .prepare(self.now_slot, channel, Some(generation))
                    .map_err(format::refusal)?
                else {
                    continue;
                };
                self.producer
                    .settle(self.now_slot, frame.token, Decision::Accept)
                    .map_err(format::refusal)?;
                frames += 1;
                match frame.lane {
                    Lane::Fresh => {
                        dropped_fresh += 1;
                    }
                    Lane::Manifest => {
                        self.collector
                            .ingest(self.now_slot, &frame.bytes)
                            .map_err(format::refusal)?;
                        manifest_frames += 1;
                    }
                    Lane::Repair => {
                        // Late collectors wait for a repeated authenticated manifest.
                        if self
                            .collector
                            .inventory()
                            .iter()
                            .any(|m| m.generation == generation)
                        {
                            self.collector
                                .ingest(self.now_slot, &frame.bytes)
                                .map_err(format::refusal)?;
                            repair_frames += 1;
                        }
                    }
                }
                if self.collector.recovered(generation).is_some() {
                    break;
                }
            }
            if self.collector.recovered(generation).is_some() {
                continue;
            }
            let next = self
                .now_slot
                .checked_add(1)
                .ok_or_else(|| format::refusal(retained_repair::Error::EpochExhausted))?;
            let expired = self.advance(next)?;
            if expired["producer"]["objects"]
                .as_array()
                .is_some_and(|objects| !objects.is_empty())
                || expired["collector"]["objects"]
                    .as_array()
                    .is_some_and(|objects| !objects.is_empty())
            {
                self.command_expiry.push(expired);
            }
        }
    }
}
