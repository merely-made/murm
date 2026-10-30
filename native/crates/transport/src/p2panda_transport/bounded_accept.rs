// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio::time::Instant;

use super::{
    AcceptError, Connection, P2pandaStream, P2pandaTransport, ProtocolHandler, QueuedStream,
};
use crate::{AcceptedSession, Alpn, IngressContext, PeerID, TransportError};

pub(super) enum AcceptReceiver {
    Unbounded(mpsc::UnboundedReceiver<QueuedStream>),
    Bounded(mpsc::Receiver<QueuedStream>),
}

impl AcceptReceiver {
    pub(super) async fn recv(&mut self) -> Option<QueuedStream> {
        match self {
            Self::Unbounded(rx) => rx.recv().await,
            Self::Bounded(rx) => rx.recv().await,
        }
    }
}

/// Observed pending/queued capacity for one opt-in ALPN. Pending includes peers
/// which have not opened their first stream; both phases share the same permits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundedAcceptStats {
    /// Configured pending plus queued session ceiling.
    pub limit: usize,
    /// Current pending or queued sessions, excluding dequeued service work.
    pub pending: usize,
    /// Highest observed pending count.
    pub peak_pending: usize,
    /// Connections refused because all queue reservations were held.
    pub refused: usize,
}

#[derive(Debug)]
struct State {
    permits: Arc<Semaphore>,
    peak: AtomicUsize,
    refused: AtomicUsize,
    limit: usize,
}

/// A read-only observation handle. Closing a route remains a transport-owner action.
#[derive(Clone, Debug)]
pub struct BoundedAcceptHandle {
    state: Arc<State>,
}

impl BoundedAcceptHandle {
    /// Read a momentary queue observation.
    pub fn stats(&self) -> BoundedAcceptStats {
        BoundedAcceptStats {
            limit: self.state.limit,
            pending: self.state.limit - self.state.permits.available_permits(),
            peak_pending: self.state.peak.load(Ordering::Relaxed),
            refused: self.state.refused.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug)]
struct Handler {
    tx: mpsc::Sender<QueuedStream>,
    state: Arc<State>,
    first_stream_timeout: Duration,
    #[cfg(feature = "noise")]
    local_peer: crate::PeerID,
    #[cfg(feature = "noise")]
    service_protocol: Alpn,
}

impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        if self.tx.is_closed() {
            connection.close(0u32.into(), b"service stopped");
            return Ok(());
        }
        let Ok(permit) = self.state.permits.clone().try_acquire_owned() else {
            self.state.refused.fetch_add(1, Ordering::Relaxed);
            connection.close(0u32.into(), b"service queue full");
            return Ok(());
        };
        self.state.peak.fetch_max(
            self.state.limit - self.state.permits.available_permits(),
            Ordering::Relaxed,
        );
        let arrived_at = Instant::now();
        let peer = PeerID::from_bytes(connection.remote_id().as_bytes()).ok();
        let pair = tokio::select! {
            _ = self.tx.closed() => { connection.close(0u32.into(), b"service stopped"); return Ok(()); }
            pair = tokio::time::timeout(self.first_stream_timeout, connection.accept_bi()) => pair,
        };
        let (send, recv) = match pair {
            Ok(Ok(pair)) => pair,
            _ => {
                connection.close(0u32.into(), b"first stream deadline");
                return Ok(());
            },
        };
        let queued = QueuedStream {
            peer,
            arrived_at,
            _queue_permit: Some(permit),
            stream: P2pandaStream {
                #[cfg(feature = "noise")]
                local_peer: self.local_peer,
                #[cfg(feature = "noise")]
                service_protocol: self.service_protocol.clone(),
                send,
                recv,
                _connection: connection,
                shutdown_ack: None,
                shutdown_finished: false,
                shutdown_complete: false,
            },
        };
        if let Err(error) = self.tx.try_send(queued) {
            error
                .into_inner()
                .stream
                ._connection
                .close(0u32.into(), b"service stopped");
        }
        Ok(())
    }
}

impl P2pandaTransport {
    /// Register a bounded opt-in protocol on the existing endpoint. This does not
    /// change other routes. Capacity is reserved before first-stream waiting and
    /// retained until dequeue; full routes close new connections immediately.
    pub async fn register_bounded_alpn(
        &self,
        alpn: Alpn,
        capacity: usize,
        first_stream_timeout: Duration,
    ) -> Result<BoundedAcceptHandle, TransportError> {
        if capacity == 0
            || capacity > 64
            || first_stream_timeout.is_zero()
            || first_stream_timeout > Duration::from_secs(60)
        {
            return Err(TransportError::Backend(
                "invalid bounded accept profile".into(),
            ));
        }
        let state = Arc::new(State {
            permits: Arc::new(Semaphore::new(capacity)),
            limit: capacity,
            peak: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
        });
        let (tx, rx) = mpsc::channel(capacity);
        {
            let mut queues = self.queues.lock().unwrap();
            if queues.contains_key(&alpn) {
                return Err(TransportError::Backend("ALPN already registered".into()));
            }
            queues.insert(
                alpn.clone(),
                Arc::new(Mutex::new(AcceptReceiver::Bounded(rx))),
            );
        }
        if let Err(error) = self
            .endpoint
            .accept(
                alpn.as_bytes(),
                Handler {
                    tx,
                    state: state.clone(),
                    first_stream_timeout,
                    #[cfg(feature = "noise")]
                    local_peer: self.peer_id,
                    #[cfg(feature = "noise")]
                    service_protocol: alpn.clone(),
                },
            )
            .await
        {
            self.queues.lock().unwrap().remove(&alpn);
            return Err(TransportError::Backend(format!(
                "bounded ALPN registration: {error}"
            )));
        }
        Ok(BoundedAcceptHandle { state })
    }

    /// Dequeue a stream together with its carrier-observed arrival time. The
    /// service uses this time for one deadline covering queue wait and all I/O.
    pub async fn accept_observed(
        &self,
        alpn: Alpn,
    ) -> Result<(AcceptedSession<P2pandaStream>, Instant), TransportError> {
        let rx = self
            .queues
            .lock()
            .unwrap()
            .get(&alpn)
            .cloned()
            .ok_or(TransportError::AlpnNotRegistered)?;
        let queued = rx.lock().await.recv().await.ok_or(TransportError::Closed)?;
        let arrived_at = queued.arrived_at;
        let session =
            AcceptedSession::new(queued.stream, alpn, queued.peer, IngressContext::p2panda());
        Ok((session, arrived_at))
    }

    /// Stop accepting a custom protocol and release queued streams. Its registered
    /// handler observes the closed channel and refuses further connections.
    pub fn stop_accepting(&self, alpn: &Alpn) {
        self.queues.lock().unwrap().remove(alpn);
    }
}
