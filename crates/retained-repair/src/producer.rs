// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{
    Error, ExpiryReport, FrameToken, MAX_EPOCH_TICK, MAX_ESI, MAX_FRAME_OCTETS, MAX_OBJECTS,
    MAX_RETAINED_ORIGINAL_OCTETS, ObjectMetadata, Profile, check_clock, wire,
};
use ed25519_dalek::SigningKey;
use raptorq::Encoder;
use std::collections::{BTreeMap, VecDeque};
use wire::{AuthenticatedFrame, Lane, Manifest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Inserted(ObjectMetadata),
    AlreadyRetained(ObjectMetadata),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Defer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlement {
    Queued,
    Deferred,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedFrame {
    pub token: FrameToken,
    pub channel: u8,
    pub lane: Lane,
    pub generation: u64,
    pub esi: Option<u32>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProducerStats {
    pub encoder_allocations: u64,
    pub queued_frames: u64,
    pub queued_wire_octets: u64,
    pub queued_manifests: u64,
    pub queued_fresh: u64,
    pub queued_repairs: u64,
    pub deferrals: u64,
    pub expired_objects: u64,
}

struct Source {
    original: Vec<u8>,
    manifest: Manifest,
    manifest_wire: Vec<u8>,
    encoder: Encoder,
    source_symbols: u32,
    fresh_cursor: u32,
    repair_cursor: u32,
    manifest_queued: bool,
}

/// Four-object producer. A prepared frame is local custody, not transmit authority.
pub struct Producer {
    profile: Profile,
    key: SigningKey,
    sources: BTreeMap<u64, Source>,
    manifest_order: VecDeque<u64>,
    repair_order: VecDeque<u64>,
    last_generation: Option<u64>,
    now: Option<u32>,
    served_channels: u8,
    pending: Option<PreparedFrame>,
    next_token: u64,
    stats: ProducerStats,
}

impl Producer {
    pub fn new(profile: Profile, key: SigningKey) -> Result<Self, Error> {
        profile.validate()?;
        if key.verifying_key() != profile.authority {
            return Err(Error::WrongAuthority);
        }
        Ok(Self {
            profile,
            key,
            sources: BTreeMap::new(),
            manifest_order: VecDeque::new(),
            repair_order: VecDeque::new(),
            last_generation: None,
            now: None,
            served_channels: 0,
            pending: None,
            next_token: 1,
            stats: ProducerStats::default(),
        })
    }
    pub fn stats(&self) -> &ProducerStats {
        &self.stats
    }
    pub fn retained_original_octets(&self) -> usize {
        self.sources
            .values()
            .map(|source| source.original.len())
            .sum()
    }
    pub fn inventory(&self) -> Vec<ObjectMetadata> {
        self.sources
            .values()
            .map(|s| s.manifest.metadata.clone())
            .collect()
    }
    pub fn lookup(&self, generation: u64) -> Option<ObjectMetadata> {
        self.sources
            .get(&generation)
            .map(|s| s.manifest.metadata.clone())
    }
    pub fn pending(&self) -> Option<&PreparedFrame> {
        self.pending.as_ref()
    }

    /// Expiry is explicit and bounded. Unqueued preparation is invalidated visibly.
    pub fn advance(&mut self, now: u32) -> Result<ExpiryReport, Error> {
        check_clock(self.now, now)?;
        if self.now != Some(now) {
            self.served_channels = 0;
        }
        let objects: Vec<_> = self
            .sources
            .values()
            .filter(|s| s.manifest.metadata.expires_at <= now)
            .map(|s| s.manifest.metadata.clone())
            .collect();
        let invalidated_token = self
            .pending
            .as_ref()
            .filter(|p| objects.iter().any(|m| m.generation == p.generation))
            .map(|p| p.token);
        if invalidated_token.is_some() {
            self.pending = None;
        }
        for object in &objects {
            self.sources.remove(&object.generation);
            self.manifest_order.retain(|id| *id != object.generation);
            self.repair_order.retain(|id| *id != object.generation);
        }
        self.stats.expired_objects += objects.len() as u64;
        self.now = Some(now);
        Ok(ExpiryReport {
            objects,
            invalidated_token,
        })
    }
    fn ready(&self, now: u32) -> Result<(), Error> {
        check_clock(self.now, now)?;
        if self.now != Some(now) {
            return Err(Error::AdvanceRequired);
        }
        Ok(())
    }

    /// Reject capacity before constructing an encoder; publication never silently evicts.
    pub fn publish(
        &mut self,
        now: u32,
        generation: u64,
        original: &[u8],
        expires_at: u32,
    ) -> Result<PublishOutcome, Error> {
        self.ready(now)?;
        let oti = self.profile.oti(original.len())?;
        if expires_at <= now || expires_at > MAX_EPOCH_TICK {
            return Err(Error::InvalidExpiry);
        }
        if let Some(source) = self.sources.get(&generation) {
            if source.original != original || source.manifest.metadata.expires_at != expires_at {
                return Err(Error::ConflictingObject);
            }
            return Ok(PublishOutcome::AlreadyRetained(
                source.manifest.metadata.clone(),
            ));
        }
        if self.last_generation.is_some_and(|last| generation <= last) {
            return Err(Error::GenerationReused);
        }
        if self.sources.len() == MAX_OBJECTS
            || self.retained_original_octets() + original.len() > MAX_RETAINED_ORIGINAL_OCTETS
        {
            return Err(Error::Full {
                next_expiry: self
                    .sources
                    .values()
                    .map(|s| s.manifest.metadata.expires_at)
                    .min(),
            });
        }
        let metadata = ObjectMetadata {
            generation,
            digest: *blake3::hash(original).as_bytes(),
            object_octets: original.len() as u32,
            publish_at: now,
            expires_at,
        };
        let proposed = Manifest {
            metadata: metadata.clone(),
            oti,
            body_hash: [0; 32],
            channels: self.profile.channels,
            manifest_interval: self.profile.manifest_interval,
            repair_floor: self.profile.repair_floor,
        };
        let manifest_wire = wire::encode_manifest(&proposed, &self.profile.namespace, &self.key)?;
        let AuthenticatedFrame::Manifest(manifest) = wire::verify(
            &manifest_wire,
            &self.profile.namespace,
            &self.profile.authority,
        )?
        else {
            return Err(Error::CodecInvariant);
        };
        let source_symbols = oti.source_symbols()?;
        let encoder = Encoder::new(original, oti.to_crate()?);
        self.stats.encoder_allocations += 1;
        self.sources.insert(
            generation,
            Source {
                original: original.to_vec(),
                manifest,
                manifest_wire,
                encoder,
                source_symbols,
                fresh_cursor: 0,
                repair_cursor: source_symbols,
                manifest_queued: false,
            },
        );
        self.manifest_order.push_back(generation);
        self.repair_order.push_back(generation);
        self.last_generation = Some(generation);
        Ok(PublishOutcome::Inserted(metadata))
    }

    /// One opportunity per logical channel/tick. None uses cadence/floor fairness;
    /// a requested generation is an explicit exact-inventory selection.
    pub fn prepare(
        &mut self,
        now: u32,
        channel: u8,
        requested_generation: Option<u64>,
    ) -> Result<Option<PreparedFrame>, Error> {
        self.ready(now)?;
        if channel >= self.profile.channels {
            return Err(Error::InvalidProfile("channel"));
        }
        if let Some(pending) = &self.pending {
            if pending.channel != channel
                || requested_generation.is_some_and(|id| id != pending.generation)
            {
                return Err(Error::PendingFrame);
            }
            return Ok(Some(pending.clone()));
        }
        if self.served_channels & (1 << channel) != 0 {
            return Ok(None);
        }
        let selected = if let Some(id) = requested_generation {
            let source = self.sources.get(&id).ok_or(Error::UnknownGeneration)?;
            let lane = if !source.manifest_queued
                || (channel == 0 && now.is_multiple_of(self.profile.manifest_interval))
            {
                Lane::Manifest
            } else if source.fresh_cursor < source.source_symbols {
                Lane::Fresh
            } else {
                Lane::Repair
            };
            Some((id, lane))
        } else {
            self.select(now, channel)
        };
        let Some((id, lane)) = selected else {
            return Ok(None);
        };
        let source = self.sources.get(&id).ok_or(Error::CodecInvariant)?;
        let esi = match lane {
            Lane::Manifest => None,
            Lane::Fresh => Some(source.fresh_cursor),
            Lane::Repair => Some(source.repair_cursor),
        };
        if esi.is_some_and(|esi| esi > MAX_ESI) {
            return Err(Error::EsiExhausted);
        }
        let bytes = if let Some(esi) = esi {
            let block = source
                .encoder
                .get_block_encoders()
                .first()
                .ok_or(Error::CodecInvariant)?;
            let packet = if esi < source.source_symbols {
                block
                    .source_packets()
                    .get(esi as usize)
                    .cloned()
                    .ok_or(Error::CodecInvariant)?
            } else {
                block
                    .repair_packets(esi - source.source_symbols, 1)
                    .into_iter()
                    .next()
                    .ok_or(Error::CodecInvariant)?
            };
            let packet = packet.serialize();
            source.manifest.oti.checked_packet(&packet)?;
            wire::encode_symbol(
                id,
                &self.profile.namespace,
                &source.manifest.body_hash,
                &packet,
                &self.key,
            )
        } else {
            source.manifest_wire.clone()
        };
        if bytes.len() > MAX_FRAME_OCTETS {
            return Err(Error::CodecInvariant);
        }
        let next = self
            .next_token
            .checked_add(1)
            .ok_or(Error::TokenExhausted)?;
        let prepared = PreparedFrame {
            token: FrameToken(self.next_token),
            channel,
            lane,
            generation: id,
            esi,
            bytes,
        };
        self.next_token = next;
        self.pending = Some(prepared.clone());
        Ok(Some(prepared))
    }
    fn select(&self, now: u32, channel: u8) -> Option<(u64, Lane)> {
        if channel == 0 && now.is_multiple_of(self.profile.manifest_interval) {
            return self
                .manifest_order
                .front()
                .copied()
                .map(|id| (id, Lane::Manifest));
        }
        let fresh = self
            .sources
            .iter()
            .filter(|(_, s)| s.fresh_cursor < s.source_symbols)
            .min_by_key(|(id, s)| (s.manifest.metadata.publish_at, **id))
            .map(|(id, _)| *id);
        let repair = self
            .repair_order
            .iter()
            .find(|id| {
                self.sources
                    .get(id)
                    .is_some_and(|s| s.repair_cursor <= MAX_ESI)
            })
            .copied();
        let index = if now.is_multiple_of(self.profile.manifest_interval) && channel > 0 {
            channel - 1
        } else {
            channel
        };
        let selected = if index < self.profile.repair_floor {
            repair
                .map(|id| (id, Lane::Repair))
                .or_else(|| fresh.map(|id| (id, Lane::Fresh)))
        } else {
            fresh
                .map(|id| (id, Lane::Fresh))
                .or_else(|| repair.map(|id| (id, Lane::Repair)))
        };
        selected.map(|(id, lane)| {
            if self.sources.get(&id).is_some_and(|s| !s.manifest_queued) {
                (id, Lane::Manifest)
            } else {
                (id, lane)
            }
        })
    }

    /// Accept means the caller's bounded sink took custody, never remote admission.
    pub fn settle(
        &mut self,
        now: u32,
        token: FrameToken,
        decision: Decision,
    ) -> Result<Settlement, Error> {
        self.ready(now)?;
        let prepared = self
            .pending
            .as_ref()
            .filter(|p| p.token == token)
            .ok_or(Error::StaleToken)?;
        if decision == Decision::Defer {
            self.stats.deferrals = self.stats.deferrals.saturating_add(1);
            return Ok(Settlement::Deferred);
        }
        let source = self
            .sources
            .get_mut(&prepared.generation)
            .ok_or(Error::StaleToken)?;
        match prepared.lane {
            Lane::Manifest => {
                source.manifest_queued = true;
                self.stats.queued_manifests += 1;
                rotate(&mut self.manifest_order, prepared.generation);
            }
            Lane::Fresh => {
                source.fresh_cursor += 1;
                self.stats.queued_fresh += 1;
            }
            Lane::Repair => {
                source.repair_cursor += 1;
                self.stats.queued_repairs += 1;
                rotate(&mut self.repair_order, prepared.generation);
            }
        }
        self.served_channels |= 1 << prepared.channel;
        self.stats.queued_frames += 1;
        self.stats.queued_wire_octets += prepared.bytes.len() as u64;
        self.pending = None;
        Ok(Settlement::Queued)
    }
}

fn rotate(queue: &mut VecDeque<u64>, id: u64) {
    queue.retain(|candidate| *candidate != id);
    queue.push_back(id);
}
