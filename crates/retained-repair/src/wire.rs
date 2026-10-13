// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Existing authored fixed-array v1 profile, adapted from transport-conformance
//! simulation/wire.rs at f1f3a89. Only unsigned integers, definite arrays and byte
//! strings are supported. Exact body bytes are signed; authority is injected.
//! Manifest: [1,0,namespace,generation,publish,expiry,OTI,digest,channels,cadence,floor].
//! Symbol: [1,1,namespace,generation,BLAKE3(manifest_body),RaptorQ_packet].
//! Envelope: [body_bytes,Ed25519_signature]. This is not COSE or a general CBOR codec.

use crate::{Error, MAX_EPOCH_TICK, MAX_FRAME_OCTETS, ObjectMetadata, Oti};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Manifest,
    Fresh,
    Repair,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub metadata: ObjectMetadata,
    pub oti: Oti,
    pub body_hash: [u8; 32],
    pub channels: u8,
    pub manifest_interval: u32,
    pub repair_floor: u8,
}

#[derive(Clone, Debug)]
pub enum AuthenticatedFrame {
    Manifest(Manifest),
    Symbol {
        generation: u64,
        manifest_hash: [u8; 32],
        packet: Vec<u8>,
    },
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let p = major << 5;
    match n {
        0..=23 => out.push(p | n as u8),
        24..=255 => out.extend_from_slice(&[p | 24, n as u8]),
        256..=65535 => {
            out.push(p | 25);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        65536..=0xffff_ffff => {
            out.push(p | 26);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
        _ => {
            out.push(p | 27);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}
fn uint(out: &mut Vec<u8>, n: u64) {
    head(out, 0, n);
}
fn bytes(out: &mut Vec<u8>, b: &[u8]) {
    head(out, 2, b.len() as u64);
    out.extend_from_slice(b);
}
fn signed(body: &[u8], key: &SigningKey) -> Vec<u8> {
    let mut out = Vec::new();
    head(&mut out, 4, 2);
    bytes(&mut out, body);
    bytes(&mut out, &key.sign(body).to_bytes());
    out
}

pub fn encode_manifest(
    manifest: &Manifest,
    namespace: &[u8; 32],
    key: &SigningKey,
) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    head(&mut body, 4, 11);
    uint(&mut body, 1);
    uint(&mut body, 0);
    bytes(&mut body, namespace);
    uint(&mut body, manifest.metadata.generation);
    uint(&mut body, u64::from(manifest.metadata.publish_at));
    uint(&mut body, u64::from(manifest.metadata.expires_at));
    bytes(&mut body, &manifest.oti.to_crate()?.serialize());
    bytes(&mut body, &manifest.metadata.digest);
    uint(&mut body, u64::from(manifest.channels));
    uint(&mut body, u64::from(manifest.manifest_interval));
    uint(&mut body, u64::from(manifest.repair_floor));
    Ok(signed(&body, key))
}

pub fn encode_symbol(
    generation: u64,
    namespace: &[u8; 32],
    manifest_hash: &[u8; 32],
    packet: &[u8],
    key: &SigningKey,
) -> Vec<u8> {
    let mut body = Vec::new();
    head(&mut body, 4, 6);
    uint(&mut body, 1);
    uint(&mut body, 1);
    bytes(&mut body, namespace);
    uint(&mut body, generation);
    bytes(&mut body, manifest_hash);
    bytes(&mut body, packet);
    signed(&body, key)
}

struct Input<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(n).ok_or(Error::MalformedFrame)?;
        let b = self
            .bytes
            .get(self.offset..end)
            .ok_or(Error::MalformedFrame)?;
        self.offset = end;
        Ok(b)
    }
    fn head(&mut self, major: u8) -> Result<u64, Error> {
        let first = self.take(1)?[0];
        if first >> 5 != major {
            return Err(Error::MalformedFrame);
        }
        let (n, min) = match first & 31 {
            x @ 0..=23 => (u64::from(x), 0),
            24 => (u64::from(self.take(1)?[0]), 24),
            25 => (
                u64::from(u16::from_be_bytes(
                    self.take(2)?
                        .try_into()
                        .map_err(|_| Error::MalformedFrame)?,
                )),
                256,
            ),
            26 => (
                u64::from(u32::from_be_bytes(
                    self.take(4)?
                        .try_into()
                        .map_err(|_| Error::MalformedFrame)?,
                )),
                65536,
            ),
            27 => (
                u64::from_be_bytes(
                    self.take(8)?
                        .try_into()
                        .map_err(|_| Error::MalformedFrame)?,
                ),
                0x1_0000_0000,
            ),
            _ => return Err(Error::MalformedFrame),
        };
        if n < min {
            return Err(Error::MalformedFrame);
        }
        Ok(n)
    }
    fn array(&mut self, n: u64) -> Result<(), Error> {
        if self.head(4)? != n {
            return Err(Error::MalformedFrame);
        }
        Ok(())
    }
    fn uint(&mut self) -> Result<u64, Error> {
        self.head(0)
    }
    fn bytes(&mut self, max: usize) -> Result<&'a [u8], Error> {
        let n = usize::try_from(self.head(2)?).map_err(|_| Error::MalformedFrame)?;
        if n > max {
            return Err(Error::MalformedFrame);
        }
        self.take(n)
    }
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.bytes(N)?.try_into().map_err(|_| Error::MalformedFrame)
    }
    fn finish(&self) -> Result<(), Error> {
        if self.offset != self.bytes.len() {
            return Err(Error::MalformedFrame);
        }
        Ok(())
    }
}

/// Authenticate bounded borrowed envelope bytes before parsing body fields or allocating a decoder.
pub fn verify(
    wire: &[u8],
    namespace: &[u8; 32],
    authority: &VerifyingKey,
) -> Result<AuthenticatedFrame, Error> {
    if wire.len() > MAX_FRAME_OCTETS {
        return Err(Error::MalformedFrame);
    }
    let mut envelope = Input {
        bytes: wire,
        offset: 0,
    };
    envelope.array(2)?;
    let body = envelope.bytes(MAX_FRAME_OCTETS)?;
    let signature = Signature::from_bytes(&envelope.fixed::<64>()?);
    envelope.finish()?;
    authority
        .verify_strict(body, &signature)
        .map_err(|_| Error::InvalidSignature)?;
    let mut input = Input {
        bytes: body,
        offset: 0,
    };
    let count = input.head(4)?;
    if input.uint()? != 1 {
        return Err(Error::ProfileMismatch);
    }
    let kind = input.uint()?;
    if input.fixed::<32>()? != *namespace {
        return Err(Error::ProfileMismatch);
    }
    let generation = input.uint()?;
    let frame = match (count, kind) {
        (11, 0) => {
            let publish_at = u32::try_from(input.uint()?).map_err(|_| Error::ProfileMismatch)?;
            let expires_at = u32::try_from(input.uint()?).map_err(|_| Error::ProfileMismatch)?;
            let oti = Oti::parse(input.bytes(12)?)?;
            let digest = input.fixed::<32>()?;
            let channels = u8::try_from(input.uint()?).map_err(|_| Error::ProfileMismatch)?;
            let manifest_interval =
                u32::try_from(input.uint()?).map_err(|_| Error::ProfileMismatch)?;
            let repair_floor = u8::try_from(input.uint()?).map_err(|_| Error::ProfileMismatch)?;
            if publish_at >= expires_at
                || expires_at > MAX_EPOCH_TICK
                || !(1..=8).contains(&channels)
                || !(1..=4096).contains(&manifest_interval)
                || repair_floor > channels
            {
                return Err(Error::ProfileMismatch);
            }
            AuthenticatedFrame::Manifest(Manifest {
                metadata: ObjectMetadata {
                    generation,
                    digest,
                    object_octets: oti.transfer_length as u32,
                    publish_at,
                    expires_at,
                },
                oti,
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
        _ => return Err(Error::ProfileMismatch),
    };
    input.finish()?;
    Ok(frame)
}
