// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use ed25519_dalek::VerifyingKey;
use raptorq::{EncodingPacket, ObjectTransmissionInformation, PayloadId};

use crate::{Error, MAX_ESI, MAX_OBJECT_OCTETS};

/// Caller-selected namespace and signing authority. No public fixture key is built in.
#[derive(Clone, Debug)]
pub struct Profile {
    pub namespace: [u8; 32],
    pub authority: VerifyingKey,
    pub channels: u8,
    pub manifest_interval: u32,
    pub repair_floor: u8,
}

impl Profile {
    pub fn new(
        namespace: [u8; 32],
        authority: VerifyingKey,
        channels: u8,
        manifest_interval: u32,
        repair_floor: u8,
    ) -> Result<Self, Error> {
        let result = Self {
            namespace,
            authority,
            channels,
            manifest_interval,
            repair_floor,
        };
        result.validate()?;
        Ok(result)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !(1..=4).contains(&self.channels)
            || !(1..=4096).contains(&self.manifest_interval)
            || self.repair_floor > self.channels
        {
            return Err(Error::InvalidProfile("channels/cadence/repair floor"));
        }
        Ok(())
    }
    pub(crate) fn oti(&self, octets: usize) -> Result<Oti, Error> {
        if !(1..=MAX_OBJECT_OCTETS).contains(&octets) {
            return Err(Error::ObjectSize);
        }
        Ok(Oti {
            transfer_length: octets as u64,
            symbol_size: 128,
            source_blocks: 1,
            sub_blocks: 1,
            alignment: 4,
        })
    }
    pub(crate) fn matches(&self, manifest: &crate::wire::Manifest) -> bool {
        manifest.channels == self.channels
            && manifest.manifest_interval == self.manifest_interval
            && manifest.repair_floor == self.repair_floor
            && manifest.oti.symbol_size == 128
            && manifest.oti.source_blocks == 1
            && manifest.oti.sub_blocks == 1
            && manifest.oti.alignment == 4
            && manifest.oti.transfer_length <= MAX_OBJECT_OCTETS as u64
    }
}

/// The existing v1 wire profile's bounded OTI, shared with byte-level consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Oti {
    pub transfer_length: u64,
    pub symbol_size: u16,
    pub source_blocks: u8,
    pub sub_blocks: u16,
    pub alignment: u8,
}

impl Oti {
    pub fn bounded(self) -> Result<Self, Error> {
        if !(1..=65_536).contains(&self.transfer_length)
            || self.symbol_size == 0
            || self.symbol_size > 256
            || !self.symbol_size.is_multiple_of(4)
            || !(1..=16).contains(&self.source_blocks)
            || self.sub_blocks != 1
            || self.alignment != 4
        {
            return Err(Error::ProfileMismatch);
        }
        let symbols = self.transfer_length.div_ceil(u64::from(self.symbol_size));
        if u64::from(self.source_blocks) > symbols
            || symbols.div_ceil(u64::from(self.source_blocks)) > 256
        {
            return Err(Error::ProfileMismatch);
        }
        Ok(self)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let b: &[u8; 12] = bytes.try_into().map_err(|_| Error::MalformedFrame)?;
        Self {
            transfer_length: u64::from_be_bytes([0, 0, 0, b[0], b[1], b[2], b[3], b[4]]),
            symbol_size: u16::from_be_bytes([b[6], b[7]]),
            source_blocks: b[8],
            sub_blocks: u16::from_be_bytes([b[9], b[10]]),
            alignment: b[11],
        }
        .bounded()
    }
    pub fn to_crate(self) -> Result<ObjectTransmissionInformation, Error> {
        self.bounded()?;
        Ok(ObjectTransmissionInformation::new(
            self.transfer_length,
            self.symbol_size,
            self.source_blocks,
            self.sub_blocks,
            self.alignment,
        ))
    }
    pub fn source_symbols(self) -> Result<u32, Error> {
        self.bounded()?;
        Ok(self.transfer_length.div_ceil(u64::from(self.symbol_size)) as u32)
    }
    pub fn checked_packet(self, bytes: &[u8]) -> Result<EncodingPacket, Error> {
        self.bounded()?;
        if bytes.len() != 4 + usize::from(self.symbol_size) {
            return Err(Error::MalformedFrame);
        }
        let id = PayloadId::deserialize(bytes[..4].try_into().map_err(|_| Error::MalformedFrame)?);
        if id.source_block_number() >= self.source_blocks || id.encoding_symbol_id() > MAX_ESI {
            return Err(Error::ProfileMismatch);
        }
        Ok(EncodingPacket::deserialize(bytes))
    }
}
