// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The release-log session: a one-line request, then catch-up sync.
//!
//! `TopicLogSync` does not exchange its topic, so a holder that serves many
//! logs on one protocol learns which log is wanted from a 34-byte request
//! (magic, version, log key) and answers with one byte: held or not. Only a
//! holder that holds the log goes on to sync. The request is advice, not
//! authority; the reader's own policy decides what it keeps.

use std::future::ready;
use std::time::Duration;

use muniment::{Backend, MemoryBackend};
use murm_sync::{
    StreamSession, StreamSync, StreamSyncError, StreamSyncLimits, StreamSyncOutcome,
    processor_accept,
};
use stickleback::edition::{
    EditionChain, EditionError, EditionExt, EditionLimits, EditionOperation, EditionPolicy,
    VerifyingKey, edition_topic, verify_edition_chain,
};
use stickleback::{MunimentStore, OperationProcessor};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use transport::Alpn;

/// The protocol every carrier serves release logs under.
pub const RELEASE_PROTOCOL: &str = "mere/release/v1";
/// The message a holder's refusal to serve carries.
pub const NOT_HELD: &str = "holder does not hold the log";

const MAGIC: [u8; 2] = [b'R', 1];

/// The generic release ALPN.
pub fn release_alpn() -> Alpn {
    Alpn::new(RELEASE_PROTOCOL)
}

/// An ALPN whose name carries the log key, for carriers that discover holders
/// by announced name (Reticulum). `mere/release/v1/<hex>` maps to the
/// destination name `mere.release.v1.<hex>`.
pub fn key_alpn(key: &[u8; 32]) -> Alpn {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    Alpn::new(&format!("{RELEASE_PROTOCOL}/{hex}"))
}

/// A store holding edition logs, as a holder or a reader keeps them.
pub type EditionStore<B = MemoryBackend> = MunimentStore<B, EditionExt>;
pub(crate) type Processor<B = MemoryBackend> = OperationProcessor<B, EditionExt, EditionPolicy>;

fn protocol(what: impl ToString) -> StreamSyncError {
    StreamSyncError::Protocol(what.to_string())
}

async fn bounded<T>(
    deadline: Duration,
    work: impl Future<Output = std::io::Result<T>>,
) -> Result<T, StreamSyncError> {
    match tokio::time::timeout(deadline, work).await {
        Ok(done) => done.map_err(protocol),
        Err(_) => Err(StreamSyncError::Deadline),
    }
}

/// The reader's side: ask for `key`, then pull what the holder sends through
/// the reader's policy, which admits only that key's editions.
pub struct ReadSession {
    pub(crate) key: [u8; 32],
    pub(crate) store: EditionStore,
    pub(crate) processor: Processor,
    pub(crate) limits: StreamSyncLimits,
}

impl StreamSession for ReadSession {
    async fn run<St>(self, mut stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = self.limits.deadline();
        let mut request = [0u8; 34];
        request[..2].copy_from_slice(&MAGIC);
        request[2..].copy_from_slice(&self.key);
        bounded(deadline, async {
            stream.write_all(&request).await?;
            stream.flush().await
        })
        .await?;
        let mut reply = [0u8; 1];
        bounded(deadline, stream.read_exact(&mut reply)).await?;
        if reply[0] != 1 {
            return Err(protocol(NOT_HELD));
        }
        StreamSync::<_, u64, EditionExt, _>::new(
            edition_topic(&self.key),
            self.store,
            self.limits,
            processor_accept(self.processor),
        )
        .run(stream)
        .await
    }
}

/// A peer that serves the edition logs it holds and takes nothing in.
pub struct Holder<B = MemoryBackend> {
    store: EditionStore<B>,
    processor: Processor<B>,
    limits: StreamSyncLimits,
}

impl<B: Backend + Clone + Send + Sync + 'static> Holder<B> {
    /// A holder over `backend`, admitting any key's editions through
    /// [`hold`](Self::hold).
    pub fn new(backend: B) -> Self {
        let store = EditionStore::new(backend);
        Self {
            processor: OperationProcessor::new(store.clone(), EditionPolicy::default()),
            store,
            limits: StreamSyncLimits::default(),
        }
    }

    /// Hold verified editions, oldest first. Returns how many were new.
    pub async fn hold(&self, editions: &[stickleback::edition::EditionOperation]) -> usize {
        let mut stored = 0;
        for edition in editions {
            let kept = self.processor.process(edition).await;
            stored += usize::from(kept.is_ok_and(|outcome| outcome.inserted()));
        }
        stored
    }

    /// The verified chain held for `key`, or `None` when nothing is held.
    ///
    /// # Errors
    /// The held operations do not verify as one log under the key.
    pub async fn chain(&self, key: &[u8; 32]) -> Result<Option<EditionChain>, EditionError> {
        let Ok(author) = VerifyingKey::from_bytes(key) else {
            return Ok(None);
        };
        let held = self
            .store
            .get_log_entries(&author, &0u64, None, None)
            .await
            .map_err(|error| EditionError::Invalid(error.to_string()))?;
        let Some(held) = held else { return Ok(None) };
        let operations: Vec<EditionOperation> = held.into_iter().map(|(op, _)| op).collect();
        verify_edition_chain(key, &operations, EditionLimits::default()).map(Some)
    }

    /// Drop every held edition of `key`. Returns how many were removed.
    pub async fn forget(&self, key: &[u8; 32]) -> usize {
        let Ok(Some(held)) = self.chain(key).await else {
            return 0;
        };
        let mut removed = 0;
        for operation in held.operations() {
            removed += usize::from(
                self.store
                    .delete_operation(&operation.hash)
                    .await
                    .unwrap_or(false),
            );
        }
        removed
    }

    /// The session to hand a lane for one accepted stream.
    pub fn session(&self) -> HolderSession<B> {
        HolderSession {
            store: self.store.clone(),
            limits: self.limits,
        }
    }
}

/// The holder's side of one stream.
pub struct HolderSession<B = MemoryBackend> {
    store: EditionStore<B>,
    limits: StreamSyncLimits,
}

impl<B: Backend + Clone + Send + Sync + 'static> StreamSession for HolderSession<B> {
    async fn run<St>(self, mut stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = self.limits.deadline();
        let mut request = [0u8; 34];
        bounded(deadline, stream.read_exact(&mut request)).await?;
        if request[..2] != MAGIC {
            return Err(protocol("not a release-log request"));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&request[2..]);
        self.serve_key(stream, key, true).await
    }
}

impl<B: Backend + Clone + Send + Sync + 'static> HolderSession<B> {
    /// Answer a request for `key` already read off the stream: reply held or
    /// not, then sync. `allowed` false answers not held whatever the store has,
    /// so a holder that serves only logs it agreed to hold can gate here.
    ///
    /// # Errors
    /// The stream failed or the sync did not finish.
    pub async fn serve_key<St>(
        self,
        mut stream: St,
        key: [u8; 32],
        allowed: bool,
    ) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = self.limits.deadline();
        let held = allowed
            && match VerifyingKey::from_bytes(&key) {
                Ok(author) => self
                    .store
                    .get_latest_entry(&author, &0u64)
                    .await
                    .is_ok_and(|latest| latest.is_some()),
                Err(_) => false,
            };
        bounded(deadline, async {
            stream.write_all(&[u8::from(held)]).await?;
            stream.flush().await
        })
        .await?;
        if !held {
            return Ok(StreamSyncOutcome::default());
        }
        StreamSync::<_, u64, EditionExt, _>::new(
            edition_topic(&key),
            self.store,
            self.limits,
            |_operation| ready(false),
        )
        .run(stream)
        .await
    }
}
