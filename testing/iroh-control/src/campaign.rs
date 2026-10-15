// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! A parent controller and fresh server processes, with assertions against actual journal bytes.

use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, ensure};
use iroh::{
    EndpointAddr, SecretKey,
    endpoint::{ReadError, ReadToEndError},
};
use postilion::control::{
    sign_request,
    verified::{ControlClient, ControlExchange, ProvisionalReceipt},
};
use retinue::{
    command::{Command, MAX_COMMAND_LEN, TargetClass},
    hash::AddressHash,
    identity::PrivateIdentity,
};
use seneschal::control::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::time::{sleep, timeout};

use crate::{
    board::{NODE, owner, public, read_state},
    carrier::{ALPN, IO_TIMEOUT, IrohCarrier, endpoint},
};

mod support;

use support::*;

pub async fn run() -> Result<Value> {
    let started = Instant::now();
    let fixture = tempfile::tempdir()?;
    let directory = fixture.path();
    let mut server = Server::start(directory).await?;
    let server_identity = server.address.id;
    let client_endpoint = endpoint(false).await?;
    let other_endpoint = endpoint(false).await?;
    let mut carrier = IrohCarrier {
        endpoint: client_endpoint.clone(),
        destination: server.address.clone(),
    };
    let signer = owner();
    let mut cases = Vec::new();

    let mut client = ControlClient::new(carrier.clone(), &signer, NODE);
    let status = client.status(1).await?;
    ensure!(
        status.known_good_generation == ConfigGeneration(7),
        "initial generation"
    );
    ensure!(counter(directory)? == 1, "status not durable");
    pass(
        &mut cases,
        "signed-status",
        json!({"known_good_generation": 7, "outer_counter": 1}),
    );

    let status_request = request(Operation::Status, 0x11, 0, 0, &[]);
    let wrong_target = sign_request(
        &status_request,
        &signer,
        AddressHash::from_bytes([0x99; 16]),
        2,
    )?;
    rejected(
        &mut carrier,
        &server,
        &wrong_target,
        "outer: WrongTarget",
        1,
    )
    .await?;
    pass(
        &mut cases,
        "wrong-command-target",
        json!("refused without journal change"),
    );

    let mut tampered = signed(&status_request, 2);
    *tampered.last_mut().unwrap() ^= 1;
    rejected(&mut carrier, &server, &tampered, "outer: BadSignature", 1).await?;
    pass(
        &mut cases,
        "bad-command-signature",
        json!("refused without journal change"),
    );

    let unauthorized = PrivateIdentity::from_secret_bytes(&[0x66; 64]);
    let foreign = sign_request(
        &status_request,
        &unauthorized,
        AddressHash::from_bytes(NODE.0),
        2,
    )?;
    let mut alternate = IrohCarrier {
        endpoint: other_endpoint.clone(),
        destination: server.address.clone(),
    };
    rejected(&mut alternate, &server, &foreign, "outer: UnknownKey", 1).await?;
    // An authenticated transport peer is not a grant. Conversely an authorized
    // signed command may be carried by a different authenticated transport peer.
    alternate.exchange(&signed(&status_request, 2)).await?;
    ensure!(
        counter(directory)? == 2,
        "alternate carrier did not preserve signed authority"
    );
    pass(
        &mut cases,
        "carrier-identity-versus-command-grant",
        json!({"ungranted_signer": "refused", "authorized_signer_via_other_endpoint": "accepted", "endpoint_acl": "not part of this probe"}),
    );

    rejected(
        &mut carrier,
        &server,
        &signed(&status_request, 2),
        "outer: CounterReplayed",
        2,
    )
    .await?;
    pass(
        &mut cases,
        "outer-replay",
        json!("refused against grant-restored counter"),
    );

    let connection = client_endpoint
        .connect(server.address.clone(), ALPN)
        .await?;
    let (mut send, mut receive) = connection.open_bi().await?;
    send.write_all(&vec![0; MAX_COMMAND_LEN + 1]).await?;
    send.finish()?;
    let oversized = timeout(IO_TIMEOUT, receive.read_to_end(MAX_RESPONSE_LEN)).await?;
    ensure!(
        matches!(&oversized, Err(ReadToEndError::Read(ReadError::Reset(code))) if code.into_inner() == 2),
        "expected bounded-input reset, received {oversized:?}"
    );
    connection.close(0u32.into(), b"oversize case complete");
    ensure!(counter(directory)? == 2, "oversized input reached board");
    pass(
        &mut cases,
        "bounded-stream-input",
        json!({"rejected_bytes": MAX_COMMAND_LEN + 1, "maximum_command_bytes": MAX_COMMAND_LEN}),
    );

    let malformed = Command {
        key_id: signer.hash(),
        class: TargetClass::Node,
        target: AddressHash::from_bytes(NODE.0),
        counter: 3,
        opcode: COMMAND_OPCODE,
        payload: b"malformed inner control",
    }
    .sign(&signer)
    .unwrap();
    ensure!(
        carrier.exchange(&malformed).await.is_err(),
        "malformed inner accepted"
    );
    ensure!(
        server.rejection()?.starts_with("inner: InvalidRequest("),
        "wrong malformed refusal"
    );
    ensure!(
        counter(directory)? == 3,
        "verified malformed counter not journaled"
    );
    pass(
        &mut cases,
        "verified-malformed-inner",
        json!({"outer_counter": 3, "inner": "refused after durable outer admission"}),
    );

    client.status(4).await?;
    let apply = client
        .provisional_apply(
            5,
            mutation(1, 7),
            ChangeId([1; 16]),
            public(907_875_000),
            10_000,
        )
        .await?;
    ensure!(
        apply.candidate_generation == ConfigGeneration(8),
        "apply generation"
    );
    ensure!(
        read_state(directory)?.provisional().is_some(),
        "candidate not durable"
    );
    client.commit(6, mutation(2, 7), &apply).await?;
    let state = read_state(directory)?;
    ensure!(
        state.known_good().generation == ConfigGeneration(8) && state.provisional().is_none(),
        "commit not durable"
    );
    ensure!(
        state.known_good().configuration.public == public(907_875_000),
        "committed configuration mismatch"
    );
    pass(
        &mut cases,
        "public-client-apply-commit",
        json!({"known_good_generation": 8, "outer_counter": 6}),
    );

    client
        .provisional_apply(
            7,
            mutation(3, 8),
            ChangeId([2; 16]),
            public(908_875_000),
            10_000,
        )
        .await?;
    client.revert(8, mutation(4, 8), ChangeId([2; 16])).await?;
    ensure!(
        read_state(directory)?.known_good().generation == ConfigGeneration(8)
            && read_state(directory)?.provisional().is_none(),
        "explicit revert"
    );
    pass(
        &mut cases,
        "public-client-revert",
        json!({"known_good_generation": 8, "outer_counter": 8}),
    );

    let arguments = ProvisionalApplyArguments {
        change: ChangeId([0x55; 16]),
        public: public(909_875_000),
        lifetime_ms: 10_000,
    }
    .encode();
    let retained = request(Operation::ProvisionalApply, 0x55, 5, 8, &arguments);
    fs::write(directory.join("drop-next-response"), [])?;
    let lost = carrier
        .exchange(&signed(&retained, 9))
        .await
        .err()
        .ok_or_else(|| anyhow!("response loss was not injected"))?;
    ensure!(
        matches!(lost.downcast_ref::<ReadToEndError>(), Some(ReadToEndError::Read(ReadError::Reset(code))) if code.into_inner() == 3),
        "expected deliberate lost-response reset, received {lost:?}"
    );
    let applied_count = server.snapshot()?["applies"].as_u64().unwrap();
    let state = read_state(directory)?;
    ensure!(
        state.provisional().is_some() && counter(directory)? == 9,
        "lost-response operation not durable"
    );
    let candidate_before_retry = state.clone();
    let retry = carrier.exchange(&signed(&retained, 10)).await?;
    ensure!(
        server.snapshot()?["applies"].as_u64() == Some(applied_count),
        "semantic retry applied twice"
    );
    ensure!(
        read_state(directory)?.provisional() == candidate_before_retry.provisional(),
        "retry changed candidate/token/deadline"
    );
    let ResponseBody::Provisional {
        deadline_ms,
        commit_token,
        ..
    } = retry.body
    else {
        return Err(anyhow!("retry did not return cached provisional"));
    };
    let receipt = ProvisionalReceipt {
        transaction: retained.transaction,
        counter: 10,
        change: ChangeId([0x55; 16]),
        candidate_generation: retry.effective_generation.unwrap(),
        deadline_ms,
        commit_token,
    };
    let mut conflicting = retained.clone();
    conflicting.arguments = heapless::Vec::from_slice(
        &ProvisionalApplyArguments {
            change: ChangeId([0x55; 16]),
            public: public(910_875_000),
            lifetime_ms: 10_000,
        }
        .encode(),
    )
    .unwrap();
    ensure!(
        carrier.exchange(&signed(&conflicting, 11)).await.is_err(),
        "conflicting semantic reuse accepted"
    );
    ensure!(
        server.rejection()? == "serve: RuntimeError::Refused(TransactionConflict)",
        "wrong semantic conflict refusal: {}",
        server.rejection()?
    );
    ensure!(
        counter(directory)? == 11
            && read_state(directory)?.provisional() == candidate_before_retry.provisional(),
        "conflict admission invariant"
    );
    pass(
        &mut cases,
        "semantic-conflict",
        json!({"outer_counter": 11, "mutation": "refused", "candidate": "unchanged"}),
    );

    client.commit(12, mutation(6, 8), &receipt).await?;
    ensure!(
        read_state(directory)?.known_good().generation == ConfigGeneration(10),
        "retry commit generation"
    );
    pass(
        &mut cases,
        "lost-response-semantic-retry",
        json!({"lost_response_counter": 9, "retry_counter": 10, "candidate_generation": 10, "additional_applies_on_retry": 0, "commit_counter": 12}),
    );
    ensure!(
        carrier.exchange(&signed(&retained, 13)).await.is_err(),
        "completed old mutation re-executed"
    );
    ensure!(
        server.rejection()? == "serve: RuntimeError::Refused(TransactionExpired)",
        "old mutation not expired"
    );
    ensure!(
        counter(directory)? == 13 && read_state(directory)?.provisional().is_none(),
        "completed mutation replay invariant"
    );
    pass(
        &mut cases,
        "old-mutation-after-cache-replacement",
        json!({"outer_counter": 13, "reason": "TransactionExpired", "additional_candidate": false}),
    );

    let before_pid = server.child.id().unwrap();
    server.stop().await?;
    server = Server::start(directory).await?;
    ensure!(
        server.child.id().unwrap() != before_pid && server.address.id == server_identity,
        "fresh process/stable transport identity"
    );
    carrier.destination = server.address.clone();
    client = ControlClient::new(carrier.clone(), &signer, NODE);
    ensure!(
        counter(directory)? == 13
            && read_state(directory)?.known_good().generation == ConfigGeneration(10),
        "committed restart lost state"
    );
    rejected(
        &mut carrier,
        &server,
        &signed(&retained, 10),
        "outer: CounterReplayed",
        13,
    )
    .await?;
    pass(
        &mut cases,
        "fresh-process-committed-reopen",
        json!({"fresh_process": true, "stable_endpoint_key": true, "known_good_generation": 10, "restored_outer_counter": 13, "old_wire": "refused"}),
    );

    client
        .provisional_apply(
            14,
            mutation(7, 10),
            ChangeId([0x77; 16]),
            public(910_875_000),
            1_000,
        )
        .await?;
    let stalled = client_endpoint
        .connect(server.address.clone(), ALPN)
        .await?;
    let (mut partial, _unread) = stalled.open_bi().await?;
    partial.write_all(&[0]).await?; // No FIN: network read stays incomplete for three seconds.
    let expiry_started = Instant::now();
    while read_state(directory)?.provisional().is_some() {
        ensure!(
            expiry_started.elapsed() < Duration::from_secs(2),
            "unfinished stream delayed rollback"
        );
        sleep(Duration::from_millis(20)).await;
    }
    ensure!(
        read_state(directory)?.known_good().generation == ConfigGeneration(10),
        "expiry changed known-good"
    );
    stalled.close(0u32.into(), b"unfinished stream case complete");
    pass(
        &mut cases,
        "autonomous-expiry-with-unfinished-stream",
        json!({"provisional_lifetime_ms": 1000, "observed_wait_ms": expiry_started.elapsed().as_millis(), "stream_timeout_ms": IO_TIMEOUT.as_millis(), "known_good_generation": 10}),
    );

    client
        .provisional_apply(
            15,
            mutation(8, 10),
            ChangeId([0x88; 16]),
            public(911_875_000),
            10_000,
        )
        .await?;
    server.stop().await?;
    server = Server::start(directory).await?;
    carrier.destination = server.address.clone();
    client = ControlClient::new(carrier.clone(), &signer, NODE);
    let recovered = read_state(directory)?;
    ensure!(
        recovered.provisional().is_none()
            && recovered.known_good().generation == ConfigGeneration(10),
        "reboot did not restore known-good"
    );
    ensure!(
        server.snapshot()?["recovered_rollback"] == true && counter(directory)? == 15,
        "reboot rollback evidence/counter"
    );
    client.status(16).await?;
    pass(
        &mut cases,
        "fresh-process-provisional-rollback",
        json!({"recovered_rollback": true, "restored_outer_counter": 15, "next_status_counter": 16, "known_good_generation": 10}),
    );

    let wrong_id = SecretKey::from_bytes(&[0x99; 32]).public();
    let wrong_address = EndpointAddr::new(wrong_id).with_addrs(server.address.addrs.clone());
    let mut impostor_target = IrohCarrier {
        endpoint: client_endpoint.clone(),
        destination: wrong_address,
    };
    ensure!(
        impostor_target
            .exchange(&signed(&status_request, 17))
            .await
            .is_err(),
        "wrong endpoint identity authenticated"
    );
    ensure!(
        counter(directory)? == 16,
        "wrong endpoint reached control authority"
    );
    pass(
        &mut cases,
        "wrong-iroh-endpoint-key",
        json!("transport failed before command admission"),
    );

    server.stop().await?;
    client_endpoint.close().await;
    other_endpoint.close().await;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_files = [
        "Cargo.toml",
        "Cargo.lock",
        "src/main.rs",
        "src/board.rs",
        "src/carrier.rs",
        "src/campaign.rs",
    ];
    let fingerprints: serde_json::Map<String, Value> = source_files
        .into_iter()
        .map(|name| {
            let bytes = fs::read(root.join(name)).unwrap();
            let hash = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (name.to_owned(), json!(hash))
        })
        .collect();
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    Ok(json!({
        "schema": "retinue-iroh-control-probe/v1", "date": "2026-10-11",
        "summary": {"passed": cases.len(), "failed": 0}, "cases": cases,
        "implementation": {"base_revision": String::from_utf8(head.stdout)?.trim(), "sha256": fingerprints,
            "iroh": "1.3.0", "platform": std::env::consts::OS, "architecture": std::env::consts::ARCH},
        "profile": {"alpn": String::from_utf8_lossy(ALPN), "route": "IPv4 loopback; direct addresses; relay and lookup disabled",
            "maximum_command_bytes": MAX_COMMAND_LEN, "maximum_response_bytes": MAX_RESPONSE_LEN,
            "maximum_network_sessions": 4, "board_queue_capacity": 4, "io_timeout_ms": IO_TIMEOUT.as_millis(),
            "journal": "two 4096-byte fsynced files; existing A/B record and durable codecs", "server_endpoint_id": server_identity.to_string()},
        "limitations": ["Host composition only; no MCU build, hardware reset, RF, power or unattended qualification",
            "Quiet owner and configuration applier are simulations; no physical radio is operated",
            "Abrupt process restart is not a mid-write power-cut campaign",
            "Probe ALPN and stream/reset mapping are local experiments, not a published protocol",
            "Mutation helper does not expose retained-request retry; probe uses sign_request with a retained semantic request",
            "Known fixture keys are test data; commissioning/key custody is not implemented",
            "No relay, address discovery, NAT traversal, bare-noq or independent endpoint interop qualification"],
        "elapsed_ms": started.elapsed().as_millis()
    }))
}
