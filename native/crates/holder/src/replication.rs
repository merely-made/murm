// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The publisher's view of replication: "pending" until holders acknowledge.
//!
//! Publishing a release makes it durable locally; it is *replicated* only
//! when every configured holder has signed a statement for the current head
//! in answer to a challenge this tracker issued. A new edition resets every
//! acknowledgement, because an old statement covers an old head.
//!
//! [`AckSink`] is the seam for governed sites: Gemot P1's accepted-transfer
//! fact is the same input (a verified holder statement about a head), recorded
//! in a Moot's history instead of here. Not built; the trait names the shape.

use std::collections::BTreeMap;

use crate::ack::{Ack, AckError};
use crate::dial::{Dial, DialError, HolderTarget};

/// Where verified acknowledgements go.
pub trait AckSink {
    /// Record `ack`, which answers `challenge`.
    ///
    /// # Errors
    /// The acknowledgement does not verify for this sink's holders and log.
    fn accept(&mut self, ack: &Ack, challenge: &[u8; 32]) -> Result<(), AckError>;
}

/// Where publishing stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicationStatus {
    /// No holder is configured, so nothing can acknowledge.
    Unconfigured,
    /// Holders that have not acknowledged the current head.
    Pending { awaiting: Vec<[u8; 32]> },
    /// Every configured holder acknowledged the current head.
    Replicated,
}

/// Tracks acknowledgements of one log's current head.
pub struct Replication {
    log: [u8; 32],
    head: [u8; 32],
    seq: u32,
    holders: Vec<[u8; 32]>,
    acks: BTreeMap<[u8; 32], Ack>,
}

impl Replication {
    /// Track `log` at `head` (hash and sequence) against `holders` (their keys).
    pub fn new(log: [u8; 32], head: [u8; 32], seq: u32, holders: Vec<[u8; 32]>) -> Self {
        Self {
            log,
            head,
            seq,
            holders,
            acks: BTreeMap::new(),
        }
    }

    /// A new edition was published: earlier acknowledgements no longer count.
    pub fn advance(&mut self, head: [u8; 32], seq: u32) {
        self.head = head;
        self.seq = seq;
        self.acks.clear();
    }

    /// A fresh challenge for one request.
    ///
    /// # Panics
    /// The operating system has no randomness.
    pub fn challenge() -> [u8; 32] {
        let mut challenge = [0u8; 32];
        getrandom::fill(&mut challenge).expect("system randomness");
        challenge
    }

    /// Where publishing stands now.
    pub fn status(&self) -> ReplicationStatus {
        if self.holders.is_empty() {
            return ReplicationStatus::Unconfigured;
        }
        let awaiting: Vec<_> = self
            .holders
            .iter()
            .filter(|holder| !self.acks.contains_key(*holder))
            .copied()
            .collect();
        if awaiting.is_empty() {
            ReplicationStatus::Replicated
        } else {
            ReplicationStatus::Pending { awaiting }
        }
    }
}

impl AckSink for Replication {
    fn accept(&mut self, ack: &Ack, challenge: &[u8; 32]) -> Result<(), AckError> {
        if !self.holders.contains(&ack.holder) {
            return Err(AckError::WrongHolder);
        }
        ack.verify(&ack.holder, &self.log, challenge)?;
        // A holder that is behind has not taken the current head: not an error,
        // just not yet an acknowledgement.
        if ack.head == self.head && ack.seq == self.seq {
            self.acks.insert(ack.holder, ack.clone());
        }
        Ok(())
    }
}

/// How one holder fared in a round of requests.
#[derive(Debug)]
pub struct Round {
    pub holder: [u8; 32],
    pub result: Result<(), RoundError>,
}

/// Why a holder gave no acknowledgement this round.
#[derive(Debug, thiserror::Error)]
pub enum RoundError {
    #[error(transparent)]
    Dial(#[from] DialError),
    #[error(transparent)]
    Ack(#[from] AckError),
}

/// Ask each target for an acknowledgement over the first dialer that handles
/// its reach, and record what verifies. Targets already acknowledged are
/// skipped.
pub async fn request_acks(
    replication: &mut Replication,
    targets: &[HolderTarget],
    dialers: &[&dyn Dial],
) -> Vec<Round> {
    let mut rounds = Vec::new();
    for target in targets {
        let holder = target.holder.to_bytes();
        if replication.acks.contains_key(&holder) {
            continue;
        }
        let challenge = Replication::challenge();
        let result = match dialers.iter().find(|dialer| dialer.handles(&target.reach)) {
            None => Err(DialError::NoCarrier.into()),
            Some(dialer) => match dialer.ask(target, replication.log, challenge).await {
                Ok(ack) if ack.holder != holder => Err(AckError::WrongHolder.into()),
                Ok(ack) => replication
                    .accept(&ack, &challenge)
                    .map_err(RoundError::from),
                Err(error) => Err(error.into()),
            },
        };
        rounds.push(Round { holder, result });
    }
    rounds
}
