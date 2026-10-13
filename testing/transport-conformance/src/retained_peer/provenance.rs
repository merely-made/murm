// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::io::Read;

use serde_json::{Value, json};

pub(crate) fn capture() -> Result<Value, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut file = std::fs::File::open(&executable).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 * 1024 {
        return Err("executable provenance file exceeds its 256MiB bound".into());
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut octets = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        octets += count as u64;
        if octets > 256 * 1024 * 1024 {
            return Err("executable provenance file grew beyond its bound".into());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(json!({
        "suite":crate::implementation_identity(),
        "executable":{"path":executable,"octets":octets,
            "blake3":hasher.finalize().to_hex().to_string()},
        "identity_scope":"Compiled source and lockfile fingerprint plus the executing file digest. Repository revision and dirty status are runtime observations of the compile-time source directory, not a claim that those current Git bytes produced this executable.",
        "application_operation_ids":false,
        "public_fixture_authority_only":true
    }))
}
