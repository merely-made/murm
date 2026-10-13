// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{
    Error, ExpiryReport, MAX_EPOCH_DECODE_WORK, MAX_OBJECTS, MAX_RETAINED_ORIGINAL_OCTETS,
    MAX_SYMBOLS_PER_OBJECT, ObjectMetadata, Profile, check_clock, wire,
};
use raptorq::Decoder;
use std::collections::{BTreeMap, BTreeSet};
use wire::{AuthenticatedFrame, Manifest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredObject {
    pub metadata: ObjectMetadata,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiveOutcome {
    ManifestAccepted(ObjectMetadata),
    DuplicateManifest(ObjectMetadata),
    SymbolAccepted,
    DuplicateSymbol,
    AlreadyCompleted,
    Completed(RecoveredObject),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectorStats {
    pub decoder_allocations: u64,
    pub decode_work: u64,
    pub completed_objects: u64,
    pub expired_objects: u64,
}
struct State {
    manifest: Manifest,
    seen: BTreeSet<u32>,
    decoder: Option<Decoder>,
    recovered: Option<RecoveredObject>,
}

/// Dynamic state learned only from signed manifests, with four bounded coding objects.
pub struct Collector {
    profile: Profile,
    now: Option<u32>,
    objects: BTreeMap<u64, State>,
    stats: CollectorStats,
}
impl Collector {
    pub fn new(profile: Profile) -> Result<Self, Error> {
        profile.validate()?;
        Ok(Self {
            profile,
            now: None,
            objects: BTreeMap::new(),
            stats: CollectorStats::default(),
        })
    }
    pub fn stats(&self) -> &CollectorStats {
        &self.stats
    }
    pub fn active_decoders(&self) -> usize {
        self.objects
            .values()
            .filter(|s| s.decoder.is_some())
            .count()
    }
    pub fn recovered_octets(&self) -> usize {
        self.objects
            .values()
            .filter_map(|s| s.recovered.as_ref())
            .map(|r| r.bytes.len())
            .sum()
    }
    pub fn recovered(&self, generation: u64) -> Option<&RecoveredObject> {
        self.objects
            .get(&generation)
            .and_then(|s| s.recovered.as_ref())
    }
    pub fn inventory(&self) -> Vec<ObjectMetadata> {
        self.objects
            .values()
            .map(|s| s.manifest.metadata.clone())
            .collect()
    }
    pub fn advance(&mut self, now: u32) -> Result<ExpiryReport, Error> {
        check_clock(self.now, now)?;
        let objects: Vec<_> = self
            .objects
            .values()
            .filter(|s| s.manifest.metadata.expires_at <= now)
            .map(|s| s.manifest.metadata.clone())
            .collect();
        for object in &objects {
            self.objects.remove(&object.generation);
        }
        self.stats.expired_objects += objects.len() as u64;
        self.now = Some(now);
        Ok(ExpiryReport {
            objects,
            invalidated_token: None,
        })
    }
    pub fn ingest(&mut self, now: u32, bytes: &[u8]) -> Result<ReceiveOutcome, Error> {
        check_clock(self.now, now)?;
        if self.now != Some(now) {
            return Err(Error::AdvanceRequired);
        }
        // Bounded borrowed envelope + signature first; no untrusted OTI allocates a decoder.
        match wire::verify(bytes, &self.profile.namespace, &self.profile.authority)? {
            AuthenticatedFrame::Manifest(manifest) => self.manifest(now, manifest),
            AuthenticatedFrame::Symbol {
                generation,
                manifest_hash,
                packet,
            } => self.symbol(now, generation, manifest_hash, &packet),
        }
    }
    fn manifest(&mut self, now: u32, manifest: Manifest) -> Result<ReceiveOutcome, Error> {
        let metadata = &manifest.metadata;
        if now < metadata.publish_at || now >= metadata.expires_at {
            return Err(Error::ExpiredFrame);
        }
        if !self.profile.matches(&manifest) {
            return Err(Error::ProfileMismatch);
        }
        if let Some(state) = self.objects.get(&metadata.generation) {
            if state.manifest != manifest {
                return Err(Error::ConflictingObject);
            }
            return Ok(ReceiveOutcome::DuplicateManifest(metadata.clone()));
        }
        let retained: usize = self
            .objects
            .values()
            .map(|s| s.manifest.metadata.object_octets as usize)
            .sum();
        if self.objects.len() == MAX_OBJECTS
            || retained + metadata.object_octets as usize > MAX_RETAINED_ORIGINAL_OCTETS
        {
            return Err(Error::Full {
                next_expiry: self
                    .objects
                    .values()
                    .map(|s| s.manifest.metadata.expires_at)
                    .min(),
            });
        }
        let metadata = metadata.clone();
        self.objects.insert(
            metadata.generation,
            State {
                manifest,
                seen: BTreeSet::new(),
                decoder: None,
                recovered: None,
            },
        );
        Ok(ReceiveOutcome::ManifestAccepted(metadata))
    }
    fn symbol(
        &mut self,
        now: u32,
        generation: u64,
        manifest_hash: [u8; 32],
        packet: &[u8],
    ) -> Result<ReceiveOutcome, Error> {
        let state = self
            .objects
            .get_mut(&generation)
            .ok_or(Error::UnknownGeneration)?;
        if now < state.manifest.metadata.publish_at || now >= state.manifest.metadata.expires_at {
            return Err(Error::ExpiredFrame);
        }
        if state.manifest.body_hash != manifest_hash {
            return Err(Error::ConflictingObject);
        }
        let packet = state.manifest.oti.checked_packet(packet)?;
        let esi = packet.payload_id().encoding_symbol_id();
        if state.recovered.is_some() {
            return Ok(ReceiveOutcome::AlreadyCompleted);
        }
        if state.seen.contains(&esi) {
            return Ok(ReceiveOutcome::DuplicateSymbol);
        }
        if state.seen.len() >= MAX_SYMBOLS_PER_OBJECT
            || self.stats.decode_work >= MAX_EPOCH_DECODE_WORK
        {
            return Err(Error::DecodeBudget);
        }
        if state.decoder.is_none() {
            state.decoder = Some(Decoder::new(state.manifest.oti.to_crate()?));
            self.stats.decoder_allocations += 1;
        }
        state.seen.insert(esi);
        self.stats.decode_work += 1;
        if let Some(bytes) = state
            .decoder
            .as_mut()
            .ok_or(Error::CodecInvariant)?
            .decode(packet)
        {
            if bytes.len() != state.manifest.metadata.object_octets as usize
                || blake3::hash(&bytes).as_bytes() != &state.manifest.metadata.digest
            {
                return Err(Error::ReconstructedDigestMismatch);
            }
            let recovered = RecoveredObject {
                metadata: state.manifest.metadata.clone(),
                bytes,
            };
            state.decoder = None;
            state.seen.clear();
            state.recovered = Some(recovered.clone());
            self.stats.completed_objects += 1;
            return Ok(ReceiveOutcome::Completed(recovered));
        }
        Ok(ReceiveOutcome::SymbolAccepted)
    }
}
