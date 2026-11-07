// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A holder: retains verified editions of the logs it agreed to hold.
//!
//! Acquisition reuses the resolver, so every carrier the resolver can dial
//! (memory, Noise over TCP, Reticulum, p2panda) feeds a holder, and nothing is
//! stored that did not verify as one log under its own key. The retention
//! bound is explicit: a cap on editions per log, a cap on logs, and a time
//! after which the agreement lapses and the editions are dropped at the next
//! [`sweep`](HolderNode::sweep). A holder never rewinds: a shorter valid chain
//! adds nothing, and a conflicting one is refused.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use identity::Ed25519Keypair;
use muniment::{Backend, MemoryBackend};
use murm_resolve::{Hint, Holder, LogAddress, ResolveError, Resolver};
use stickleback::edition::{EditionError, EditionLimits, verify_edition_chain};

use crate::service::HolderService;

/// How long and how much a holder keeps of one log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    /// Most editions kept; a longer log is refused whole, never truncated.
    pub max_editions: usize,
    /// When the agreement lapses, in the caller's clock (milliseconds).
    pub hold_until_ms: u64,
}

/// Why a holder did not take or keep a log.
#[derive(Debug, thiserror::Error)]
pub enum HolderError {
    /// No agreement covers this key.
    #[error("no agreement to hold this log")]
    NotAgreed,
    /// The node already holds as many logs as it allows.
    #[error("holder is at its limit of {0} logs")]
    TooManyLogs(usize),
    /// The log is longer than the agreement allows.
    #[error("log has {have} editions, the agreement allows {max}")]
    OverBound { have: usize, max: usize },
    /// No source gave a verified chain.
    #[error("no source gave a verified chain")]
    Resolve(Box<ResolveError>),
    /// What was fetched conflicts with, or does not extend, what is held.
    #[error("fetched editions conflict with the retained log: {0}")]
    Conflict(EditionError),
    /// The retained editions no longer verify.
    #[error("retained editions do not verify: {0}")]
    Retained(EditionError),
}

/// What one acquisition changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acquired {
    /// Sequence of the newest edition held before, if any.
    pub before: Option<u32>,
    /// Sequence of the newest edition held now.
    pub after: u32,
    /// How many editions were new.
    pub added: usize,
    /// The carrier that produced the chain.
    pub via: &'static str,
}

pub(crate) type Agreements = Arc<Mutex<BTreeMap<[u8; 32], Retention>>>;

pub(crate) fn lock(agreements: &Agreements) -> MutexGuard<'_, BTreeMap<[u8; 32], Retention>> {
    agreements
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A holder over a store, signing acknowledgements as `signer`.
pub struct HolderNode<B = MemoryBackend> {
    pub(crate) holder: Arc<Holder<B>>,
    pub(crate) agreements: Agreements,
    pub(crate) signer: Arc<Ed25519Keypair>,
    max_logs: usize,
}

impl<B: Backend + Clone + Send + Sync + 'static> HolderNode<B> {
    /// A node over `backend`. `signer` is the key its Noise sessions prove,
    /// so a publisher that dialed the holder by that key can check its
    /// acknowledgements. It holds at most `max_logs` logs.
    pub fn new(backend: B, signer: Ed25519Keypair, max_logs: usize) -> Self {
        Self {
            holder: Arc::new(Holder::new(backend)),
            agreements: Arc::default(),
            signer: Arc::new(signer),
            max_logs,
        }
    }

    /// Agree to hold `key` under `retention`, or renew an agreement.
    ///
    /// # Errors
    /// The node is at its log limit.
    pub fn agree(&self, key: [u8; 32], retention: Retention) -> Result<(), HolderError> {
        let mut agreements = lock(&self.agreements);
        if !agreements.contains_key(&key) && agreements.len() >= self.max_logs {
            return Err(HolderError::TooManyLogs(self.max_logs));
        }
        agreements.insert(key, retention);
        Ok(())
    }

    /// Whether an agreement covers `key`.
    pub fn is_agreed(&self, key: &[u8; 32]) -> bool {
        lock(&self.agreements).contains_key(key)
    }

    /// End an agreement and drop its editions now.
    pub async fn release(&self, key: &[u8; 32]) -> usize {
        lock(&self.agreements).remove(key);
        self.holder.forget(key).await
    }

    /// Drop every agreement lapsed at `now_ms`, with its editions. Returns
    /// the keys dropped.
    pub async fn sweep(&self, now_ms: u64) -> Vec<[u8; 32]> {
        let lapsed: Vec<_> = lock(&self.agreements)
            .iter()
            .filter(|(_, retention)| retention.hold_until_ms <= now_ms)
            .map(|(key, _)| *key)
            .collect();
        for key in &lapsed {
            self.release(key).await;
        }
        lapsed
    }

    /// The newest edition held for `key`: its hash and sequence.
    ///
    /// # Errors
    /// The retained editions do not verify.
    pub async fn head(&self, key: &[u8; 32]) -> Result<Option<([u8; 32], u32)>, EditionError> {
        Ok(self.holder.chain(key).await?.map(|chain| {
            let head = chain.head();
            (head.id.0, head.seq_num)
        }))
    }

    /// Pull `key` from whatever `resolver` can reach, verify, and retain.
    ///
    /// Give the resolver every carrier the holder enables. An
    /// [`exhaustive`](Resolver::exhaustive) resolver merges all sources;
    /// otherwise the first that verifies wins. Each call re-pulls the whole
    /// log; incremental catch-up from the held head is not built.
    ///
    /// # Errors
    /// See [`HolderError`]. Nothing is stored on an error.
    pub async fn acquire(
        &self,
        resolver: &Resolver,
        key: [u8; 32],
        hints: &[Hint],
    ) -> Result<Acquired, HolderError> {
        let retention = *lock(&self.agreements)
            .get(&key)
            .ok_or(HolderError::NotAgreed)?;
        let address = LogAddress::new(key, "index.gmi");
        let fetched = resolver
            .resolve(&address, hints)
            .await
            .map_err(|error| HolderError::Resolve(Box::new(error)))?
            .chain;
        let before = self
            .holder
            .chain(&key)
            .await
            .map_err(HolderError::Retained)?;
        let mut all = before
            .as_ref()
            .map_or_else(Vec::new, |c| c.operations().to_vec());
        all.extend_from_slice(fetched.operations());
        let merged = verify_edition_chain(&key, &all, EditionLimits::default())
            .map_err(HolderError::Conflict)?;
        if merged.editions().len() > retention.max_editions {
            return Err(HolderError::OverBound {
                have: merged.editions().len(),
                max: retention.max_editions,
            });
        }
        let added = self.holder.hold(merged.operations()).await;
        Ok(Acquired {
            before: before.map(|chain| chain.head().seq_num),
            after: merged.head().seq_num,
            added,
            via: "resolver",
        })
    }

    /// The session to hand a lane for one accepted stream: it serves agreed
    /// logs and answers acknowledgement requests.
    pub fn service(&self) -> HolderService<B> {
        HolderService::new(self)
    }
}
