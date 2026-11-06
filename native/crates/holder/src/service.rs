// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The holder's side of one stream: serve a log, or acknowledge custody.
//!
//! Two requests share the release ALPN, told apart by their first byte. `R`
//! is the resolver's read request. `A` is an acknowledgement request: log key
//! and a 32-byte challenge. The reply to `A` is one byte, then the signed
//! statement when the holder agreed to hold the log and has verified editions.

use std::time::Duration;

use identity::Ed25519Keypair;
use muniment::Backend;
use murm_resolve::Holder;
use murm_sync::{StreamSession, StreamSyncError, StreamSyncLimits, StreamSyncOutcome};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::ack::Ack;
use crate::node::{Agreements, HolderNode, lock};

const VERSION: u8 = 1;
pub(crate) const ACK_REQUEST: u8 = b'A';
const READ_REQUEST: u8 = b'R';

fn protocol(what: impl ToString) -> StreamSyncError {
    StreamSyncError::Protocol(what.to_string())
}

pub(crate) async fn bounded<T>(
    deadline: Duration,
    work: impl Future<Output = std::io::Result<T>>,
) -> Result<T, StreamSyncError> {
    match tokio::time::timeout(deadline, work).await {
        Ok(done) => done.map_err(protocol),
        Err(_) => Err(StreamSyncError::Deadline),
    }
}

/// One accepted stream's worth of holder service.
pub struct HolderService<B> {
    holder: Arc<Holder<B>>,
    agreements: Agreements,
    signer: Arc<Ed25519Keypair>,
}

impl<B> HolderService<B> {
    pub(crate) fn new(node: &HolderNode<B>) -> Self
    where
        B: Backend + Clone + Send + Sync + 'static,
    {
        Self {
            holder: node.holder.clone(),
            agreements: node.agreements.clone(),
            signer: node.signer.clone(),
        }
    }
}

impl<B: Backend + Clone + Send + Sync + 'static> StreamSession for HolderService<B> {
    async fn run<St>(self, mut stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = StreamSyncLimits::default().deadline();
        let mut head = [0u8; 2];
        bounded(deadline, stream.read_exact(&mut head)).await?;
        if head[1] != VERSION {
            return Err(protocol("unsupported holder request version"));
        }
        let mut key = [0u8; 32];
        bounded(deadline, stream.read_exact(&mut key)).await?;
        let agreed = lock(&self.agreements).contains_key(&key);
        match head[0] {
            READ_REQUEST => self.holder.session().serve_key(stream, key, agreed).await,
            ACK_REQUEST => {
                let mut challenge = [0u8; 32];
                bounded(deadline, stream.read_exact(&mut challenge)).await?;
                let statement = self.statement(&key, agreed, challenge).await;
                bounded(deadline, async {
                    match statement {
                        Some(ack) => {
                            stream.write_all(&[1]).await?;
                            stream.write_all(&ack.encode()).await?;
                        },
                        None => stream.write_all(&[0]).await?,
                    }
                    stream.flush().await
                })
                .await?;
                Ok(StreamSyncOutcome::default())
            },
            _ => Err(protocol("not a holder request")),
        }
    }
}

impl<B: Backend + Clone + Send + Sync + 'static> HolderService<B> {
    async fn statement(&self, key: &[u8; 32], agreed: bool, challenge: [u8; 32]) -> Option<Ack> {
        if !agreed {
            return None;
        }
        let chain = self.holder.chain(key).await.ok()??;
        let head = chain.head();
        Some(Ack::sign(
            &self.signer,
            *key,
            head.id.0,
            head.seq_num,
            challenge,
        ))
    }
}
