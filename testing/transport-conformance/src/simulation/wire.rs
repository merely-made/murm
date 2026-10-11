// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Authored fixed-array profile using RFC8949 core deterministic CBOR.
//! This implements only unsigned integers, definite arrays and byte strings.
//! It is neither a general CBOR decoder nor a COSE protocol.
//! Manifest body: [1,0,namespace,generation,publish,expiry,OTI,digest,channels,
//!                 manifest_interval,repair_floor].
//! Symbol body: [1,1,namespace,generation,BLAKE3(manifest_body),RaptorQ_packet].
//! Both are wrapped as [body_bytes,Ed25519_signature_of_exact_body_bytes].

use super::types::{GenerationSpec, MAX_FRAME_BYTES, Scenario};
use crate::Oti;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

#[derive(Clone, Debug)]
pub(crate) struct Manifest {
    pub generation: u64,
    pub publish: u32,
    pub expiry: u32,
    pub oti: Oti,
    pub digest: [u8; 32],
    pub body_hash: [u8; 32],
    pub channels: u8,
    pub manifest_interval: u32,
    pub repair_floor: u8,
}

#[derive(Clone, Debug)]
pub(crate) enum AuthenticatedFrame {
    Manifest(Manifest),
    Symbol {
        generation: u64,
        manifest_hash: [u8; 32],
        packet: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireError {
    Malformed,
    InvalidSignature,
    InvalidProfile,
}

// Deliberately public fixture authority: this is a simulation key, never a user key.
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42; 32])
}

pub(crate) fn authority() -> VerifyingKey {
    signing_key().verifying_key()
}

pub(crate) fn sign_checkpoint(payload: &[u8]) -> [u8; 64] {
    signing_key().sign(&checkpoint_message(payload)).to_bytes()
}

pub(crate) fn verify_checkpoint(payload: &[u8], signature: &[u8; 64]) -> Result<(), String> {
    authority()
        .verify_strict(
            &checkpoint_message(payload),
            &Signature::from_bytes(signature),
        )
        .map_err(|_| "checkpoint signature rejected".into())
}

fn checkpoint_message(payload: &[u8]) -> Vec<u8> {
    let mut message = b"retinue.retained-repair-simulation.checkpoint.v1\0".to_vec();
    message.extend_from_slice(payload);
    message
}

pub(crate) fn namespace(number: u64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"retinue.retained-repair-simulation.namespace.v1");
    hasher.update(&number.to_be_bytes());
    *hasher.finalize().as_bytes()
}

pub(crate) fn original(spec: &GenerationSpec, namespace: &[u8; 32]) -> Result<Vec<u8>, String> {
    if let Some(hexadecimal) = &spec.original_hex {
        crate::require(
            spec.object_octets <= 65_536 && hexadecimal.len() == spec.object_octets as usize * 2,
            "external original length budget",
        )?;
        return hex::decode(hexadecimal)
            .map_err(|_| "external original contains invalid hex".into());
    }
    let mut bytes = vec![0; spec.object_octets as usize];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"retinue.retained-repair-simulation.object.v1");
    hasher.update(namespace);
    hasher.update(&spec.id.to_be_bytes());
    hasher.finalize_xof().fill(&mut bytes);
    Ok(bytes)
}

fn head(out: &mut Vec<u8>, major: u8, number: u64) {
    let prefix = major << 5;
    match number {
        0..=23 => out.push(prefix | number as u8),
        24..=255 => out.extend_from_slice(&[prefix | 24, number as u8]),
        256..=65535 => {
            out.push(prefix | 25);
            out.extend_from_slice(&(number as u16).to_be_bytes());
        }
        65536..=0xffff_ffff => {
            out.push(prefix | 26);
            out.extend_from_slice(&(number as u32).to_be_bytes());
        }
        _ => {
            out.push(prefix | 27);
            out.extend_from_slice(&number.to_be_bytes());
        }
    }
}

fn uint(out: &mut Vec<u8>, number: u64) {
    head(out, 0, number);
}
fn bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    head(out, 2, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn signed(body: &[u8]) -> Vec<u8> {
    let signature = signing_key().sign(body);
    let mut out = Vec::new();
    head(&mut out, 4, 2);
    bytes(&mut out, body);
    bytes(&mut out, &signature.to_bytes());
    out
}

pub(crate) fn manifest(
    spec: &GenerationSpec,
    scenario: &Scenario,
    object: &[u8],
) -> Result<(Manifest, Vec<u8>), String> {
    let oti = scenario.oti(spec)?;
    let mut body = Vec::new();
    head(&mut body, 4, 11);
    uint(&mut body, 1);
    uint(&mut body, 0);
    bytes(&mut body, &namespace(scenario.namespace));
    uint(&mut body, spec.id);
    uint(&mut body, u64::from(spec.publish_slot));
    uint(&mut body, u64::from(spec.expires_slot));
    bytes(&mut body, &oti.to_crate()?.serialize());
    bytes(&mut body, blake3::hash(object).as_bytes());
    uint(&mut body, u64::from(scenario.channels));
    uint(&mut body, u64::from(scenario.manifest_interval));
    uint(&mut body, u64::from(scenario.repair_floor));
    let wire = signed(&body);
    let AuthenticatedFrame::Manifest(parsed) =
        verify(&wire, &namespace(scenario.namespace)).map_err(|_| "authored manifest invalid")?
    else {
        return Err("authored manifest kind".into());
    };
    Ok((parsed, wire))
}

pub(crate) fn symbol(
    generation: u64,
    namespace: &[u8; 32],
    manifest_hash: &[u8; 32],
    packet: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    head(&mut body, 4, 6);
    uint(&mut body, 1);
    uint(&mut body, 1);
    bytes(&mut body, namespace);
    uint(&mut body, generation);
    bytes(&mut body, manifest_hash);
    bytes(&mut body, packet);
    signed(&body)
}

struct Input<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Input<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], WireError> {
        let end = self.offset.checked_add(count).ok_or(WireError::Malformed)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(WireError::Malformed)?;
        self.offset = end;
        Ok(bytes)
    }
    fn head(&mut self, major: u8) -> Result<u64, WireError> {
        let first = self.take(1)?[0];
        if first >> 5 != major {
            return Err(WireError::Malformed);
        }
        let additional = first & 31;
        let (number, minimum) = match additional {
            0..=23 => (u64::from(additional), 0),
            24 => (u64::from(self.take(1)?[0]), 24),
            25 => (
                u64::from(u16::from_be_bytes(
                    self.take(2)?.try_into().map_err(|_| WireError::Malformed)?,
                )),
                256,
            ),
            26 => (
                u64::from(u32::from_be_bytes(
                    self.take(4)?.try_into().map_err(|_| WireError::Malformed)?,
                )),
                65536,
            ),
            27 => (
                u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| WireError::Malformed)?),
                0x1_0000_0000,
            ),
            _ => return Err(WireError::Malformed),
        };
        if number < minimum {
            return Err(WireError::Malformed);
        }
        Ok(number)
    }
    fn array(&mut self, count: u64) -> Result<(), WireError> {
        if self.head(4)? == count {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
    fn uint(&mut self) -> Result<u64, WireError> {
        self.head(0)
    }
    fn bytes(&mut self, limit: usize) -> Result<&'a [u8], WireError> {
        let count = usize::try_from(self.head(2)?).map_err(|_| WireError::Malformed)?;
        if count > limit {
            return Err(WireError::Malformed);
        }
        self.take(count)
    }
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.bytes(N)?.try_into().map_err(|_| WireError::Malformed)
    }
    fn finish(&self) -> Result<(), WireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
}

pub(crate) fn verify(
    wire: &[u8],
    expected_namespace: &[u8; 32],
) -> Result<AuthenticatedFrame, WireError> {
    if wire.len() > MAX_FRAME_BYTES {
        return Err(WireError::Malformed);
    }
    let mut envelope = Input {
        bytes: wire,
        offset: 0,
    };
    envelope.array(2)?;
    let body = envelope.bytes(MAX_FRAME_BYTES)?;
    let signature = Signature::from_bytes(&envelope.fixed::<64>()?);
    envelope.finish()?;
    // No untrusted manifest parameters or symbol data reach a decoder before this.
    authority()
        .verify_strict(body, &signature)
        .map_err(|_| WireError::InvalidSignature)?;
    let mut input = Input {
        bytes: body,
        offset: 0,
    };
    let count = input.head(4)?;
    if input.uint()? != 1 {
        return Err(WireError::InvalidProfile);
    }
    let kind = input.uint()?;
    if input.fixed::<32>()? != *expected_namespace {
        return Err(WireError::InvalidProfile);
    }
    let generation = input.uint()?;
    let frame = match (count, kind) {
        (11, 0) => {
            let publish = u32::try_from(input.uint()?).map_err(|_| WireError::InvalidProfile)?;
            let expiry = u32::try_from(input.uint()?).map_err(|_| WireError::InvalidProfile)?;
            let oti = Oti::parse(input.bytes(12)?)
                .and_then(Oti::bounded)
                .map_err(|_| WireError::InvalidProfile)?;
            let digest = input.fixed::<32>()?;
            let channels = u8::try_from(input.uint()?).map_err(|_| WireError::InvalidProfile)?;
            let manifest_interval =
                u32::try_from(input.uint()?).map_err(|_| WireError::InvalidProfile)?;
            let repair_floor =
                u8::try_from(input.uint()?).map_err(|_| WireError::InvalidProfile)?;
            if publish >= expiry
                || expiry > 8192
                || !(1..=8).contains(&channels)
                || manifest_interval == 0
                || manifest_interval > 4096
                || repair_floor > channels
            {
                return Err(WireError::InvalidProfile);
            }
            AuthenticatedFrame::Manifest(Manifest {
                generation,
                publish,
                expiry,
                oti,
                digest,
                body_hash: *blake3::hash(body).as_bytes(),
                channels,
                manifest_interval,
                repair_floor,
            })
        }
        (6, 1) => AuthenticatedFrame::Symbol {
            generation,
            manifest_hash: input.fixed::<32>()?,
            packet: input.bytes(260)?.to_vec(),
        },
        _ => return Err(WireError::InvalidProfile),
    };
    input.finish()?;
    Ok(frame)
}

pub(crate) fn matches_scenario(manifest: &Manifest, scenario: &Scenario) -> bool {
    scenario.generations.iter().any(|spec| {
        spec.id == manifest.generation
            && spec.publish_slot == manifest.publish
            && spec.expires_slot == manifest.expiry
            && scenario.oti(spec).ok() == Some(manifest.oti)
    }) && manifest.channels == scenario.channels
        && manifest.manifest_interval == scenario.manifest_interval
        && manifest.repair_floor == scenario.repair_floor
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferred_lengths_and_authentication_are_enforced() {
        for (value, expected) in [
            (23, vec![23]),
            (24, vec![24, 24]),
            (255, vec![24, 255]),
            (256, vec![25, 1, 0]),
            (65536, vec![26, 0, 1, 0, 0]),
        ] {
            let mut bytes = Vec::new();
            uint(&mut bytes, value);
            assert_eq!(bytes, expected);
            assert_eq!(
                Input {
                    bytes: &bytes,
                    offset: 0
                }
                .uint()
                .unwrap(),
                value
            );
        }
        assert!(
            Input {
                bytes: &[24, 1],
                offset: 0
            }
            .uint()
            .is_err()
        );
        assert!(
            Input {
                bytes: &[0x9f],
                offset: 0
            }
            .head(4)
            .is_err()
        );
        let ns = namespace(1);
        let mut wire = symbol(1, &ns, &[0; 32], &[0; 132]);
        assert!(verify(&wire, &ns).is_ok());
        *wire.last_mut().unwrap() ^= 1;
        assert_eq!(verify(&wire, &ns).unwrap_err(), WireError::InvalidSignature);
    }
}
