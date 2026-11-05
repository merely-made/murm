// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Catch-up sync over any ordered byte stream.
//!
//! [`StreamSync`] runs p2panda-sync's `TopicLogSync` protocol, the same one
//! p2panda-net drives for [`JoinedSpace`](crate::JoinedSpace), over anything
//! that is `AsyncRead + AsyncWrite`: a memory duplex, a Reticulum link, a
//! Noise-secured TCP socket. Catch-up only, no live mode and no gossip. The
//! protocol is symmetric, so one call serves both the dialer and the acceptor.
//!
//! The caller owns carriage and security. Pass a stream that has already been
//! secured and admitted; this module opens nothing and trusts nothing it
//! reads. Both ends must name the same topic, which is agreed out of band (the
//! protocol id or address that selected the stream), not exchanged here.
//!
//! ## Bounds
//!
//! Frames carry their own ceiling (a header, a body and the log heights fit
//! inside one) and the session has a read budget and one deadline. An operation
//! larger than the frame ceiling cannot be carried and fails the session; large
//! content belongs on resource carriage, not in an operation body.
//!
//! ## Ingestion
//!
//! Each received operation goes to the injected `accept` closure, exactly as
//! with [`SyncedSpace::drive`](crate::SyncedSpace::drive): the closure verifies
//! and stores, and returns whether the operation counted. The protocol itself
//! checks nothing about an operation beyond decoding its header. Use
//! [`processor_accept`] to route through a stickleback `OperationProcessor`,
//! which verifies the signature and body hash and then asks the domain policy.

use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::time::Duration;

use futures_util::StreamExt;
use muniment::Backend;
use p2panda_core::{AnyOperation, Extensions, Hash, LogId, Operation, SeqNum, Topic, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::topics::TopicStore;
use p2panda_sync::protocols::{TopicLogSync, TopicLogSyncEvent, TopicLogSyncMessage};
use p2panda_sync::traits::Protocol;
use stickleback::{OperationPolicy, OperationProcessor};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use tokio_util::codec::Framed;

use crate::stream_codec::{CborCodec, Fault, FaultKind};

/// Hard ceiling for one frame, whatever the caller configures.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;
/// Hard ceiling for one session's read budget.
pub const MAX_SESSION_BYTES: u64 = 1 << 32;
/// Hard ceiling for one session's deadline.
pub const MAX_DEADLINE: Duration = Duration::from_secs(3600);
/// Events the drain may fall behind by before it loses operations.
const EVENT_BACKLOG: usize = 4096;

/// Resource bounds for one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSyncLimits {
    max_frame_bytes: u32,
    max_session_bytes: u64,
    deadline: Duration,
}

impl StreamSyncLimits {
    /// Choose nonzero bounds within the hard ceilings.
    pub fn new(
        max_frame_bytes: u32,
        max_session_bytes: u64,
        deadline: Duration,
    ) -> Result<Self, StreamSyncError> {
        if max_frame_bytes == 0 || max_frame_bytes > MAX_FRAME_BYTES {
            return Err(StreamSyncError::Limits("frame ceiling out of range"));
        }
        if max_session_bytes < u64::from(max_frame_bytes) || max_session_bytes > MAX_SESSION_BYTES {
            return Err(StreamSyncError::Limits("session budget out of range"));
        }
        if deadline.is_zero() || deadline > MAX_DEADLINE {
            return Err(StreamSyncError::Limits("deadline out of range"));
        }
        Ok(Self {
            max_frame_bytes,
            max_session_bytes,
            deadline,
        })
    }

    /// Largest accepted frame body, excluding its four-byte prefix.
    pub fn max_frame_bytes(&self) -> u32 {
        self.max_frame_bytes
    }

    /// Most bytes one session may read, prefixes included.
    pub fn max_session_bytes(&self) -> u64 {
        self.max_session_bytes
    }

    /// One deadline for the whole session.
    pub fn deadline(&self) -> Duration {
        self.deadline
    }
}

impl Default for StreamSyncLimits {
    /// 1 MiB frames, a 64 MiB read budget and one minute.
    fn default() -> Self {
        Self {
            max_frame_bytes: 1024 * 1024,
            max_session_bytes: 64 * 1024 * 1024,
            deadline: Duration::from_secs(60),
        }
    }
}

/// How a session ended when it did not fail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamSyncOutcome {
    /// Operations the remote sent that decoded.
    pub received: u64,
    /// Operations `accept` counted: verified and new.
    pub accepted: u64,
    /// Operations `accept` declined: forged, out of policy, stale or repeated.
    pub declined: u64,
    /// Events the drain missed because `accept` fell too far behind. Nonzero
    /// means operations were dropped and the pair should sync again.
    pub lagged: u64,
}

/// Why a session failed.
#[derive(Debug, thiserror::Error)]
pub enum StreamSyncError {
    /// The configured limits were out of range.
    #[error("stream sync limits: {0}")]
    Limits(&'static str),
    /// The session deadline expired.
    #[error("stream sync deadline expired")]
    Deadline,
    /// A frame exceeded the ceiling, in either direction.
    #[error("stream sync frame over the ceiling")]
    FrameTooLarge,
    /// The remote sent more than the session's read budget.
    #[error("stream sync read budget exceeded")]
    BudgetExceeded,
    /// The protocol or the stream failed.
    #[error("stream sync protocol: {0}")]
    Protocol(String),
}

/// Something that can run one sync session over a stream the caller secured.
///
/// Carrier lanes take this instead of naming [`StreamSync`]'s type parameters.
pub trait StreamSession {
    /// Run the session to its end.
    fn run<St>(
        self,
        stream: St,
    ) -> impl Future<Output = Result<StreamSyncOutcome, StreamSyncError>>
    where
        St: AsyncRead + AsyncWrite + Unpin;
}

/// One catch-up session over a topic's logs in `store`.
pub struct StreamSync<S, L, E, A> {
    topic: Topic,
    store: S,
    limits: StreamSyncLimits,
    accept: A,
    _types: PhantomData<fn() -> (L, E)>,
}

impl<S, L, E, A, Fut> StreamSync<S, L, E, A>
where
    A: FnMut(Operation<E>) -> Fut,
    Fut: Future<Output = bool>,
{
    /// Prepare a session. `L` and `E` cannot be inferred from a store that
    /// serves every log id, so a call site pins them:
    /// `StreamSync::<_, u64, (), _>::new(..)`.
    pub fn new(topic: impl Into<Topic>, store: S, limits: StreamSyncLimits, accept: A) -> Self {
        Self {
            topic: topic.into(),
            store,
            limits,
            accept,
            _types: PhantomData,
        }
    }
}

impl<S, L, E, A, Fut> StreamSession for StreamSync<S, L, E, A>
where
    S: LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash>
        + TopicStore<Topic, VerifyingKey, L>
        + Clone
        + Send
        + 'static,
    L: LogId + Debug + Send + 'static,
    E: Extensions + Clone + Send + 'static,
    A: FnMut(Operation<E>) -> Fut,
    Fut: Future<Output = bool>,
{
    async fn run<St>(mut self, stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let limits = self.limits;
        let fault = Fault::default();
        let codec = CborCodec::<TopicLogSyncMessage<L>>::new(
            limits.max_frame_bytes,
            limits.max_session_bytes,
            fault.clone(),
        );
        let (mut sink, mut messages) = Framed::new(stream, codec).split();
        let (event_tx, mut events) = broadcast::channel(EVENT_BACKLOG);
        let protocol = TopicLogSync::<Topic, S, L, E>::new(
            self.topic,
            self.store.clone(),
            None,
            event_tx,
        );

        let run = protocol.run(&mut sink, &mut messages);
        let accept = &mut self.accept;
        let drain = async {
            let mut outcome = StreamSyncOutcome::default();
            loop {
                match events.recv().await {
                    Ok(TopicLogSyncEvent::OperationReceived { operation, .. }) => {
                        outcome.received += 1;
                        if accept(*operation).await {
                            outcome.accepted += 1;
                        } else {
                            outcome.declined += 1;
                        }
                    },
                    Ok(_) => {},
                    Err(broadcast::error::RecvError::Lagged(missed)) => outcome.lagged += missed,
                    Err(broadcast::error::RecvError::Closed) => break outcome,
                }
            }
        };

        let (result, outcome) =
            tokio::time::timeout(limits.deadline, async { tokio::join!(run, drain) })
                .await
                .map_err(|_| StreamSyncError::Deadline)?;

        match (result, fault.get()) {
            (Ok(()), _) => Ok(outcome),
            (Err(_), Some(FaultKind::Oversize)) => Err(StreamSyncError::FrameTooLarge),
            (Err(_), Some(FaultKind::Budget)) => Err(StreamSyncError::BudgetExceeded),
            (Err(error), None) => Err(StreamSyncError::Protocol(error.to_string())),
        }
    }
}

/// An `accept` closure that stores through a stickleback processor.
///
/// The processor verifies each operation's signature and body hash, asks the
/// domain policy, checks log continuity and then inserts. It counts only an
/// operation that was new and stored; refusals and repeats are declined.
pub fn processor_accept<B, E, P>(
    processor: OperationProcessor<B, E, P>,
) -> impl FnMut(Operation<E>) -> std::pin::Pin<Box<dyn Future<Output = bool>>>
where
    B: Backend + Clone + 'static,
    E: Extensions + 'static,
    P: OperationPolicy<E> + Clone + 'static,
{
    move |operation| {
        let processor = processor.clone();
        Box::pin(async move {
            processor
                .process(&operation)
                .await
                .is_ok_and(|outcome| outcome.inserted())
        })
    }
}
