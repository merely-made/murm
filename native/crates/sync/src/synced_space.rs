// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The reconciling-log drain, shared by every LogSync consumer.
//!
//! [`SyncedSpace`] is the deduplicated core the former near-identical copies
//! under direct exchange, Mesh, and Standing all share: the drain loop over a
//! subscribed p2panda-net LogSync stream, the real
//! (non-placebo) [`SyncStatus`] counters, the settle-based [`resync`](SyncedSpace::resync)
//! checkpoint, and the drop-aborts-the-task lifetime.
//!
//! ## The seam
//!
//! The one thing that differed across the copies was how a received operation is
//! turned into a stored fact: Murm ingests through its cable engine, Standing
//! verifies then does a synchronous redb insert, and Mesh verifies with an
//! addressed-mesh guard before an asynchronous sqlite insert. That is the injected
//! `accept` closure — `FnMut(Operation<E>) -> impl Future<Output = bool>` — which
//! returns whether the operation counted (verified and was new). An async return
//! covers both the sync inserts (a ready future) and the async ones.
//!
//! ## Ownership
//!
//! The caller keeps its own `LogSync` session and its `SyncHandle` (for liveness
//! and for any live publish, as mesh does); `SyncedSpace` owns only the drain
//! task. Build the session, `subscribe` it, and hand the subscription here:
//!
//! ```ignore
//! let log_sync = LogSync::builder(store, endpoint, gossip).spawn().await?;
//! let handle = log_sync.stream(Topic::from(space_id), true).await?;
//! let sub = handle.subscribe().await?;
//! let space = SyncedSpace::drive(sub, move |op| {
//!     let ok = verify(&op) && insert(&op);      // sync consumer
//!     std::future::ready(ok)
//! });
//! // keep `log_sync` (and `handle`, if you publish on it) alive alongside `space`.
//! ```

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use p2panda_core::{Extensions, Operation};
use p2panda_net::sync::SyncSubscription;
use p2panda_sync::protocols::TopicLogSyncEvent;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

/// Unix-epoch milliseconds, for stamping the last sync activity.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A snapshot of a space's LogSync activity, for a real (non-placebo) sync
/// indicator. Counters accumulate over the session; read one with
/// [`SyncedSpace::sync_status`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStatus {
    /// A LogSync reconciliation round is currently in progress.
    pub syncing: bool,
    /// LogSync rounds that have finished (a peer's catch-up completing).
    pub sync_rounds: u64,
    /// Operations caught up via LogSync (the ones the `accept` closure counted).
    pub ops_received: u64,
    /// Unix-epoch milliseconds of the most recent sync activity, or `None` if
    /// nothing has arrived yet.
    pub last_activity_ms: Option<u64>,
}

/// The result of a manual [`SyncedSpace::resync`] checkpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncRound {
    /// New operations that landed during the checkpoint window. `0` means
    /// already up to date.
    pub items_received: u64,
}

/// The drain half of a reconciling log session.
///
/// Owns the background task draining a subscribed LogSync stream into an injected
/// `accept` closure, plus the sync-status counters the task writes. The caller
/// keeps the `LogSync` + `SyncHandle` that produced the subscription. Dropping
/// this aborts the drain task.
pub struct SyncedSpace {
    task: JoinHandle<()>,
    status: Arc<Mutex<SyncStatus>>,
}

impl SyncedSpace {
    /// Spawn the drain over a subscribed LogSync stream.
    ///
    /// Drains `sub`, calling `accept` on each received operation and counting the
    /// ones it accepts (returns `true`). `accept` absorbs the per-consumer verify
    /// + insert; returning a future lets a synchronous insert hand back
    ///   [`std::future::ready`] while an asynchronous one awaits its store.
    pub fn drive<E, A, Fut>(mut sub: SyncSubscription<TopicLogSyncEvent<E>>, mut accept: A) -> Self
    where
        E: Extensions + Send + 'static,
        A: FnMut(Operation<E>) -> Fut + Send + 'static,
        Fut: Future<Output = bool> + Send,
    {
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let task_status = Arc::clone(&status);
        let task = tokio::spawn(async move {
            while let Some(item) = sub.next().await {
                let Ok(from_sync) = item else { continue };
                match from_sync.event {
                    TopicLogSyncEvent::SyncStarted { .. } => {
                        let mut s = task_status.lock().unwrap();
                        s.syncing = true;
                        s.last_activity_ms = Some(now_ms());
                    },
                    TopicLogSyncEvent::OperationReceived { operation, .. } => {
                        if accept(*operation).await {
                            let mut s = task_status.lock().unwrap();
                            s.ops_received += 1;
                            s.last_activity_ms = Some(now_ms());
                        }
                    },
                    TopicLogSyncEvent::SyncFinished { .. } => {
                        let mut s = task_status.lock().unwrap();
                        s.syncing = false;
                        s.sync_rounds += 1;
                        s.last_activity_ms = Some(now_ms());
                    },
                    _ => {},
                }
            }
        });
        Self { task, status }
    }

    /// A snapshot of this session's sync activity.
    pub fn sync_status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }

    /// The running accepted-operation count. For callers that compose their own
    /// status across a second lane (murm's gossip lane, say), read this and add
    /// their own counter rather than delegating to [`sync_status`](Self::sync_status).
    pub fn ops_received(&self) -> u64 {
        self.status.lock().unwrap().ops_received
    }

    /// A shared handle on this lane's counters.
    ///
    /// For a host watching several lanes for arrivals from a task that cannot
    /// borrow the handles themselves: sampling through this observes the same
    /// counter [`sync_status`](Self::sync_status) reports, without holding the
    /// space alive or duplicating it.
    pub fn status_handle(&self) -> Arc<Mutex<SyncStatus>> {
        Arc::clone(&self.status)
    }

    /// Run a manual "sync now" checkpoint and report what arrived.
    ///
    /// LogSync reconciles **continuously**, so this is not a fake spinner and not
    /// a forced re-fetch (p2panda 0.6.1 exposes no "re-initiate" hook): it watches
    /// the live session until it goes quiet and returns the real count of
    /// operations that landed during the window. A quiet, already-synced space
    /// reports `0` quickly. Callers that track a second lane should watch their
    /// combined counter instead (see [`ops_received`](Self::ops_received)).
    pub async fn resync(&self) -> SyncRound {
        let start = self.ops_received();
        let mut last = start;
        let mut quiet = 0u8;
        // Poll up to ~3s; stop after ~300ms with nothing new (settled).
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let now = self.ops_received();
            if now == last {
                quiet += 1;
                if quiet >= 3 {
                    break;
                }
            } else {
                last = now;
                quiet = 0;
            }
        }
        SyncRound {
            items_received: last.saturating_sub(start),
        }
    }

    /// Abort the drain and wait until its captured acceptance store is
    /// released.
    pub async fn shutdown(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for SyncedSpace {
    fn drop(&mut self) {
        self.task.abort();
    }
}
