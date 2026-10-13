// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! In-process [`Transport`] implementation for tests.
//!
//! [`MemoryTransport`] connects two simulated peers via tokio
//! [`DuplexStream`]s, with per-ALPN channels lazily created. It's the
//! test fixture for protocol-level integration tests in `murm`,
//! `murm`, and elsewhere — exercise full Cable (or future protocol)
//! flows between two peers without bringing up real iroh / network I/O.
//!
//! ## Status
//!
//! Test-fixture quality, not production. Optimized for clarity and test
//! ergonomics, not performance or pairwise-scale correctness. Specifically:
//!
//! - Only **paired** peers can talk: `MemoryTransport::pair(node_a, node_b)`
//!   yields two transports that connect to each other; arbitrary
//!   3+-peer meshes are out of scope.
//! - Buffer size for each `DuplexStream` is 64 KiB. Large messages may
//!   hit backpressure — fine for tests, not modeled for production.
//! - No simulated network failures, latency, or reordering.
//!
//! ## Quick start
//!
//! ```ignore
//! use transport::{Alpn, PeerID, Transport};
//! use transport::memory::MemoryTransport;
//! use identity::{IdentityProvider, InMemoryProvider};
//!
//! let alice_id = PeerID::from_public_key(
//!     InMemoryProvider::from_seed([1; 32]).master_public_key(),
//! );
//! let bob_id = PeerID::from_public_key(
//!     InMemoryProvider::from_seed([2; 32]).master_public_key(),
//! );
//!
//! let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);
//! let alpn = Alpn::new("mere/cable/v1");
//!
//! let connect_fut = alice.connect(bob_id, alpn.clone());
//! let accept_fut = bob.accept(alpn);
//! let (alice_stream, bob_stream) =
//!     tokio::join!(connect_fut, accept_fut);
//! // both streams are open and connected
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{DuplexStream, duplex};
use tokio::sync::{Mutex as TokioMutex, mpsc};

use crate::{AcceptedSession, Alpn, IngressContext, PeerID, Transport, TransportError};

/// Default duplex buffer size (64 KiB).
const DUPLEX_BUF: usize = 64 * 1024;

/// One mpsc channel per (transport, ALPN). Both ends of the pair can hold
/// `Sender` clones; the `Receiver` is wrapped in [`TokioMutex`] so a single
/// active `accept()` call has exclusive access.
struct PerAlpnChannel {
    tx: mpsc::UnboundedSender<DuplexStream>,
    rx: Arc<TokioMutex<mpsc::UnboundedReceiver<DuplexStream>>>,
}

impl Clone for PerAlpnChannel {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: Arc::clone(&self.rx),
        }
    }
}

type ChannelMap = Arc<Mutex<HashMap<Alpn, PerAlpnChannel>>>;

fn get_or_create_channel(map: &ChannelMap, alpn: &Alpn) -> PerAlpnChannel {
    let mut guard = map.lock().unwrap();
    guard
        .entry(alpn.clone())
        .or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            PerAlpnChannel {
                tx,
                rx: Arc::new(TokioMutex::new(rx)),
            }
        })
        .clone()
}

/// Test-fixture in-process [`Transport`] connecting two paired peers.
///
/// Construct with [`MemoryTransport::pair`].
pub struct MemoryTransport {
    peer_id: PeerID,
    peer_node: PeerID,
    /// Streams *arriving at this transport* (used by `accept`).
    incoming: ChannelMap,
    /// Streams *being sent to the peer* (used by `connect`). The `Arc` is
    /// cloned from the peer's `incoming` map.
    peer_incoming: ChannelMap,
}

impl MemoryTransport {
    /// Create two paired transports that can connect to each other.
    ///
    /// `node_a` and `node_b` should differ — connecting from `a` to `a`'s
    /// own `peer_id` is rejected with [`TransportError::ConnectionRefused`].
    pub fn pair(node_a: PeerID, node_b: PeerID) -> (MemoryTransport, MemoryTransport) {
        let a_incoming: ChannelMap = Arc::new(Mutex::new(HashMap::new()));
        let b_incoming: ChannelMap = Arc::new(Mutex::new(HashMap::new()));

        let a = MemoryTransport {
            peer_id: node_a,
            peer_node: node_b,
            incoming: Arc::clone(&a_incoming),
            peer_incoming: Arc::clone(&b_incoming),
        };
        let b = MemoryTransport {
            peer_id: node_b,
            peer_node: node_a,
            incoming: b_incoming,
            peer_incoming: a_incoming,
        };
        (a, b)
    }
}

impl Transport for MemoryTransport {
    type Stream = DuplexStream;

    fn local_peer_id(&self) -> PeerID {
        self.peer_id
    }

    async fn connect(&self, peer: PeerID, alpn: Alpn) -> Result<DuplexStream, TransportError> {
        if peer != self.peer_node {
            return Err(TransportError::ConnectionRefused);
        }
        let (peer_end, my_end) = duplex(DUPLEX_BUF);
        let channel = get_or_create_channel(&self.peer_incoming, &alpn);
        channel
            .tx
            .send(peer_end)
            .map_err(|_| TransportError::Closed)?;
        Ok(my_end)
    }

    async fn accept(&self, alpn: Alpn) -> Result<AcceptedSession<DuplexStream>, TransportError> {
        let channel = get_or_create_channel(&self.incoming, &alpn);
        let mut rx = channel.rx.lock().await;
        let stream = rx.recv().await.ok_or(TransportError::Closed)?;
        // The fixture is a *pair*, so the counterparty is known by
        // construction — this is knowledge the fixture genuinely has, not a
        // claim read off the wire.
        Ok(AcceptedSession::new(
            stream,
            alpn,
            Some(self.peer_node),
            IngressContext::memory(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use identity::{IdentityProvider, InMemoryProvider};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn alice_bob_ids() -> (PeerID, PeerID) {
        let a = PeerID::from_public_key(InMemoryProvider::from_seed([1; 32]).master_public_key());
        let b = PeerID::from_public_key(InMemoryProvider::from_seed([2; 32]).master_public_key());
        (a, b)
    }

    #[tokio::test]
    async fn pair_local_peer_ids_match_construction() {
        let (alice_id, bob_id) = alice_bob_ids();
        let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);
        assert_eq!(alice.local_peer_id(), alice_id);
        assert_eq!(bob.local_peer_id(), bob_id);
    }

    #[tokio::test]
    async fn connect_then_accept_yields_paired_streams() {
        let (alice_id, bob_id) = alice_bob_ids();
        let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);
        let alpn = Alpn::new("mere/test/v1");

        let connect_fut = alice.connect(bob_id, alpn.clone());
        let accept_fut = bob.accept(alpn);

        let (alice_res, bob_res) = tokio::join!(connect_fut, accept_fut);
        let _alice_stream = alice_res.expect("connect failed");
        let _bob_stream = bob_res.expect("accept failed");
    }

    #[tokio::test]
    async fn bytes_round_trip_through_paired_streams() {
        let (alice_id, bob_id) = alice_bob_ids();
        let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);
        let alpn = Alpn::new("mere/test/v1");

        let connect_fut = alice.connect(bob_id, alpn.clone());
        let accept_fut = bob.accept(alpn);
        let (alice_res, bob_res) = tokio::join!(connect_fut, accept_fut);
        let mut alice_stream = alice_res.unwrap();
        let mut bob_stream = bob_res.unwrap().into_stream();

        // Alice writes; bob reads.
        let payload = b"hello over the memory transport";
        alice_stream.write_all(payload).await.unwrap();
        alice_stream.flush().await.unwrap();
        let mut buf = vec![0u8; payload.len()];
        bob_stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, payload);

        // And the reverse.
        let reply = b"got it";
        bob_stream.write_all(reply).await.unwrap();
        bob_stream.flush().await.unwrap();
        let mut buf = vec![0u8; reply.len()];
        alice_stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, reply);
    }

    #[tokio::test]
    async fn connect_to_wrong_peer_is_refused() {
        let (alice_id, bob_id) = alice_bob_ids();
        let charlie_id =
            PeerID::from_public_key(InMemoryProvider::from_seed([3; 32]).master_public_key());
        let (alice, _bob) = MemoryTransport::pair(alice_id, bob_id);

        let result = alice.connect(charlie_id, Alpn::new("mere/test/v1")).await;
        assert!(matches!(result, Err(TransportError::ConnectionRefused)));
    }

    #[tokio::test]
    async fn different_alpns_route_independently() {
        let (alice_id, bob_id) = alice_bob_ids();
        let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);

        let cable_alpn = Alpn::new("mere/cable/v1");
        let coop_alpn = Alpn::new("mere/coop/v1");

        // Alice connects on Cable; bob accepts on Cable.
        let cable_pair = tokio::join!(
            alice.connect(bob_id, cable_alpn.clone()),
            bob.accept(cable_alpn.clone())
        );
        assert!(cable_pair.0.is_ok());
        assert!(cable_pair.1.is_ok());

        // Alice connects on co-op; bob accepts on co-op. Independent
        // channel — doesn't interact with the Cable streams above.
        let coop_pair = tokio::join!(
            alice.connect(bob_id, coop_alpn.clone()),
            bob.accept(coop_alpn.clone())
        );
        assert!(coop_pair.0.is_ok());
        assert!(coop_pair.1.is_ok());
    }

    #[tokio::test]
    async fn multiple_concurrent_connects_on_same_alpn_each_get_their_stream() {
        // A protocol handler running an accept loop should be able to
        // service multiple incoming connections on the same ALPN.
        let (alice_id, bob_id) = alice_bob_ids();
        let (alice, bob) = MemoryTransport::pair(alice_id, bob_id);
        let alpn = Alpn::new("mere/test/v1");

        // Three concurrent connects from Alice.
        let alpn_clone = alpn.clone();
        let connect_a = alice.connect(bob_id, alpn_clone.clone());
        let connect_b = alice.connect(bob_id, alpn_clone.clone());
        let connect_c = alice.connect(bob_id, alpn_clone);

        // Three sequential accepts on Bob.
        let accept_a = bob.accept(alpn.clone());
        let accept_b = bob.accept(alpn.clone());
        let accept_c = bob.accept(alpn);

        let ((ca, cb, cc), (aa, ab, ac)) = tokio::join!(
            async { tokio::join!(connect_a, connect_b, connect_c) },
            async { tokio::join!(accept_a, accept_b, accept_c) }
        );

        assert!(ca.is_ok() && cb.is_ok() && cc.is_ok());
        assert!(aa.is_ok() && ab.is_ok() && ac.is_ok());
    }
}
