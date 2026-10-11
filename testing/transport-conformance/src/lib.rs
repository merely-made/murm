// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Portable, isolated probes. A passing case establishes only its named scope.
//! RFC6330 metadata preflight does not certify the underlying RaptorQ codec.

use raptorq::{Decoder, Encoder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::process::Command;

mod independent;
pub mod interop;
mod preflight;
mod primitives;
pub use preflight::{Oti, checked_packet, checked_payload_id, parse_payload_id};
pub(crate) use preflight::{checked_packet_for_alignment, small_oti};
use preflight::{oti_cases, packet_preflight_cases, payload_id_cases};
use primitives::{blake3_vectors, ed25519_known_answer, ed25519_tamper};

pub const RECEIPT_SCHEMA: &str = "transport-conformance.receipt/v1";
/// RFC6330 section 3.3.2, as corrected by Verified Technical erratum 5548.
pub const MAX_TRANSFER_LENGTH: u64 = 942_574_504_275;
const MAX_SYMBOLS_PER_BLOCK: u64 = 56_403;
const MAX_ESI: u32 = 0xFF_FFFF;
const PROFILE_MAX_OBJECT: u64 = 65_536;
const PROFILE_MAX_SYMBOL: u16 = 256;
const PROFILE_MAX_ESI: u32 = 4_095;
const PROFILE_MAX_BLOCK_SYMBOLS: u64 = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaseMetadata {
    pub id: String,
    pub specification: String,
    pub section: String,
    pub evidence_class: String,
    pub provenance: String,
    pub implementation: String,
}

#[derive(Deserialize)]
struct Catalog {
    schema: String,
    profile: String,
    description: String,
    cases: Vec<CaseMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Unsupported,
}

#[derive(Debug, Serialize)]
pub struct CaseResult {
    #[serde(flatten)]
    pub metadata: CaseMetadata,
    pub status: Status,
    pub observations: Value,
}

#[derive(Debug, Serialize)]
pub struct Receipt {
    pub schema: &'static str,
    pub profile: String,
    pub description: String,
    pub implementation: Value,
    pub coverage: Value,
    pub summary: Value,
    pub cases: Vec<CaseResult>,
}

impl Receipt {
    pub fn failed(&self) -> bool {
        self.cases.iter().any(|case| case.status == Status::Fail)
    }
}

pub fn case_catalog() -> Result<Vec<CaseMetadata>, String> {
    Ok(catalog()?.cases)
}

fn catalog() -> Result<Catalog, String> {
    let catalog: Catalog = toml::from_str(include_str!("../metadata.toml"))
        .map_err(|error| format!("case catalog: {error}"))?;
    require(
        catalog.schema == "transport-conformance.cases/v1",
        "unknown catalog schema",
    )?;
    let mut ids = BTreeSet::new();
    for case in &catalog.cases {
        require(ids.insert(&case.id), "duplicate case ID")?;
    }
    Ok(catalog)
}

pub fn run() -> Result<Receipt, String> {
    verify_corpus()?;
    let catalog = catalog()?;
    let mut cases = Vec::new();
    for metadata in catalog.cases {
        if metadata.id == "raptorq.full-codec-conformance" {
            cases.push(CaseResult {
                metadata,
                status: Status::Unsupported,
                observations: json!({"reason": "Selected independent N=1, Al=1 vectors do not cover the complete RFC6330 parameter and reception space: multiple sub-blocks, multi-symbol packets, shortened final-source packets, all systematic-index transitions and resource/security behavior remain unqualified."}),
            });
            continue;
        }
        let outcome = std::panic::catch_unwind(|| match metadata.id.as_str() {
            "blake3.official.all-modes" => blake3_vectors(),
            "ed25519.rfc8032.test-1" => ed25519_known_answer(),
            "ed25519.rfc8032.tamper" => ed25519_tamper(),
            "raptorq.rfc6330.payload-id" => payload_id_cases(),
            "raptorq.rfc6330.oti" => oti_cases(),
            "raptorq.bounded.packet-preflight" => packet_preflight_cases(),
            "raptorq.bounded.loss-reorder-duplicate" => repair_composition(),
            "raptorq.independent-encoder-interoperability" => independent::run(),
            id => Err(format!("unregistered executable case: {id}")),
        });
        let result = outcome.unwrap_or_else(|_| Err("implementation panicked".into()));
        let (status, observations) = match result {
            Ok(observations) => (Status::Pass, observations),
            Err(error) => (Status::Fail, json!({"error": error})),
        };
        cases.push(CaseResult {
            metadata,
            status,
            observations,
        });
    }
    let count = |status| cases.iter().filter(|case| case.status == status).count();
    let summary = json!({"pass": count(Status::Pass), "fail": count(Status::Fail), "unsupported": count(Status::Unsupported)});
    let independent_passed = cases.iter().any(|case| {
        case.metadata.id == "raptorq.independent-encoder-interoperability"
            && case.status == Status::Pass
    });
    Ok(Receipt {
        schema: RECEIPT_SCHEMA,
        profile: catalog.profile,
        description: catalog.description,
        implementation: implementation_identity(),
        coverage: json!({
            "full_raptorq_codec_conformance": false,
            "independent_raptorq_encoder_interoperability": independent_passed,
            "independent_profile": "selected OpenRQ and raptorq vectors; N=1; Al=1; T=128; one full symbol per packet; retained Java output and Rust replay",
            "live_java_execution_in_this_run": false,
            "production_stack_integration": false,
            "radio_hardware_qualification": false,
            "performance_benchmark": false,
            "bounded_packet_profile": "one full symbol per packet; N=1; Al=4; F in 1..=65536; T<=256; ESI<=4095; Z<=16; source symbols per block<=256"
        }),
        summary,
        cases,
    })
}

fn implementation_identity() -> Value {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let git = |arguments: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(manifest)
            .args(arguments)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let mut hasher = blake3::Hasher::new();
    for (name, bytes) in [
        ("Cargo.toml", include_bytes!("../Cargo.toml").as_slice()),
        ("Cargo.lock", include_bytes!("../Cargo.lock").as_slice()),
        (
            "metadata.toml",
            include_bytes!("../metadata.toml").as_slice(),
        ),
        ("src/lib.rs", include_bytes!("lib.rs").as_slice()),
        ("src/main.rs", include_bytes!("main.rs").as_slice()),
        ("src/interop.rs", include_bytes!("interop.rs").as_slice()),
        (
            "src/independent.rs",
            include_bytes!("independent.rs").as_slice(),
        ),
        (
            "src/preflight.rs",
            include_bytes!("preflight.rs").as_slice(),
        ),
        (
            "src/primitives.rs",
            include_bytes!("primitives.rs").as_slice(),
        ),
        (
            "vectors/manifest.json",
            include_bytes!("../vectors/manifest.json").as_slice(),
        ),
    ] {
        hasher.update(name.as_bytes());
        hasher.update(&(bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    json!({
        "runner": "transport-conformance 0.1.0",
        "repository_revision": git(&["rev-parse", "HEAD"]),
        "repository_worktree_dirty": git(&["status", "--porcelain", "--untracked-files=all"]).map(|status| !status.is_empty()),
        "suite_source_blake3": hasher.finalize().to_hex().to_string(),
        "dependency_versions": {"blake3": "1.8.7", "ed25519-dalek": "3.0.0", "raptorq": "2.0.1"},
        "target_arch": std::env::consts::ARCH,
        "target_os": std::env::consts::OS,
        "corpus_manifest_sha256": hex::encode(Sha256::digest(include_bytes!("../vectors/manifest.json")))
    })
}

fn verify_corpus() -> Result<(), String> {
    verify_corpus_manifest(include_bytes!("../vectors/manifest.json"))
}

fn verify_corpus_manifest(bytes: &[u8]) -> Result<(), String> {
    let manifest: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    require(
        manifest["schema"].as_str() == Some("transport-conformance.corpus/v1"),
        "unknown corpus schema",
    )?;
    let artifacts = manifest["artifacts"]
        .as_array()
        .ok_or("missing corpus artifacts")?;
    require(
        artifacts.len() == 6,
        "corpus requires all six pinned artifacts",
    )?;
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        let path = artifact["path"].as_str().ok_or("missing artifact path")?;
        require(paths.insert(path), "duplicate corpus artifact")?;
        let bytes: &[u8] = match path {
            "blake3-test-vectors.json" => include_bytes!("../vectors/blake3-test-vectors.json"),
            "BLAKE3-LICENSE-APACHE-2.0.txt" => {
                include_bytes!("../vectors/BLAKE3-LICENSE-APACHE-2.0.txt")
            }
            "rfc8032-test1.json" => include_bytes!("../vectors/rfc8032-test1.json"),
            "RFC-CODE-COMPONENTS-LICENSE.txt" => {
                include_bytes!("../vectors/RFC-CODE-COMPONENTS-LICENSE.txt")
            }
            "openrq-interop.json" => include_bytes!("../vectors/openrq-interop.json"),
            "../interop/OpenRQ-LICENSE.txt" => include_bytes!("../interop/OpenRQ-LICENSE.txt"),
            _ => return Err(format!("unregistered corpus artifact: {path}")),
        };
        let digest = hex::encode(Sha256::digest(bytes));
        require(
            artifact["sha256"].as_str() == Some(digest.as_str()),
            &format!("corpus hash mismatch: {path}"),
        )?;
    }
    Ok(())
}

fn require(condition: bool, error: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(error.to_owned())
    }
}

fn repair_composition() -> Result<Value, String> {
    let mut scenarios = Vec::new();
    for length in [1024usize, 4097, 8191] {
        let data: Vec<u8> = (0..length)
            .map(|i| (i.wrapping_mul(17) % 251) as u8)
            .collect();
        let oti = Oti {
            transfer_length: length as u64,
            ..small_oti()
        }
        .bounded()?;
        let config = oti.to_crate()?;
        let encoder = Encoder::new(&data, config);
        let mut decoder = Decoder::new(config);
        let mut packets = encoder.get_encoded_packets(24);
        let offered = packets.len();
        // Drop every third offered packet, reverse arrival order, duplicate survivors.
        packets = packets
            .into_iter()
            .enumerate()
            .filter_map(|(i, packet)| (i % 3 != 0).then_some(packet))
            .collect();
        packets.reverse();
        let mut result = None;
        let mut deliveries = 0;
        for packet in &packets {
            let wire = packet.serialize();
            for _ in 0..2 {
                let checked = checked_packet(&wire, oti)?;
                if let Some(decoded) = decoder.decode(checked) {
                    result = Some(decoded);
                }
                deliveries += 1;
            }
        }
        require(
            result.as_deref() == Some(data.as_slice()),
            "repair scenario did not recover exact bytes",
        )?;
        // One symbol for only the first of two blocks cannot complete the object.
        let mut incomplete = Decoder::new(config);
        let source_packet = encoder.get_block_encoders()[0].source_packets().remove(0);
        require(
            incomplete
                .decode(checked_packet(&source_packet.serialize(), oti)?)
                .is_none(),
            "incomplete decode incorrectly completed",
        )?;
        scenarios.push(json!({"object_octets": length, "source_blocks": 2, "offered_packets": offered, "distinct_delivered_packets": packets.len(), "deliveries_including_duplicates": deliveries, "reconstructed_blake3": blake3::hash(&data).to_hex().to_string(), "incomplete_is_not_malformed": true}));
    }
    Ok(
        json!({"encoder_and_decoder": "same raptorq 2.0.1 library", "independent_interoperability": false, "scenarios": scenarios}),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_duplicate_or_corrupt_artifacts_cannot_pass_integrity() {
        let original: serde_json::Value =
            serde_json::from_slice(include_bytes!("../vectors/manifest.json")).unwrap();
        let check = |value: &serde_json::Value| {
            super::verify_corpus_manifest(&serde_json::to_vec(value).unwrap())
        };
        let mut missing = original.clone();
        missing["artifacts"].as_array_mut().unwrap().pop();
        assert!(check(&missing).is_err());
        let mut duplicate = original.clone();
        duplicate["artifacts"][1] = duplicate["artifacts"][0].clone();
        assert!(check(&duplicate).is_err());
        let mut corrupt = original;
        corrupt["artifacts"][0]["sha256"] = serde_json::json!("0".repeat(64));
        assert!(check(&corrupt).is_err());
    }

    #[test]
    fn scoped_cases_pass_with_full_codec_conformance_explicitly_unsupported() {
        let receipt = super::run().expect("suite runs");
        assert!(!receipt.failed(), "{:#?}", receipt.cases);
        assert!(receipt.cases.iter().any(|case| {
            case.metadata.id == "raptorq.independent-encoder-interoperability"
                && case.status == super::Status::Pass
        }));
        assert_eq!(
            receipt
                .cases
                .iter()
                .filter(|case| case.status == super::Status::Unsupported)
                .count(),
            1
        );
    }
}
