// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Exact independent OpenRQ symbols and retained bidirectional peer observations.

use super::{Oti, checked_packet_for_alignment, checked_payload_id, interop, require};
use raptorq::{Decoder, Encoder, EncodingPacket};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Corpus {
    schema: String,
    profile: Value,
    generator: Value,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    object_hex: String,
    oti_hex: String,
    blocks: Vec<Block>,
    packets: Vec<Packet>,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize, Serialize)]
struct Block {
    sbn: u8,
    k: u64,
    k_prime: u64,
}

#[derive(Deserialize)]
struct Packet {
    sbn: u8,
    esi: u32,
    symbol_hex: String,
}

#[derive(Deserialize)]
struct Scenario {
    id: String,
    delivery: Vec<PacketId>,
    expected: String,
    java_decoded_object_hex: String,
    rust_packets_sha256: String,
}

#[derive(Clone, Copy, Deserialize)]
struct PacketId {
    sbn: u8,
    esi: u32,
}

fn decode_hex(hexadecimal: &str, maximum: usize) -> Result<Vec<u8>, String> {
    require(
        hexadecimal.len() <= maximum * 2,
        "independent artifact byte budget exceeded",
    )?;
    hex::decode(hexadecimal).map_err(|error| error.to_string())
}

pub(super) fn run() -> Result<Value, String> {
    let corpus: Corpus = serde_json::from_slice(include_bytes!("../vectors/openrq-interop.json"))
        .map_err(|error| error.to_string())?;
    require(
        corpus.schema == "transport-conformance.raptorq-interop/v1",
        "unknown independent corpus schema",
    )?;
    require(
        !corpus.cases.is_empty() && corpus.cases.len() <= 64,
        "independent case count budget",
    )?;
    let mut observations = Vec::new();
    let mut exact_symbols = 0usize;
    let mut rust_decoder_runs = 0usize;
    let mut retained_java_decoder_runs = 0usize;
    let mut case_ids = BTreeSet::new();
    for case in &corpus.cases {
        require(case_ids.insert(&case.id), "duplicate independent case ID")?;
        let object = decode_hex(&case.object_hex, 65_536)?;
        let oti_bytes = decode_hex(&case.oti_hex, 12)?;
        let oti = Oti::parse(&oti_bytes)?.bounded_for_alignment(1)?;
        require(
            oti.transfer_length == object.len() as u64,
            "independent object length differs from OTI",
        )?;
        require(
            oti.to_crate()?.serialize().as_slice() == oti_bytes,
            "independent OTI exact octets differ",
        )?;
        require(
            !case.packets.is_empty() && case.packets.len() <= 1024,
            "independent packet count budget",
        )?;
        require(
            !case.scenarios.is_empty() && case.scenarios.len() <= 32,
            "independent scenario count budget",
        )?;
        let mut expected = BTreeMap::<(u8, u32), EncodingPacket>::new();
        let mut ids = Vec::new();
        for packet in &case.packets {
            let id = checked_payload_id(packet.sbn, packet.esi)?;
            let mut wire = id.serialize().to_vec();
            wire.extend_from_slice(&decode_hex(
                &packet.symbol_hex,
                usize::from(oti.symbol_size),
            )?);
            let checked = checked_packet_for_alignment(&wire, oti, 1)?;
            require(
                expected.insert((packet.sbn, packet.esi), checked).is_none(),
                "duplicate independent packet ID",
            )?;
            ids.push(id);
        }
        // Only fully preflighted object/metadata/IDs reach assertion-based encoding APIs.
        let encoder = Encoder::new(&object, oti.to_crate()?);
        let rust_packets = interop::encode_ids(&encoder, &ids)?;
        let mut generated = BTreeMap::new();
        for packet in rust_packets {
            let id = packet.payload_id();
            let key = (id.source_block_number(), id.encoding_symbol_id());
            require(
                expected.get(&key).map(EncodingPacket::serialize) == Some(packet.serialize()),
                &format!(
                    "independent symbol differs: {} SBN={} ESI={}",
                    case.id, key.0, key.1
                ),
            )?;
            generated.insert(key, packet);
            exact_symbols += 1;
        }
        let total_source_symbols = oti.transfer_length.div_ceil(u64::from(oti.symbol_size));
        let small = total_source_symbols / u64::from(oti.source_blocks);
        let large_blocks = total_source_symbols % u64::from(oti.source_blocks);
        let block_symbols: Vec<u64> = (0..u64::from(oti.source_blocks))
            .map(|sbn| small + u64::from(sbn < large_blocks))
            .collect();
        require(
            case.blocks.len() == block_symbols.len(),
            "independent block annotation count",
        )?;
        for (index, block) in case.blocks.iter().enumerate() {
            require(
                usize::from(block.sbn) == index && block.k == block_symbols[index],
                "independent source block annotation differs from partition",
            )?;
            require(
                block.k_prime >= block.k,
                "independent K-prime annotation smaller than K",
            )?;
        }
        let mut scenario_observations = Vec::new();
        let mut scenario_ids = BTreeSet::new();
        for scenario in &case.scenarios {
            require(
                scenario_ids.insert(&scenario.id),
                "duplicate independent scenario ID",
            )?;
            require(
                scenario.expected == "complete",
                "unknown independent scenario expectation",
            )?;
            require(
                !scenario.delivery.is_empty() && scenario.delivery.len() <= 1024,
                "independent delivery count budget",
            )?;
            let mut received = Vec::with_capacity(scenario.delivery.len());
            let mut rust_hash = Sha256::new();
            for id in &scenario.delivery {
                let key = (id.sbn, id.esi);
                received.push(
                    expected
                        .get(&key)
                        .ok_or("scenario references missing Java packet")?
                        .clone(),
                );
                rust_hash.update(
                    generated
                        .get(&key)
                        .ok_or("scenario references missing Rust packet")?
                        .serialize(),
                );
            }
            let mut decoder = Decoder::new(oti.to_crate()?);
            let mut result = None;
            for packet in received {
                if let Some(decoded) = decoder.decode(packet) {
                    result = Some(decoded);
                }
            }
            require(
                result.as_deref() == Some(object.as_slice()),
                &format!("Rust failed Java decode: {} / {}", case.id, scenario.id),
            )?;
            rust_decoder_runs += 1;
            require(
                hex::encode(rust_hash.finalize()) == scenario.rust_packets_sha256,
                "retained Java observation not bound to current Rust packet bytes",
            )?;
            let java_object = decode_hex(&scenario.java_decoded_object_hex, 65_536)?;
            require(
                java_object == object,
                "retained Java decode differs from object",
            )?;
            retained_java_decoder_runs += 1;
            scenario_observations.push(json!({"id": scenario.id, "deliveries": scenario.delivery.len(), "rust_decoded_java": true, "java_decoded_rust": "retained independent peer output; external campaign reruns Java", "packet_sha256": scenario.rust_packets_sha256}));
        }
        observations.push(json!({"id": case.id, "object_octets": object.len(), "oti_hex": case.oti_hex, "source_symbols_per_block": block_symbols, "upstream_systematic_index_annotations": case.blocks, "partial_final_source_symbol": !object.len().is_multiple_of(usize::from(oti.symbol_size)), "exact_symbols": case.packets.len(), "scenarios": scenario_observations}));
    }
    Ok(json!({
        "shared_profile": corpus.profile,
        "generator": corpus.generator,
        "independent_encoder": "OpenRQ Java",
        "rust_encoder_decoder": "raptorq 2.0.1",
        "exact_encoded_symbols": exact_symbols,
        "rust_decode_java_scenarios_this_run": rust_decoder_runs,
        "retained_java_decode_rust_scenarios_verified": retained_java_decoder_runs,
        "complete_rfc6330_codec_conformance": false,
        "cases": observations
    }))
}
