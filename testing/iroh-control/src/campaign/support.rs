// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Server processes and request helpers for the campaign.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, ensure};
use iroh::EndpointAddr;
use postilion::control::{
    sign_request,
    verified::{ControlExchange, Mutation},
};
use retinue::hash::AddressHash;
use seneschal::control::*;
use serde_json::{Value, json};
use tokio::{
    process::{Child, Command as Process},
    time::sleep,
};

use crate::{
    board::{NODE, owner, read_state},
    carrier::IrohCarrier,
};

pub(super) struct Server {
    pub(super) child: Child,
    pub(super) address: EndpointAddr,
    directory: PathBuf,
}

impl Server {
    pub(super) async fn start(directory: &Path) -> Result<Self> {
        let ready = directory.join("ready.json");
        if ready.exists() {
            fs::remove_file(&ready)?;
        }
        let mut child = Process::new(std::env::current_exe()?)
            .arg("serve")
            .arg(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(anyhow!("server exited before ready: {status}"));
            }
            if ready.exists() {
                let address = serde_json::from_slice(&fs::read(&ready)?)?;
                return Ok(Self {
                    child,
                    address,
                    directory: directory.to_owned(),
                });
            }
            ensure!(Instant::now() < deadline, "server readiness timeout");
            sleep(Duration::from_millis(10)).await;
        }
    }
    pub(super) async fn stop(&mut self) -> Result<()> {
        self.child.kill().await?;
        self.child.wait().await?;
        Ok(())
    }
    pub(super) fn snapshot(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&fs::read(
            self.directory.join("snapshot.json"),
        )?)?)
    }
    pub(super) fn rejection(&self) -> Result<String> {
        let value: Value =
            serde_json::from_slice(&fs::read(self.directory.join("rejection.json"))?)?;
        Ok(value["reason"].as_str().unwrap().to_owned())
    }
}

pub(super) fn request(
    operation: Operation,
    transaction: u8,
    sequence: u64,
    generation: u64,
    arguments: &[u8],
) -> Request {
    Request {
        operation,
        transaction: TransactionId([transaction; 16]),
        transaction_sequence: sequence,
        expected_generation: ConfigGeneration(generation),
        arguments: heapless::Vec::from_slice(arguments).unwrap(),
    }
}

pub(super) fn signed(request: &Request, counter: u64) -> Vec<u8> {
    sign_request(request, &owner(), AddressHash::from_bytes(NODE.0), counter).unwrap()
}

pub(super) fn counter(directory: &Path) -> Result<u64> {
    Ok(read_state(directory)?.owner_grants()[0].accepted_outer_counter())
}
pub(super) fn mutation(sequence: u64, generation: u64) -> Mutation {
    Mutation {
        sequence,
        expected_generation: ConfigGeneration(generation),
    }
}

pub(super) fn pass(cases: &mut Vec<Value>, id: &str, observation: Value) {
    cases.push(json!({ "id": id, "status": "pass", "evidence_class": "host-process-composition", "observation": observation }));
}

pub(super) async fn rejected(
    carrier: &mut IrohCarrier,
    server: &Server,
    bytes: &[u8],
    reason: &str,
    unchanged_counter: u64,
) -> Result<()> {
    ensure!(
        carrier.exchange(bytes).await.is_err(),
        "{reason}: command unexpectedly accepted"
    );
    ensure!(
        server.rejection()? == reason,
        "wrong refusal: {}",
        server.rejection()?
    );
    ensure!(
        counter(&server.directory)? == unchanged_counter,
        "refusal changed durable counter"
    );
    Ok(())
}
