// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Shared lifecycle for one p2panda gossip overlay.
//!
//! Product hosts still own identity, admission, stores, and protocol handlers.
//! This type owns the repeated transport work around them: relay and discovery
//! setup, overlay route seeding, endpoint reporting, and orderly endpoint
//! shutdown.

use crate::p2panda_transport::{KnownPeer, MdnsDiscoveryMode, P2pandaTransportBuilder, RelayUrl};
use crate::{P2pandaTransport, PeerID, PeerPath, Transport, TransportError};

/// Owner-selected network policy for a resident p2panda endpoint.
#[derive(Clone, Debug)]
pub struct P2pandaHostPolicy {
    /// Local discovery mode. `None` disables mDNS.
    pub mdns: Option<MdnsDiscoveryMode>,
    /// Explicit relays trusted by the owner.
    pub relay_urls: Vec<RelayUrl>,
}

impl Default for P2pandaHostPolicy {
    fn default() -> Self {
        Self {
            mdns: Some(MdnsDiscoveryMode::Active),
            relay_urls: Vec::new(),
        }
    }
}

impl P2pandaHostPolicy {
    /// Parse owner-configured relay strings with one fail-closed policy.
    pub fn parse_relay_urls<'a>(
        urls: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<RelayUrl>, TransportError> {
        urls.into_iter()
            .map(|url| {
                url.parse::<RelayUrl>()
                    .map_err(|error| TransportError::Backend(format!("relay URL {url:?}: {error}")))
            })
            .collect()
    }

    /// Apply owner-selected discovery and relay policy to a product builder.
    pub fn configure<'a>(
        &self,
        mut builder: P2pandaTransportBuilder<'a>,
    ) -> P2pandaTransportBuilder<'a> {
        if let Some(mode) = &self.mdns {
            builder = builder.mdns(mode.clone());
        }
        for relay in &self.relay_urls {
            builder = builder.relay_url(relay.clone());
        }
        builder
    }
}

/// A stored route that could not be restored during startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedPeerHint {
    /// The supplied ticket.
    pub ticket: String,
    /// Parse or address-book failure.
    pub error: String,
}

/// One bound p2panda transport associated with one caller-chosen overlay.
///
/// Association with this overlay is reachability state only. The caller must
/// separately admit operations and content reads from its own authority.
pub struct P2pandaOverlayHost {
    transport: P2pandaTransport,
    topic: [u8; 32],
}

impl P2pandaOverlayHost {
    /// Configure and bind a product-supplied transport builder.
    pub async fn bind(
        builder: P2pandaTransportBuilder<'_>,
        topic: [u8; 32],
        policy: &P2pandaHostPolicy,
    ) -> Result<Self, TransportError> {
        let transport = policy.configure(builder).bind().await?;
        Ok(Self { transport, topic })
    }

    /// The underlying transport for protocol-specific joining and blob I/O.
    pub fn transport(&self) -> &P2pandaTransport {
        &self.transport
    }

    /// Stable authenticated endpoint identity.
    pub fn local_peer_id(&self) -> PeerID {
        self.transport.local_peer_id()
    }

    /// Associate a peer identity with this overlay.
    pub async fn add_peer(&self, peer: [u8; 32]) -> Result<(), TransportError> {
        let peer = PeerID::from_bytes(&peer)
            .map_err(|error| TransportError::Backend(format!("overlay peer id: {error}")))?;
        self.transport.add_topics(peer, &[self.topic]).await
    }

    /// Remove only this overlay association, retaining learned routes.
    pub async fn remove_peer(&self, peer: [u8; 32]) -> Result<(), TransportError> {
        let peer = PeerID::from_bytes(&peer)
            .map_err(|error| TransportError::Backend(format!("overlay peer id: {error}")))?;
        self.transport.remove_topic(peer, self.topic).await
    }

    /// Seed identities that should participate in this overlay.
    pub async fn seed_peers(
        &self,
        peers: impl IntoIterator<Item = [u8; 32]>,
    ) -> Result<(), TransportError> {
        for peer in peers {
            self.add_peer(peer).await?;
        }
        Ok(())
    }

    /// Apply an explicit, owner-supplied ticket and tag its peer immediately.
    pub async fn add_peer_ticket(&self, ticket: &str) -> Result<PeerID, TransportError> {
        let peer = self.transport.add_peer_ticket(ticket).await?;
        self.transport.add_topics(peer, &[self.topic]).await?;
        Ok(peer)
    }

    /// Restore cached route hints without making stale state a startup error.
    pub async fn seed_peer_hints(
        &self,
        hints: impl IntoIterator<Item = String>,
    ) -> Vec<RejectedPeerHint> {
        let mut rejected = Vec::new();
        for ticket in hints {
            if let Err(error) = self.add_peer_ticket(&ticket).await {
                rejected.push(RejectedPeerHint {
                    ticket,
                    error: error.to_string(),
                });
            }
        }
        rejected
    }

    /// Current route and connection report for this overlay.
    pub async fn known_peers(&self) -> Result<Vec<KnownPeer>, TransportError> {
        self.transport.peers_for_topic(self.topic).await
    }

    /// This endpoint's shareable current route.
    pub async fn ticket(&self) -> Result<String, TransportError> {
        self.transport.ticket().await
    }

    /// Current route for one peer, if known.
    pub async fn peer_ticket(&self, peer: [u8; 32]) -> Result<Option<String>, TransportError> {
        let peer = PeerID::from_bytes(&peer)
            .map_err(|error| TransportError::Backend(format!("overlay peer id: {error}")))?;
        self.transport.peer_ticket(peer).await
    }

    /// Every address held for one peer, with which ones carry traffic now.
    pub async fn peer_paths(&self, peer: [u8; 32]) -> Result<Vec<PeerPath>, TransportError> {
        let peer = PeerID::from_bytes(&peer)
            .map_err(|error| TransportError::Backend(format!("overlay peer id: {error}")))?;
        self.transport.peer_paths(peer).await
    }

    /// Gracefully close the endpoint after protocol sessions have left.
    pub async fn close(&self) -> Result<(), TransportError> {
        self.transport.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_host_policy_does_not_share_node_identity() {
        let policy = P2pandaHostPolicy {
            mdns: None,
            relay_urls: Vec::new(),
        };
        let personal = P2pandaOverlayHost::bind(
            P2pandaTransport::builder_from_seed([31; 32]).gossip(),
            [1; 32],
            &policy,
        )
        .await
        .unwrap();
        let knot = P2pandaOverlayHost::bind(
            P2pandaTransport::builder_from_seed([32; 32]).gossip(),
            [2; 32],
            &policy,
        )
        .await
        .unwrap();

        let personal_id = personal.local_peer_id();
        assert_ne!(personal_id, knot.local_peer_id());
        personal.close().await.unwrap();
        knot.close().await.unwrap();

        let explicitly_reused = P2pandaOverlayHost::bind(
            P2pandaTransport::builder_from_seed([31; 32]).gossip(),
            [3; 32],
            &policy,
        )
        .await
        .unwrap();
        assert_eq!(personal_id, explicitly_reused.local_peer_id());
        explicitly_reused.close().await.unwrap();
    }
}
