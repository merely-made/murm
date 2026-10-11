// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

fn main() {
    if let Err(error) = execute() {
        eprintln!("transport-conformance: {error}");
        std::process::exit(2);
    }
}

fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut output = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--interop-peer" => {
                let mut request = Vec::new();
                std::io::stdin()
                    .lock()
                    .take((transport_conformance::interop::MAX_REQUEST_BYTES + 1) as u64)
                    .read_to_end(&mut request)?;
                let response = transport_conformance::interop::run_request(&request)?;
                std::io::stdout().lock().write_all(&response)?;
                return Ok(());
            }
            "--list" => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&transport_conformance::case_catalog()?)?
                );
                return Ok(());
            }
            "--output" => output = Some(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => {
                println!(
                    "transport-conformance [--list | --output NEW_RECEIPT_PATH | --interop-peer]\nPrints a JSON receipt; exits 1 for failed cases, 2 for runner errors. Unsupported cases are explicit. Interop peer exchanges one bounded RQI1/RQO1 binary request over stdin/stdout."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    let receipt = transport_conformance::run()?;
    let json = serde_json::to_string_pretty(&receipt)? + "\n";
    if let Some(path) = output {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
    } else {
        std::io::stdout().lock().write_all(json.as_bytes())?;
    }
    if receipt.failed() {
        std::process::exit(1);
    }
    Ok(())
}
