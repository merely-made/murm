// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A p2panda address book over a muniment [`Backend`].
//!
//! This is the node-information store `p2panda-net` keeps its transport
//! addresses, bootstrap flags and per-node topic interests in. It is the
//! transport layer's own directory, unrelated to mere's handle resolution.
//!
//! It exists so that the address book joins everything else mere stores: one
//! [`Backend`], one durability story, and no second embedded database. Before
//! this, `p2panda-net` bound its bundled SQLite store concretely, which pulled
//! sqlx into any graph that did networking and could never reach wasm.
//!
//! ## Layout
//!
//! ```text
//! node/<node-id-hex>    -> Record { info, updated_at }
//! topics/<node-id-hex>  -> Vec<Topic>
//! ```
//!
//! `updated_at` is a *local* wall-clock stamp, refreshed on every write, which
//! is what [`AddressBookStore::remove_older_than`] ages entries by. It is
//! deliberately not the author's own timestamp: the trait asks how long *we*
//! have gone without hearing anything new.
//!
//! Topics live in their own slot rather than inside the record so that
//! `set_topics` never has to read, merge and rewrite a node's info, which would
//! race the transport's own writes to it.
//!
//! ## Queries are scans
//!
//! There is no secondary index. `node_infos_by_topics`, the two counts and both
//! random pickers walk the `node/` key set and filter in memory. An address
//! book holds the nodes one peer has heard of, so the set is small and a scan
//! is cheaper than the writes an index would cost on every insert. A consumer
//! that grows this to where enumeration hurts wants a real index, and that is
//! the change to make rather than tuning around it here.

use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use muniment::{Backend, Codec, StoreError, WriteOp};
use p2panda_core::{Topic, VerifyingKey};
use p2panda_net::addrs::NodeInfo;
use p2panda_store::address_book::{AddressBookStore, NodeInfo as NodeInfoTrait};
use serde::{Deserialize, Serialize};

const NODE_PREFIX: &str = "node/";
const TOPICS_PREFIX: &str = "topics/";

/// A stored node info together with the local time it was last written.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    info: NodeInfo,
    /// Seconds since the Unix epoch, by our own clock.
    updated_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn node_key(id: &VerifyingKey) -> String {
    format!("{NODE_PREFIX}{}", id.to_hex())
}

fn topics_key(id: &VerifyingKey) -> String {
    format!("{TOPICS_PREFIX}{}", id.to_hex())
}

/// An address book for `p2panda-net` backed by a muniment [`Backend`].
///
/// Generic over the backend `B` and the [`Codec`] `C` records are written
/// through, matching [`SlotStore`](muniment::SlotStore). Cheap to clone over a
/// `Clone` backend, which is what handing it to `p2panda_net::AddressBook`
/// requires.
pub struct MunimentAddressBook<B, C> {
    backend: B,
    _codec: PhantomData<fn() -> C>,
}

impl<B: fmt::Debug, C> fmt::Debug for MunimentAddressBook<B, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MunimentAddressBook")
            .field("backend", &self.backend)
            .finish()
    }
}

impl<B: Clone, C> Clone for MunimentAddressBook<B, C> {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            _codec: PhantomData,
        }
    }
}

impl<B, C> MunimentAddressBook<B, C> {
    /// Wrap a backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            _codec: PhantomData,
        }
    }

    /// The backend this address book reads and writes through.
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

// Its own block: `transact` is a defaulted async method, which needs `Sync`.
impl<B: Backend + Sync, C: Codec> MunimentAddressBook<B, C> {
    /// Write `info` only if this node has no record yet, in one transaction.
    ///
    /// A get followed by an insert leaves a gap in which a discovered address
    /// lands and is then overwritten. Inside `transact` there is no gap: a
    /// record written first is kept, and one written after sees ours and
    /// merges into it. Returns whether `info` was written.
    pub async fn insert_node_info_if_absent(&self, info: NodeInfo) -> Result<bool, StoreError> {
        use std::sync::{Arc, Mutex};

        let key = node_key(&info.node_id);
        let bytes = C::encode(&Record {
            updated_at: now_secs(),
            info,
        })?;
        // What the closure saw, read back once the transaction has run.
        let outcome: Arc<Mutex<Result<bool, StoreError>>> = Arc::new(Mutex::new(Ok(false)));
        let seen = Arc::clone(&outcome);
        self.backend
            .transact(Box::new(move |reader| {
                let present = reader.get(&key).map(|bytes| bytes.is_some());
                let write = matches!(present, Ok(false));
                *seen.lock().unwrap() = present.map(|present| !present);
                if write {
                    vec![WriteOp::Put { key, value: bytes }]
                } else {
                    Vec::new()
                }
            }))
            .await?;
        let mut result = outcome.lock().unwrap();
        std::mem::replace(&mut *result, Ok(false))
    }
}

impl<B: Backend, C: Codec> MunimentAddressBook<B, C> {
    async fn read_record(&self, id: &VerifyingKey) -> Result<Option<Record>, StoreError> {
        match self.backend.get(&node_key(id)).await? {
            Some(bytes) => Ok(Some(C::decode(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn write_record(&self, info: NodeInfo) -> Result<(), StoreError> {
        let record = Record {
            updated_at: now_secs(),
            info,
        };
        let bytes = C::encode(&record)?;
        self.backend
            .put(&node_key(&record.info.node_id), &bytes)
            .await
    }

    /// Every stored record, in key order.
    async fn all_records(&self) -> Result<Vec<Record>, StoreError> {
        let keys = self.backend.list(NODE_PREFIX).await?;
        let mut records = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(bytes) = self.backend.get(&key).await? {
                records.push(C::decode::<Record>(&bytes)?);
            }
        }
        Ok(records)
    }

    async fn read_topics(&self, id: &VerifyingKey) -> Result<HashSet<Topic>, StoreError> {
        match self.backend.get(&topics_key(id)).await? {
            Some(bytes) => Ok(C::decode::<Vec<Topic>>(&bytes)?.into_iter().collect()),
            None => Ok(HashSet::new()),
        }
    }

    /// Picks the first record satisfying `predicate`, after rotating the set by
    /// a value derived from the clock.
    ///
    /// `list` returns keys in a stable order, so taking the first match would
    /// hand back the same node forever and defeat the random walk this feeds.
    async fn pick<F>(&self, predicate: F) -> Result<Option<NodeInfo>, StoreError>
    where
        F: Fn(&NodeInfo) -> bool,
    {
        let candidates: Vec<NodeInfo> = self
            .all_records()
            .await?
            .into_iter()
            .map(|record| record.info)
            .filter(|info| predicate(info))
            .collect();

        if candidates.is_empty() {
            return Ok(None);
        }

        let offset = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos() as usize)
            .unwrap_or(0);
        Ok(Some(candidates[offset % candidates.len()].clone()))
    }
}

impl<B: Backend, C: Codec> AddressBookStore<VerifyingKey, NodeInfo> for MunimentAddressBook<B, C> {
    type Error = StoreError;

    async fn insert_node_info(&self, info: NodeInfo) -> Result<bool, Self::Error> {
        let is_new = self.backend.get(&node_key(&info.node_id)).await?.is_none();
        self.write_record(info).await?;
        Ok(is_new)
    }

    async fn remove_node_info(&self, id: &VerifyingKey) -> Result<bool, Self::Error> {
        if self.backend.get(&node_key(id)).await?.is_none() {
            return Ok(false);
        }
        // The node and its topics go together, or neither does.
        self.backend
            .apply(&[
                WriteOp::Delete { key: node_key(id) },
                WriteOp::Delete {
                    key: topics_key(id),
                },
            ])
            .await?;
        Ok(true)
    }

    async fn remove_older_than(&self, duration: Duration) -> Result<usize, Self::Error> {
        let cutoff = now_secs().saturating_sub(duration.as_secs());
        let stale: Vec<VerifyingKey> = self
            .all_records()
            .await?
            .into_iter()
            .filter(|record| record.updated_at < cutoff)
            .map(|record| record.info.node_id)
            .collect();

        if stale.is_empty() {
            return Ok(0);
        }

        let ops: Vec<WriteOp> = stale
            .iter()
            .flat_map(|id| {
                [
                    WriteOp::Delete { key: node_key(id) },
                    WriteOp::Delete {
                        key: topics_key(id),
                    },
                ]
            })
            .collect();
        self.backend.apply(&ops).await?;
        Ok(stale.len())
    }

    async fn node_info(&self, id: &VerifyingKey) -> Result<Option<NodeInfo>, Self::Error> {
        Ok(self.read_record(id).await?.map(|record| record.info))
    }

    async fn node_topics(&self, id: &VerifyingKey) -> Result<HashSet<Topic>, Self::Error> {
        self.read_topics(id).await
    }

    async fn all_node_infos(&self) -> Result<Vec<NodeInfo>, Self::Error> {
        Ok(self
            .all_records()
            .await?
            .into_iter()
            .map(|record| record.info)
            .collect())
    }

    async fn all_nodes_len(&self) -> Result<usize, Self::Error> {
        Ok(self.backend.list(NODE_PREFIX).await?.len())
    }

    async fn all_bootstrap_nodes_len(&self) -> Result<usize, Self::Error> {
        Ok(self
            .all_records()
            .await?
            .iter()
            .filter(|record| record.info.is_bootstrap())
            .count())
    }

    async fn selected_node_infos(
        &self,
        ids: &[VerifyingKey],
    ) -> Result<Vec<NodeInfo>, Self::Error> {
        let mut infos = Vec::new();
        for id in ids {
            if let Some(record) = self.read_record(id).await? {
                infos.push(record.info);
            }
        }
        Ok(infos)
    }

    async fn set_topics(
        &self,
        id: VerifyingKey,
        topics: HashSet<Topic>,
    ) -> Result<(), Self::Error> {
        // Overwrite rather than extend: the trait asks implementers to replace
        // the previous set, because topics arrive as a whole statement.
        let topics: Vec<Topic> = topics.into_iter().collect();
        if topics.is_empty() {
            self.backend.delete(&topics_key(&id)).await
        } else {
            self.backend
                .put(&topics_key(&id), &C::encode(&topics)?)
                .await
        }
    }

    async fn node_infos_by_topics(&self, topics: &[Topic]) -> Result<Vec<NodeInfo>, Self::Error> {
        let wanted: HashSet<&Topic> = topics.iter().collect();
        let mut infos = Vec::new();
        for record in self.all_records().await? {
            let held = self.read_topics(&record.info.node_id).await?;
            if held.iter().any(|topic| wanted.contains(topic)) {
                infos.push(record.info);
            }
        }
        Ok(infos)
    }

    async fn random_node(&self) -> Result<Option<NodeInfo>, Self::Error> {
        self.pick(|info| !info.is_stale()).await
    }

    async fn random_bootstrap_node(&self) -> Result<Option<NodeInfo>, Self::Error> {
        self.pick(|info| info.is_bootstrap()).await
    }
}

#[cfg(test)]
mod tests;
