// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SCHEMA: &str = "retinue.retained-peer/v1";
/// Includes the newline terminator. Parsing never receives a larger line.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Each direction has its own budget, including hello and line terminators.
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REQUESTS: usize = 4096;
pub(crate) const MAX_ORIGINAL_BYTES: usize = 16 * 1024;
pub(crate) const MAX_FRAME_BYTES: usize = 512;
pub(crate) const MAX_RECOVERY_FRAMES: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub schema: String,
    pub request_id: u32,
    pub now_slot: u32,
    pub command: Command,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Init {
        public_fixture: bool,
        namespace_hex: String,
        authority_seed_hex: String,
        channels: u8,
        manifest_interval: u32,
        repair_floor: u8,
    },
    Publish {
        generation: u64,
        original_bytes_hex: String,
        expires_at: u32,
    },
    Snapshot,
    LookupMetadata {
        generation: u64,
    },
    Prepare {
        channel: u8,
        #[serde(default)]
        requested_generation: Option<u64>,
    },
    Settle {
        token: u64,
        decision: WireDecision,
    },
    Receive {
        frame_bytes_hex: String,
    },
    Recovered {
        generation: u64,
    },
    Recover {
        generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireDecision {
    Accept,
    Defer,
}

#[derive(Debug, Serialize)]
pub(crate) struct Refusal {
    pub code: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_slot: Option<u32>,
}

impl Refusal {
    pub fn new(code: impl Into<String>, detail: impl AsRef<str>) -> Self {
        Self {
            code: code.into(),
            detail: detail.as_ref().chars().take(256).collect(),
            retry_at_slot: None,
        }
    }
}

pub(crate) fn decode_hex(bytes: &str, maximum: usize) -> Result<Vec<u8>, Refusal> {
    if bytes.len() > maximum * 2 || !bytes.len().is_multiple_of(2) {
        return Err(Refusal::new(
            "invalid_bytes",
            "hex length exceeds the byte limit or is odd",
        ));
    }
    hex::decode(bytes).map_err(|_| Refusal::new("invalid_bytes", "invalid hexadecimal bytes"))
}

pub(crate) fn fixed_hex<const N: usize>(bytes: &str) -> Result<[u8; N], Refusal> {
    if bytes.len() != N * 2 {
        return Err(Refusal::new(
            "invalid_bytes",
            "incorrect fixed-length hexadecimal field",
        ));
    }
    decode_hex(bytes, N)?
        .try_into()
        .map_err(|_| Refusal::new("invalid_bytes", "incorrect fixed-length hexadecimal field"))
}

pub(crate) fn encode_line(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if bytes.len() >= MAX_LINE_BYTES {
        return Err("output line budget exceeded".into());
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn terminal_line(code: &str, detail: &str) -> Result<Vec<u8>, String> {
    encode_line(&json!({
        "schema":SCHEMA,"kind":"closed","refusal":Refusal::new(code,detail)
    }))
}
