// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The join ceremony over [`SyncedSpace`], owned once.
//!
//! Every LogSync consumer used to hand-write the same sequence: build the
//! session over its store, open a live stream on its topic, subscribe, hand
//! the subscription to [`SyncedSpace::drive`], then keep the session and
//! stream handle alive in drop-order-sensitive fields. Seven copies existed
//! (mesh, murm, gemot's four lane proofs plus its example) and the
//! domain-specific part was three values: the store, the topic, and the
//! `accept` closure. [`JoinedSpace`] is that ceremony as one call.
//!
//! ## Type shape
//!
//! Generic over the extensions `E` only. The store and log-id types are
//! erased at construction (the session is kept alive as an opaque box), so a
//! holder names one type — `JoinedSpace<CabalExt>` — instead of the full
//! `LogSync<Store, LogId, Ext>` triple, and stays `Send + Sync` without
//! hand-rolled keepalive erasure. The cost is that `L` cannot be inferred
//! from a store that implements `LogStore` for every log id (muniment's
//! does), so a call site pins it: `JoinedSpace::join::<_, u64, _, _>(…)`.
//!
//! ## Ownership and drop order
//!
//! Dropping a `JoinedSpace` aborts the drain task first, then closes the
//! stream handle, then stops the session actor — the ordering every copy had
//! to get right by field position is now fixed here once.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

use p2panda_core::{AnyOperation, Extensions, Hash, LogId, Operation, SeqNum, Topic, VerifyingKey};
use p2panda_net::sync::SyncHandle;
use p2panda_net::{Endpoint, Gossip, LogSync};
use p2panda_store::logs::LogStore;
use p2panda_store::topics::TopicStore;
use p2panda_sync::protocols::TopicLogSyncEvent;

use crate::JoinError;
use crate::synced_space::{SyncRound, SyncStatus, SyncedSpace};

/// A joined reconciling-log session: the LogSync session, its live stream
/// handle, and the [`SyncedSpace`] drain, held together with the correct
/// drop order.
pub struct JoinedSpace<E>
where
    E: Extensions + Send + 'static,
{
    /// Dropped first: aborts the drain task before the session ends.
    space: SyncedSpace,
    handle: SyncHandle<Operation<E>, TopicLogSyncEvent<E>>,
    /// Keeps the session actor alive; store and log-id types erased.
    log_sync: Box<dyn LogSyncLifetime>,
}

trait LogSyncLifetime: Send + Sync {
    fn shutdown(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
}

impl<S, L, E> LogSyncLifetime for LogSync<S, L, E>
where
    S: LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash>
        + TopicStore<Topic, VerifyingKey, L>
        + Clone
        + Send
        + Sync
        + 'static,
    L: LogId + Debug + Send + Sync + 'static,
    E: Extensions + Send + Sync + 'static,
{
    fn shutdown(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move { (*self).shutdown().await.map_err(|error| error.to_string()) })
    }
}

impl<E> JoinedSpace<E>
where
    E: Extensions + Send + 'static,
{
    /// Join a topic's LogSync session over `store`, in live mode, draining
    /// received operations into `accept`.
    ///
    /// `accept` returns whether the operation counted (verified and new); a
    /// synchronous consumer hands back [`std::future::ready`]. `endpoint` +
    /// `gossip` come from the host transport's `sync_parts`.
    ///
    /// `topic` takes anything convertible, so a caller passes its raw 32-byte
    /// space id and never names a p2panda type — which is what lets a domain
    /// lane join without importing p2panda at all.
    ///
    /// `lane` names this session's sync protocol (its ALPN), and it is
    /// required rather than defaulted because the endpoint keeps exactly one
    /// handler per protocol id: two sessions sharing an id means the
    /// last-joined one silently receives ALL inbound sync and every other
    /// stops converging (measured, 2026-07-31, gemot's lane-coexistence
    /// receipt). Scope it to the lane kind AND the space —
    /// `gemot/constitution/v1/<moot-hex>` — since two spaces of the same kind
    /// on one endpoint collide identically. Both peers must derive the same
    /// string; the transport hashes it with the network id before the wire,
    /// so the readable form never travels.
    pub async fn join<S, L, A, Fut>(
        lane: impl AsRef<[u8]>,
        store: S,
        endpoint: Endpoint,
        gossip: Gossip,
        topic: impl Into<Topic>,
        accept: A,
    ) -> Result<Self, JoinError>
    where
        S: LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash>
            + TopicStore<Topic, VerifyingKey, L>
            + Clone
            + Send
            + Sync
            + 'static,
        L: LogId + Debug + Send + Sync + 'static,
        E: Sync,
        A: FnMut(Operation<E>) -> Fut + Send + 'static,
        Fut: Future<Output = bool> + Send,
    {
        let log_sync = LogSync::<S, L, E>::builder(store, endpoint, gossip)
            .protocol_id(lane)
            .spawn()
            .await
            .map_err(|e| JoinError::Spawn(e.to_string()))?;
        let handle = log_sync
            .stream(topic.into(), true)
            .await
            .map_err(|e| JoinError::Stream(e.to_string()))?;
        let sub = handle
            .subscribe()
            .await
            .map_err(|e| JoinError::Subscribe(e.to_string()))?;
        let space = SyncedSpace::drive(sub, accept);
        Ok(Self {
            space,
            handle,
            log_sync: Box::new(log_sync),
        })
    }

    /// Push a newly authored operation onto the live lane so connected peers
    /// receive it without waiting for a reconciliation round.
    pub fn publish(&self, operation: Operation<E>) -> Result<(), JoinError> {
        self.handle
            .publish(operation)
            .map_err(|e| JoinError::Publish(e.to_string()))
    }

    /// Leave the space: abort the drain, close the stream, stop the session.
    ///
    /// Exactly what dropping does — named so a caller can say it, and so a
    /// deliberate leave reads differently from a value going out of scope.
    pub fn leave(self) {}

    /// Leave and wait until the drain and sync actor have released their
    /// captured store handles.
    ///
    /// Resident processes use this stronger boundary before reopening a
    /// single-writer backend. Ordinary scope-based holders may keep using
    /// [`Self::leave`] or drop.
    pub async fn leave_and_wait(self) -> Result<(), JoinError> {
        let Self {
            space,
            handle,
            log_sync,
        } = self;
        space.shutdown().await;
        drop(handle);
        log_sync.shutdown().await.map_err(JoinError::Shutdown)
    }

    /// The drain, for callers composing their own status across a second
    /// lane (murm's gossip counters, say).
    pub fn space(&self) -> &SyncedSpace {
        &self.space
    }

    /// A shared handle on this lane's counters; see
    /// [`SyncedSpace::status_handle`].
    pub fn status_handle(&self) -> std::sync::Arc<std::sync::Mutex<SyncStatus>> {
        self.space.status_handle()
    }

    /// A snapshot of this session's sync activity.
    pub fn sync_status(&self) -> SyncStatus {
        self.space.sync_status()
    }

    /// The running accepted-operation count.
    pub fn ops_received(&self) -> u64 {
        self.space.ops_received()
    }

    /// Run a manual "sync now" checkpoint and report what arrived. Delegates
    /// to the drain's settle-based checkpoint.
    pub async fn resync(&self) -> SyncRound {
        self.space.resync().await
    }
}
