// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Resolve a `knot://` release-log address to a verified edition chain.
//!
//! A reader holding only an address asks each enabled carrier for holders,
//! pulls the log from them with `murm-sync` through the shared setup gate, and
//! verifies it with `verify_edition_chain` against the key in the address. The
//! reader's store admits only that key's editions, so a holder (or a hint's
//! target) serving another log is refused, never believed. `@<hash>` pins an
//! edition the chain must contain.
//!
//! The governed `.moot` form is not resolved here: [`ResolveError::Governed`]
//! hands back the Moot, collection and page for Gemot's reader verification
//! (`ReaderTrust::for_address`), which also needs the founder the reader
//! trusts, something an address does not carry.
//!
//! Success shows the newest edition the contacted holders had, not the newest
//! that exists. Carriers are opt-in features; memory and Noise over TCP need
//! none.

mod carrier;
mod hint;
#[cfg(feature = "p2panda")]
mod p2panda;
#[cfg(feature = "reticulum")]
mod reticulum;
mod wire;

use gemot_address::{AddressHost, KnotAddress};
use muniment::MemoryBackend;
use murm_sync::{StreamSyncError, StreamSyncLimits, StreamSyncOutcome};
use stickleback::OperationProcessor;
use stickleback::edition::{
    Edition, EditionChain, EditionError, EditionId, EditionLimits, EditionPolicy,
    verify_edition_chain,
};
use transport::sync_lane::SyncLaneError;

pub use carrier::{Ask, Carrier, Local, NoiseTcpAdapter, PullError, Route, TransportAdapter};
pub use hint::{Hint, HintError};
#[cfg(feature = "p2panda")]
pub use p2panda::P2pandaAdapter;
#[cfg(feature = "reticulum")]
pub use reticulum::reticulum_adapter;
pub use wire::{
    EditionStore, Holder, HolderSession, NOT_HELD, RELEASE_PROTOCOL, ReadSession, key_alpn,
    release_alpn,
};

/// Why one route did not give a usable log.
#[derive(Debug)]
pub enum Failure {
    /// The carrier or lane could not complete the session.
    Unreachable(String),
    /// The holder does not hold this log.
    NotHeld,
    /// The holder sent operations the log's policy refused: another log, a
    /// forgery or a conflicting edition.
    Mismatch { declined: u64 },
    /// What arrived does not verify as a chain.
    Invalid(EditionError),
    /// The chain verifies but lacks the pinned edition.
    PinMissing,
}

/// One route tried and how it ended.
#[derive(Debug)]
pub struct Attempt {
    pub carrier: &'static str,
    pub route: String,
    pub result: Result<StreamSyncOutcome, Failure>,
}

/// Why an address did not resolve.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The governed form: verify through Gemot with the reader's founder.
    #[error("governed address: verify through Gemot's reader evidence")]
    Governed {
        moot_id: [u8; 32],
        collection_id: [u8; 32],
        page: String,
    },
    /// No enabled carrier gave a verified log. Each attempt says why.
    #[error("no holder gave a verified log")]
    Unresolved { attempts: Vec<Attempt> },
}

/// A verified resolution.
#[derive(Debug)]
pub struct Resolved {
    /// Every edition the contacted holders showed, verified under the key.
    pub chain: EditionChain,
    /// The pinned edition, or the chain's head when the address pins none.
    pub selected: Edition,
    /// The page or file inside the edition, as the address wrote it.
    pub path: String,
    /// The carrier and route of the first attempt that produced the chain.
    pub via: (&'static str, String),
    /// Every attempt, in order, successful or not.
    pub attempts: Vec<Attempt>,
}

/// Tries each enabled carrier's routes until a log verifies.
pub struct Resolver {
    carriers: Vec<Box<dyn Carrier>>,
    edition: EditionLimits,
    sync: StreamSyncLimits,
    exhaustive: bool,
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolver {
    /// A resolver with no carriers.
    pub fn new() -> Self {
        Self {
            carriers: Vec::new(),
            edition: EditionLimits::default(),
            sync: StreamSyncLimits::default(),
            exhaustive: false,
        }
    }

    /// Enable a carrier. Carriers are tried in the order added.
    #[must_use]
    pub fn with(mut self, carrier: impl Carrier + 'static) -> Self {
        self.carriers.push(Box::new(carrier));
        self
    }

    /// Pull from every route and verify the union, to see the newest log any
    /// reachable holder has, instead of stopping at the first that verifies.
    #[must_use]
    pub fn exhaustive(mut self) -> Self {
        self.exhaustive = true;
        self
    }

    /// Resolve `address`, trying `hints` as extra routes.
    ///
    /// # Errors
    /// [`ResolveError::Governed`] for the `.moot` form;
    /// [`ResolveError::Unresolved`] when no route produced a verified chain
    /// (holding the pin, when the address has one).
    pub async fn resolve(
        &self,
        address: &KnotAddress,
        hints: &[Hint],
    ) -> Result<Resolved, ResolveError> {
        let AddressHost::LogKey(key) = address.host else {
            let parts = address
                .collection()
                .expect("governed addresses name a collection");
            return Err(ResolveError::Governed {
                moot_id: parts.moot_id,
                collection_id: parts.collection_id,
                page: parts.page.to_string(),
            });
        };
        let store = EditionStore::<MemoryBackend>::new(MemoryBackend::new());
        let policy = EditionPolicy {
            only_key: Some(key),
            limits: self.edition,
        };
        let processor = OperationProcessor::new(store.clone(), policy);
        let ask = Ask { key, hints };
        let mut attempts = Vec::new();
        let mut via = None;
        for carrier in &self.carriers {
            for route in carrier.routes(&ask).await {
                let session = ReadSession {
                    key,
                    store: store.clone(),
                    processor: processor.clone(),
                    limits: self.sync,
                };
                let result = match carrier.pull(&route, &ask, session).await {
                    Ok(outcome) => self.judge(&store, &key, address.pin, outcome).await,
                    Err(error) => Err(classify(error)),
                };
                let verified = result.is_ok();
                attempts.push(Attempt {
                    carrier: carrier.name(),
                    route: route.label.clone(),
                    result: result.map(|(outcome, _)| outcome),
                });
                if verified && via.is_none() {
                    via = Some((carrier.name(), route.label));
                    if !self.exhaustive {
                        return self.finish(&store, &key, address, via, attempts).await;
                    }
                }
            }
        }
        if via.is_some() {
            return self.finish(&store, &key, address, via, attempts).await;
        }
        Err(ResolveError::Unresolved { attempts })
    }

    /// Check what the store now holds after one pull.
    async fn judge(
        &self,
        store: &EditionStore,
        key: &[u8; 32],
        pin: Option<[u8; 32]>,
        outcome: StreamSyncOutcome,
    ) -> Result<(StreamSyncOutcome, EditionChain), Failure> {
        if outcome.accepted == 0 && outcome.declined > 0 {
            return Err(Failure::Mismatch {
                declined: outcome.declined,
            });
        }
        let chain = self.chain(store, key).await?;
        match pin {
            Some(pin) if chain.get(&EditionId(pin)).is_none() => Err(Failure::PinMissing),
            _ => Ok((outcome, chain)),
        }
    }

    async fn chain(&self, store: &EditionStore, key: &[u8; 32]) -> Result<EditionChain, Failure> {
        let author = stickleback::edition::VerifyingKey::from_bytes(key)
            .map_err(|error| Failure::Unreachable(error.to_string()))?;
        let held = store
            .get_log_entries(&author, &0u64, None, None)
            .await
            .map_err(|error| Failure::Unreachable(error.to_string()))?
            .ok_or(Failure::NotHeld)?;
        let operations: Vec<_> = held.into_iter().map(|(operation, _)| operation).collect();
        verify_edition_chain(key, &operations, self.edition).map_err(Failure::Invalid)
    }

    async fn finish(
        &self,
        store: &EditionStore,
        key: &[u8; 32],
        address: &KnotAddress,
        via: Option<(&'static str, String)>,
        attempts: Vec<Attempt>,
    ) -> Result<Resolved, ResolveError> {
        let chain = match self.chain(store, key).await {
            Ok(chain) => chain,
            Err(_) => return Err(ResolveError::Unresolved { attempts }),
        };
        let selected = match address.pin {
            Some(pin) => chain.get(&EditionId(pin)).cloned(),
            None => Some(chain.head().clone()),
        };
        match (selected, via) {
            (Some(selected), Some(via)) => Ok(Resolved {
                chain,
                selected,
                path: address.path.clone(),
                via,
                attempts,
            }),
            _ => Err(ResolveError::Unresolved { attempts }),
        }
    }
}

fn classify(error: PullError) -> Failure {
    match error {
        PullError::Lane(SyncLaneError::Sync(StreamSyncError::Protocol(message)))
            if message == NOT_HELD =>
        {
            Failure::NotHeld
        },
        other => Failure::Unreachable(other.to_string()),
    }
}
