// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Cases,
    List,
    InteropPeer,
    Simulation,
    SimulationPeer,
    RetainedPeer,
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("transport-conformance: {error}");
        std::process::exit(2);
    }
}

fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut output = None;
    let mut mode = Mode::Cases;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--interop-peer" | "--list" | "--simulate" | "--simulation-peer"
            | "--retained-peer" => {
                if mode != Mode::Cases {
                    return Err("select only one runner mode".into());
                }
                mode = match argument.as_str() {
                    "--interop-peer" => Mode::InteropPeer,
                    "--list" => Mode::List,
                    "--simulate" => Mode::Simulation,
                    "--retained-peer" => Mode::RetainedPeer,
                    _ => Mode::SimulationPeer,
                };
            }
            "--output" => output = Some(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => {
                println!(
                    "transport-conformance [--list | --interop-peer | --simulate | --simulation-peer | --retained-peer] [--output NEW_RECEIPT_PATH]\nPrints a JSON receipt; exits 1 for failed cases, 2 for runner errors. Unsupported cases are explicit. The interop peer exchanges bounded RQI1/RQO1 bytes; the simulation peer resumes bounded JSON checkpoints; the retained peer runs a finite persistent NDJSON producer/collector session."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    if mode == Mode::RetainedPeer {
        if output.is_some() {
            return Err("process peers write only to stdout; --output is unavailable".into());
        }
        transport_conformance::retained_peer::run_stdio()?;
        return Ok(());
    }
    if mode == Mode::InteropPeer || mode == Mode::SimulationPeer {
        if output.is_some() {
            return Err("process peers write only to stdout; --output is unavailable".into());
        }
        let maximum = if mode == Mode::InteropPeer {
            transport_conformance::interop::MAX_REQUEST_BYTES
        } else {
            transport_conformance::simulation::MAX_PEER_BYTES
        };
        let mut request = Vec::new();
        std::io::stdin()
            .lock()
            .take((maximum + 1) as u64)
            .read_to_end(&mut request)?;
        if request.len() > maximum {
            return Err(if mode == Mode::InteropPeer {
                "interop request budget exceeded"
            } else {
                "simulation request budget exceeded"
            }
            .into());
        }
        let response = if mode == Mode::InteropPeer {
            transport_conformance::interop::run_request(&request)?
        } else {
            transport_conformance::simulation::run_peer(&request)?
        };
        std::io::stdout().lock().write_all(&response)?;
        return Ok(());
    }
    let receipt = match mode {
        Mode::Cases => serde_json::to_value(transport_conformance::run()?)?,
        Mode::List => serde_json::to_value(transport_conformance::case_catalog()?)?,
        Mode::Simulation => transport_conformance::run_simulation()?,
        _ => unreachable!(),
    };
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
    if receipt["summary"]["fail"].as_u64().unwrap_or(0) != 0 {
        std::process::exit(1);
    }
    Ok(())
}
