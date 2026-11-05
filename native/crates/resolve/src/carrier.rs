// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! One adapter per carrier: find routes to holders, pull through a lane.
//!
//! An adapter never decides what is true. It lists routes (from its carrier's
//! own discovery and from hints) and runs the session it is handed through the
//! shared setup gate; the resolver's store and policy decide what is kept.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::{Duration, Instant};

use identity::Ed25519Keypair;
use murm_sync::StreamSyncOutcome;
use transport::security::SetupLimits;
use transport::sync_lane::{SyncLane, SyncLaneError};
use transport::{Alpn, PeerID, Transport};

use crate::hint::Hint;
use crate::wire::{ReadSession, key_alpn, release_alpn};

/// A boxed future, not required to be `Send`.
pub type Local<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// What an adapter needs to find routes for one address.
#[derive(Clone, Debug)]
pub struct Ask<'a> {
    pub key: [u8; 32],
    pub hints: &'a [Hint],
}

/// One way to reach one candidate holder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Shown in traces; carries no authority.
    pub label: String,
    /// The peer to dial on a peer-addressed carrier.
    pub peer: Option<PeerID>,
    /// The Noise key a hint says the holder should prove.
    pub expected: Option<PeerID>,
    /// A socket address from a hint.
    pub addr: Option<SocketAddr>,
}

/// Why a pull did not run to the end.
#[derive(Debug, thiserror::Error)]
pub enum PullError {
    /// The lane or the carrier refused or failed.
    #[error(transparent)]
    Lane(#[from] SyncLaneError),
    /// The route lacks what this carrier needs to dial it.
    #[error("route is unusable on this carrier")]
    Unusable,
}

/// A carrier the resolver can try.
pub trait Carrier {
    /// Short name for traces: `memory`, `p2panda`, `reticulum`, `noise+tcp`.
    fn name(&self) -> &'static str;
    /// Routes to candidate holders for `ask`. May wait on discovery.
    fn routes<'a>(&'a self, ask: &'a Ask<'a>) -> Local<'a, Vec<Route>>;
    /// Dial `route` and run `session` through the setup gate.
    fn pull<'a>(
        &'a self,
        route: &'a Route,
        ask: &'a Ask<'a>,
        session: ReadSession,
    ) -> Local<'a, Result<StreamSyncOutcome, PullError>>;
}

fn lane_for(alpn: Alpn) -> SyncLane {
    SyncLane::new(alpn, 4, SetupLimits::default())
}

/// Any [`Transport`] whose peers are named by id: memory, Reticulum.
///
/// Dials with Noise and no expected responder key: a holder is anyone with a
/// copy, and the log verifies against its own key.
pub struct TransportAdapter<T> {
    name: &'static str,
    transport: T,
    noise_key: Ed25519Keypair,
    peers: Vec<PeerID>,
    discover: Option<fn(&T, &Alpn) -> Vec<PeerID>>,
    discovery_wait: Duration,
    per_key_alpn: bool,
}

impl<T: Transport> TransportAdapter<T> {
    /// An adapter dialing `transport` as `noise_key`, on the generic ALPN.
    pub fn new(name: &'static str, transport: T, noise_key: Ed25519Keypair) -> Self {
        Self {
            name,
            transport,
            noise_key,
            peers: Vec::new(),
            discover: None,
            discovery_wait: Duration::ZERO,
            per_key_alpn: false,
        }
    }

    /// Peers always tried (a memory pair's other end).
    #[must_use]
    pub fn with_peers(mut self, peers: Vec<PeerID>) -> Self {
        self.peers = peers;
        self
    }

    /// Find holders by announced name: dial on [`key_alpn`] and list peers the
    /// carrier heard announce it.
    #[must_use]
    pub fn announced(mut self, discover: fn(&T, &Alpn) -> Vec<PeerID>) -> Self {
        self.discover = Some(discover);
        self.per_key_alpn = true;
        self
    }

    /// Wait up to `wait` for announces before listing routes: announces
    /// arrive on the carrier's own schedule.
    #[must_use]
    pub fn waiting(mut self, wait: Duration) -> Self {
        self.discovery_wait = wait;
        self
    }

    fn alpn(&self, key: &[u8; 32]) -> Alpn {
        if self.per_key_alpn {
            key_alpn(key)
        } else {
            release_alpn()
        }
    }
}

impl<T: Transport> Carrier for TransportAdapter<T> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn routes<'a>(&'a self, ask: &'a Ask<'a>) -> Local<'a, Vec<Route>> {
        Box::pin(async move {
            let mut peers = self.peers.clone();
            if let Some(discover) = self.discover {
                let alpn = self.alpn(&ask.key);
                let start = Instant::now();
                loop {
                    let found = discover(&self.transport, &alpn);
                    if !found.is_empty() || start.elapsed() >= self.discovery_wait {
                        peers.extend(found);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            peers.extend(ask.hints.iter().filter_map(|hint| match hint {
                Hint::Peer(peer) => Some(*peer),
                _ => None,
            }));
            let mut seen = Vec::new();
            for peer in peers {
                if !seen.contains(&peer) && peer != self.transport.local_peer_id() {
                    seen.push(peer);
                }
            }
            seen.into_iter()
                .map(|peer| Route {
                    label: format!("{} peer {}", self.name, short(&peer)),
                    peer: Some(peer),
                    expected: None,
                    addr: None,
                })
                .collect()
        })
    }

    fn pull<'a>(
        &'a self,
        route: &'a Route,
        ask: &'a Ask<'a>,
        session: ReadSession,
    ) -> Local<'a, Result<StreamSyncOutcome, PullError>> {
        Box::pin(async move {
            let peer = route.peer.ok_or(PullError::Unusable)?;
            Ok(lane_for(self.alpn(&ask.key))
                .dial_noise_any(&self.transport, peer, &self.noise_key, None, session)
                .await?)
        })
    }
}

/// Noise over TCP, from `noise+tcp://` hints only: the address is input and
/// the hint names the key the holder must prove.
pub struct NoiseTcpAdapter {
    noise_key: Ed25519Keypair,
}

impl NoiseTcpAdapter {
    /// An adapter dialing as `noise_key`.
    pub fn new(noise_key: Ed25519Keypair) -> Self {
        Self { noise_key }
    }
}

impl Carrier for NoiseTcpAdapter {
    fn name(&self) -> &'static str {
        "noise+tcp"
    }

    fn routes<'a>(&'a self, ask: &'a Ask<'a>) -> Local<'a, Vec<Route>> {
        Box::pin(async move {
            ask.hints
                .iter()
                .filter_map(|hint| match hint {
                    Hint::NoiseTcp { addr, peer } => Some(Route {
                        label: format!("noise+tcp {addr}"),
                        peer: None,
                        expected: Some(*peer),
                        addr: Some(*addr),
                    }),
                    _ => None,
                })
                .collect()
        })
    }

    fn pull<'a>(
        &'a self,
        route: &'a Route,
        _ask: &'a Ask<'a>,
        session: ReadSession,
    ) -> Local<'a, Result<StreamSyncOutcome, PullError>> {
        Box::pin(async move {
            let (Some(addr), Some(expected)) = (route.addr, route.expected) else {
                return Err(PullError::Unusable);
            };
            Ok(lane_for(release_alpn())
                .dial_noise_tcp(addr, &self.noise_key, expected, session)
                .await?)
        })
    }
}

pub(crate) fn short(peer: &PeerID) -> String {
    peer.to_bytes()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
