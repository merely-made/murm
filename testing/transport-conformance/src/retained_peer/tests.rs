// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use serde_json::{Value, json};

use super::actor::Actor;
use super::protocol::{MAX_LINE_BYTES, SCHEMA};
use super::stdio::{Input, read_input};

fn actor() -> Actor {
    Actor::with_identity(json!({"test_identity":true}))
}

fn command(actor: &mut Actor, id: u32, now: u32, command: Value) -> Value {
    let request = json!({"schema":SCHEMA,"request_id":id,"now_slot":now,"command":command});
    serde_json::from_slice(
        &actor
            .handle_line(&serde_json::to_vec(&request).unwrap())
            .unwrap(),
    )
    .unwrap()
}

fn init(actor: &mut Actor) {
    let result = command(
        actor,
        1,
        0,
        json!({"op":"init","public_fixture":true,
        "namespace_hex":hex::encode([3u8;32]),"authority_seed_hex":hex::encode([7u8;32]),
        "channels":4,"manifest_interval":5,"repair_floor":1}),
    );
    assert_eq!(result["status"], "ok");
}

#[test]
fn persistent_inventory_deferred_token_and_actual_repair_recovery() {
    let mut actor = actor();
    init(&mut actor);
    let a: Vec<_> = (0..333).map(|n| (n % 251) as u8).collect();
    let b = b"object B published after the first inventory snapshot";
    let publish = command(
        &mut actor,
        2,
        0,
        json!({"op":"publish","generation":1,
        "original_bytes_hex":hex::encode(&a),"expires_at":500}),
    );
    assert_eq!(
        publish["result"]["metadata"]["digest_hex"],
        blake3::hash(&a).to_hex().to_string()
    );
    let first = command(&mut actor, 3, 0, json!({"op":"snapshot"}));
    assert_eq!(first["result"]["inventory"].as_array().unwrap().len(), 1);
    assert_eq!(
        command(
            &mut actor,
            4,
            1,
            json!({"op":"publish","generation":2,
        "original_bytes_hex":hex::encode(b),"expires_at":500})
        )["status"],
        "ok"
    );
    let second = command(&mut actor, 5, 1, json!({"op":"snapshot"}));
    assert_eq!(second["result"]["inventory"].as_array().unwrap().len(), 2);
    let prepare = command(
        &mut actor,
        6,
        2,
        json!({"op":"prepare","channel":0,"requested_generation":1}),
    );
    let prepared = prepare["result"]["prepared"].clone();
    let token = prepared["token"].as_u64().unwrap();
    let deferred = command(
        &mut actor,
        7,
        2,
        json!({"op":"settle","token":token,"decision":"defer"}),
    );
    assert_eq!(deferred["result"]["settlement"], "deferred");
    assert_eq!(deferred["producer_stats"]["queued_frames"], 0);
    let again = command(
        &mut actor,
        8,
        2,
        json!({"op":"prepare","channel":0,"requested_generation":1}),
    );
    assert_eq!(again["result"]["prepared"], prepared);
    let refused = command(&mut actor, 9, 2, json!({"op":"recover","generation":1}));
    assert_eq!(refused["refusal"]["code"], "pending_frame");
    assert_eq!(refused["pending_token"], token);
    assert_eq!(
        command(
            &mut actor,
            10,
            2,
            json!({"op":"settle","token":token,"decision":"accept"})
        )["status"],
        "ok"
    );
    // That accepted first manifest was never delivered. The late collector
    // learns a repeated signed manifest and receives repair symbols only.
    let recovered = command(&mut actor, 11, 3, json!({"op":"recover","generation":1}));
    assert_eq!(recovered["status"], "ok");
    assert_eq!(
        recovered["result"]["recovery_kind"],
        "repair_only_delivery_loopback"
    );
    assert_eq!(recovered["result"]["preexisting_selected_object"], false);
    assert_eq!(
        recovered["result"]["recovered"]["original_bytes_hex"],
        hex::encode(&a)
    );
    assert!(
        recovered["result"]["delivered_repair_frames"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        recovered["result"]["dropped_fresh_frames"]
            .as_u64()
            .unwrap()
            > 0
    );
    let now = recovered["now_slot"].as_u64().unwrap() as u32;
    let final_inventory = command(&mut actor, 12, now, json!({"op":"snapshot"}));
    assert_eq!(
        final_inventory["result"]["inventory"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(final_inventory["producer_stats"]["encoder_allocations"], 2);
}

#[test]
fn invalid_auth_and_malformed_command_leave_prepared_frame_unsettled() {
    let mut actor = actor();
    init(&mut actor);
    command(
        &mut actor,
        2,
        0,
        json!({"op":"publish","generation":1,
        "original_bytes_hex":"010203","expires_at":100}),
    );
    let prepared = command(
        &mut actor,
        3,
        0,
        json!({"op":"prepare","channel":0,"requested_generation":1}),
    );
    let token = prepared["pending_token"].clone();
    let mut bytes = hex::decode(
        prepared["result"]["prepared"]["frame_bytes_hex"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = command(
        &mut actor,
        4,
        0,
        json!({"op":"receive","frame_bytes_hex":hex::encode(bytes)}),
    );
    assert_eq!(bad["refusal"]["code"], "invalid_auth");
    assert_eq!(bad["collector_stats"]["decoder_allocations"], 0);
    assert_eq!(bad["collector_stats"]["decode_work"], 0);
    assert_eq!(bad["producer_stats"]["queued_frames"], 0);
    assert_eq!(bad["pending_token"], token);
    let malformed: Value =
        serde_json::from_slice(&actor.handle_line(br#"{"schema":"bad"}"#).unwrap()).unwrap();
    assert_eq!(malformed["refusal"]["code"], "invalid_request");
    assert_eq!(malformed["pending_token"], token);
    assert_eq!(malformed["now_slot"], 0);
}

#[test]
fn typed_capacity_and_expiry_refusals_report_retry_and_invalidated_token() {
    let mut actor = actor();
    init(&mut actor);
    for generation in 1..=4 {
        let result = command(
            &mut actor,
            generation as u32 + 1,
            0,
            json!({"op":"publish","generation":generation,"original_bytes_hex":"0102","expires_at":2}),
        );
        assert_eq!(result["status"], "ok");
    }
    let full = command(
        &mut actor,
        6,
        0,
        json!({"op":"publish","generation":5,
        "original_bytes_hex":"0102","expires_at":3}),
    );
    assert_eq!(full["refusal"]["code"], "full");
    assert_eq!(full["refusal"]["retry_at_slot"], 2);
    assert_eq!(full["producer_stats"]["encoder_allocations"], 4);
    let prepared = command(
        &mut actor,
        7,
        1,
        json!({"op":"prepare","channel":0,"requested_generation":1}),
    );
    let token = prepared["pending_token"].as_u64().unwrap();
    let expired = command(
        &mut actor,
        8,
        2,
        json!({"op":"settle","token":token,"decision":"accept"}),
    );
    assert_eq!(expired["refusal"]["code"], "stale_token");
    assert_eq!(expired["expiry"]["producer"]["invalidated_token"], token);
    assert_eq!(
        expired["expiry"]["producer"]["objects"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(expired["producer_stats"]["queued_frames"], 0);

    let mut actor = Actor::with_identity(json!({"test_identity":true}));
    init(&mut actor);
    for generation in 1..=2 {
        command(
            &mut actor,
            generation as u32 + 1,
            0,
            json!({"op":"publish","generation":generation,
                "original_bytes_hex":hex::encode([9u8;1024]),"expires_at":1}),
        );
    }
    let during = command(&mut actor, 4, 0, json!({"op":"recover","generation":1}));
    assert_eq!(during["status"], "refused");
    assert_eq!(during["refusal"]["code"], "expired");
    assert_eq!(during["now_slot"], 1);
    assert_eq!(
        during["expiry"]["during_command"][0]["producer"]["objects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        during["expiry"]["during_command"][0]["collector"]["objects"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn input_events(bytes: Vec<u8>) -> Vec<Input> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || read_input(std::io::Cursor::new(bytes), sender));
    let events = receiver.iter().collect();
    reader.join().unwrap();
    events
}

#[test]
fn input_is_bounded_before_json_and_eof_is_only_clean_between_lines() {
    assert!(matches!(input_events(Vec::new()).as_slice(), [Input::Eof]));
    assert!(
        matches!(input_events(b"{}\n".to_vec()).as_slice(), [Input::Line(bytes),Input::Eof] if bytes==b"{}\n")
    );
    assert!(matches!(
        input_events(b"{}".to_vec()).as_slice(),
        [Input::Refused("truncated_line", _)]
    ));
    let mut oversized = vec![b'x'; MAX_LINE_BYTES];
    oversized.push(b'\n');
    assert!(matches!(
        input_events(oversized).as_slice(),
        [Input::Refused("line_too_large", _)]
    ));
}
