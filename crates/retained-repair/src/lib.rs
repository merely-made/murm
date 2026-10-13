// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A host-side, sans-I/O retained-object producer and dynamic collector.
//!
//! Call [`Producer::advance`] and [`Collector::advance`] with the owner's
//! monotonic logical clock before other calls at that time. The producer owns
//! original bytes and coding cursors; the caller owns sink queues, airtime,
//! radio activation, persistence and application admission. Sink acceptance
//! advances a prepared frame exactly once; deferral preserves its identity.
//! The existing authored v1 wire epoch ends at logical tick 8192. A new epoch
//! requires a new namespace; this library never wraps a clock or generation.

mod collector;
mod producer;
mod profile;
pub mod wire;

pub use collector::{Collector, CollectorStats, ReceiveOutcome, RecoveredObject};
pub use producer::{Decision, PreparedFrame, Producer, ProducerStats, PublishOutcome, Settlement};
pub use profile::{Oti, Profile};
pub use wire::Lane;

pub const MAX_OBJECTS: usize = 4;
pub const MAX_OBJECT_OCTETS: usize = 16 * 1024;
/// Original payload retention only, not total codec/heap memory.
pub const MAX_RETAINED_ORIGINAL_OCTETS: usize = 64 * 1024;
pub const MAX_FRAME_OCTETS: usize = 512;
pub const MAX_SYMBOLS_PER_OBJECT: usize = 512;
pub const MAX_EPOCH_DECODE_WORK: u64 = 4096;
pub const MAX_EPOCH_TICK: u32 = 8192;
pub const MAX_ESI: u32 = 4095;

/// A local preparation capability. It is not remote authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameToken(pub u64);

/// Exact local retention metadata, keyed by original object digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub generation: u64,
    pub digest: [u8; 32],
    pub object_octets: u32,
    pub publish_at: u32,
    pub expires_at: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    pub objects: Vec<ObjectMetadata>,
    pub invalidated_token: Option<FrameToken>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidProfile(&'static str),
    WrongAuthority,
    ClockRegression,
    EpochExhausted,
    AdvanceRequired,
    InvalidExpiry,
    ObjectSize,
    Full { next_expiry: Option<u32> },
    GenerationReused,
    ConflictingObject,
    UnknownGeneration,
    PendingFrame,
    StaleToken,
    TokenExhausted,
    EsiExhausted,
    MalformedFrame,
    InvalidSignature,
    ProfileMismatch,
    ExpiredFrame,
    DecodeBudget,
    ReconstructedDigestMismatch,
    CodecInvariant,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "retained repair: {self:?}")
    }
}
impl std::error::Error for Error {}

pub(crate) fn check_clock(last: Option<u32>, now: u32) -> Result<(), Error> {
    if now > MAX_EPOCH_TICK {
        return Err(Error::EpochExhausted);
    }
    if last.is_some_and(|last| now < last) {
        return Err(Error::ClockRegression);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
