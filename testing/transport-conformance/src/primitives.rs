// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::require;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Blake3Vectors {
    key: String,
    context_string: String,
    cases: Vec<Blake3Vector>,
}

#[derive(Deserialize)]
struct Blake3Vector {
    input_len: usize,
    hash: String,
    keyed_hash: String,
    derive_key: String,
}

pub(super) fn blake3_vectors() -> Result<Value, String> {
    let vectors: Blake3Vectors =
        serde_json::from_slice(include_bytes!("../vectors/blake3-test-vectors.json"))
            .map_err(|error| error.to_string())?;
    let key: [u8; 32] = vectors
        .key
        .as_bytes()
        .try_into()
        .map_err(|_| "BLAKE3 key length")?;
    let mut comparisons = 0;
    for vector in &vectors.cases {
        require(
            vector.input_len <= 1_000_000,
            "vector input budget exceeded",
        )?;
        // The official corpus defines byte i as i modulo 251.
        let input: Vec<u8> = (0..vector.input_len).map(|i| (i % 251) as u8).collect();
        for (mode, mut hasher, expected) in [
            ("hash", blake3::Hasher::new(), &vector.hash),
            (
                "keyed_hash",
                blake3::Hasher::new_keyed(&key),
                &vector.keyed_hash,
            ),
            (
                "derive_key",
                blake3::Hasher::new_derive_key(&vectors.context_string),
                &vector.derive_key,
            ),
        ] {
            let expected = hex::decode(expected).map_err(|error| error.to_string())?;
            require(expected.len() <= 4096, "vector output budget exceeded")?;
            let mut actual = vec![0; expected.len()];
            hasher.update(&input);
            hasher.finalize_xof().fill(&mut actual);
            require(
                actual == expected,
                &format!("BLAKE3 {mode} input_len={}", vector.input_len),
            )?;
            comparisons += 1;
        }
    }
    Ok(
        json!({"official_input_lengths": vectors.cases.len(), "mode_comparisons": comparisons, "modes": ["hash", "keyed_hash", "derive_key"], "upstream_revision": "f55849f89cd85083c1c9daa2c5de20766f309c00"}),
    )
}

#[derive(Deserialize)]
struct EdVector {
    secret_seed: String,
    public_key: String,
    message: String,
    signature: String,
}

fn ed_vector() -> Result<(SigningKey, VerifyingKey, Vec<u8>, Signature), String> {
    let vector: EdVector = serde_json::from_slice(include_bytes!("../vectors/rfc8032-test1.json"))
        .map_err(|error| error.to_string())?;
    let seed: [u8; 32] = hex::decode(vector.secret_seed)
        .map_err(|error| error.to_string())?
        .try_into()
        .map_err(|_| "seed length")?;
    let public: [u8; 32] = hex::decode(vector.public_key)
        .map_err(|error| error.to_string())?
        .try_into()
        .map_err(|_| "public length")?;
    let signature =
        Signature::from_slice(&hex::decode(vector.signature).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    Ok((
        SigningKey::from_bytes(&seed),
        VerifyingKey::from_bytes(&public).map_err(|error| error.to_string())?,
        hex::decode(vector.message).map_err(|error| error.to_string())?,
        signature,
    ))
}

pub(super) fn ed25519_known_answer() -> Result<Value, String> {
    let (key, public, message, signature) = ed_vector()?;
    require(
        key.verifying_key() == public,
        "derived public key differs from TEST 1",
    )?;
    require(
        key.sign(&message) == signature,
        "signature differs from TEST 1",
    )?;
    public
        .verify_strict(&message, &signature)
        .map_err(|error| error.to_string())?;
    Ok(
        json!({"published_vectors": 1, "comparisons": ["derived public key", "deterministic signature", "strict verification"]}),
    )
}

pub(super) fn ed25519_tamper() -> Result<Value, String> {
    let (_, public, _, signature) = ed_vector()?;
    require(
        public.verify_strict(&[0], &signature).is_err(),
        "changed message accepted",
    )?;
    let mut bytes = signature.to_bytes();
    bytes[0] ^= 1;
    require(
        public
            .verify_strict(&[], &Signature::from_bytes(&bytes))
            .is_err(),
        "changed signature accepted",
    )?;
    Ok(
        json!({"authored_negative_cases": 2, "mutations": ["message changed from empty to 00", "signature bit 0 flipped"]}),
    )
}
