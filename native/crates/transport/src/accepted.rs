// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Honest acceptance context for inbound sessions.
//!
//! V4 of the 2026-07-24 low-power radio and managed-network plan
//! ([`mere/design_docs/mere_docs/implementation_strategy/2026-07-24_low_power_managed_network_plan.md`](../../../../../design_docs/mere_docs/implementation_strategy/2026-07-24_low_power_managed_network_plan.md)).
//!
//! [`crate::Transport::accept`] used to return a naked stream, which left a
//! policy layer above with nothing to decide on: it could not tell which
//! protocol a session claimed, whether the transport had actually
//! authenticated the peer, or which bearer the session arrived over.
//!
//! ## D4: transport facts are facts, not claims
//!
//! The plan's load-bearing rule. [`AcceptedSession::peer`] is `Some` **only**
//! when the transport itself authenticated the peer:
//!
//! - p2panda authenticates its connections, so it reports the peer.
//! - [`crate::MemoryTransport`] is a paired test fixture, so its counterparty
//!   is known by construction.
//! - Reticulum best-effort acceptance cannot identify its initiator, so it
//!   reports `None`, and an application identity arrives later through a
//!   session proof (plan D6).
//!
//! A subject named by *application bytes* is never placed here. Code that
//! wants "who does this peer claim to be" must carry that separately, so the
//! two can never be confused at a policy boundary.

use crate::{Alpn, PeerID};

/// Which transport carried a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// In-process paired fixture.
    Memory,
    /// p2panda / iroh.
    P2panda,
    /// Reticulum, over any retinue interface (TCP, direct PHY, ...).
    Reticulum,
    /// Noise over plain TCP: encrypted and mutually authenticated with no
    /// certificate machinery.
    Noise,
    /// A browser-initiated WebRTC data channel. The carrier authenticates no
    /// initiator; link identity instead comes from a host-signed
    /// DTLS-fingerprint challenge, not from the transport itself.
    WebRtc,
}

/// Opaque local identifier for the interface a session arrived on.
///
/// Deliberately not a retinue type: `mere-transport` must build without
/// retinue, and the number is meaningful only to the local node (it is not
/// stable across restarts and never goes on the wire).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressInterfaceId(pub u64);

/// Where a session physically arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IngressContext {
    /// The transport that carried it.
    pub transport: TransportKind,
    /// The local interface it arrived on, when the transport tracks one.
    pub interface: Option<IngressInterfaceId>,
    /// The link it arrived on, when the transport has link identity.
    pub link: Option<[u8; 16]>,
}

impl IngressContext {
    /// Context for a transport with no interface or link identity.
    pub fn bare(transport: TransportKind) -> Self {
        Self {
            transport,
            interface: None,
            link: None,
        }
    }

    /// Context for the in-process fixture.
    pub fn memory() -> Self {
        Self::bare(TransportKind::Memory)
    }

    /// Context for a p2panda connection.
    pub fn p2panda() -> Self {
        Self::bare(TransportKind::P2panda)
    }

    /// Context for a Noise session. Noise over TCP has neither an interface
    /// number nor a link identity, so both stay absent rather than invented.
    pub fn noise() -> Self {
        Self::bare(TransportKind::Noise)
    }

    /// Context for a Reticulum link, carrying the interface it arrived on and
    /// the link it belongs to.
    pub fn reticulum(interface: IngressInterfaceId, link: [u8; 16]) -> Self {
        Self {
            transport: TransportKind::Reticulum,
            interface: Some(interface),
            link: Some(link),
        }
    }

    /// Context for a browser WebRTC data channel. WebRTC has no local
    /// interface number, so that stays absent; the link identity comes from
    /// the host-signed DTLS-fingerprint challenge shared by both ends.
    pub fn webrtc(shared_link: [u8; 16]) -> Self {
        Self {
            transport: TransportKind::WebRtc,
            interface: None,
            link: Some(shared_link),
        }
    }
}

/// An inbound session plus the facts the transport can honestly report.
#[derive(Debug)]
pub struct AcceptedSession<S> {
    /// The bidirectional stream.
    pub stream: S,
    /// The protocol it was accepted for.
    pub protocol: Alpn,
    /// The peer, **only** when the transport authenticated it. See the module
    /// docs: never populated from application bytes.
    pub peer: Option<PeerID>,
    /// Where it arrived.
    pub ingress: IngressContext,
}

impl<S> AcceptedSession<S> {
    /// Build an accepted session.
    pub fn new(stream: S, protocol: Alpn, peer: Option<PeerID>, ingress: IngressContext) -> Self {
        Self {
            stream,
            protocol,
            peer,
            ingress,
        }
    }

    /// Whether the transport authenticated the peer.
    ///
    /// A policy that requires transport identity checks this rather than
    /// unwrapping [`Self::peer`], so the intent is explicit at the call site.
    pub fn is_transport_authenticated(&self) -> bool {
        self.peer.is_some()
    }

    /// Consume the session, returning just the stream.
    ///
    /// For call sites that genuinely do not care about ingress (protocol tests
    /// and the like). Production accept paths should keep the context.
    pub fn into_stream(self) -> S {
        self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerID {
        use identity::{IdentityProvider, InMemoryProvider};
        PeerID::from_public_key(InMemoryProvider::from_seed([7; 32]).master_public_key())
    }

    #[test]
    fn authenticated_only_when_a_peer_is_present() {
        let authed = AcceptedSession::new(
            (),
            Alpn::new("mere/cable/v1"),
            Some(peer()),
            IngressContext::p2panda(),
        );
        assert!(authed.is_transport_authenticated());

        let anonymous = AcceptedSession::new(
            (),
            Alpn::new("mere/cable/v1"),
            None,
            IngressContext::reticulum(IngressInterfaceId(3), [0xab; 16]),
        );
        assert!(
            !anonymous.is_transport_authenticated(),
            "a transport that cannot identify its initiator must not look authenticated"
        );
    }

    #[test]
    fn reticulum_context_carries_interface_and_link() {
        let ctx = IngressContext::reticulum(IngressInterfaceId(9), [1u8; 16]);
        assert_eq!(ctx.transport, TransportKind::Reticulum);
        assert_eq!(ctx.interface, Some(IngressInterfaceId(9)));
        assert_eq!(ctx.link, Some([1u8; 16]));
    }

    #[test]
    fn bare_contexts_have_no_bearer_detail() {
        for ctx in [IngressContext::memory(), IngressContext::p2panda()] {
            assert!(ctx.interface.is_none());
            assert!(ctx.link.is_none());
        }
        assert_eq!(IngressContext::memory().transport, TransportKind::Memory);
        assert_eq!(IngressContext::p2panda().transport, TransportKind::P2panda);
    }

    #[test]
    fn a_webrtc_session_reports_no_transport_authentication() {
        let session = AcceptedSession::new(
            (),
            Alpn::new("mere/cable/v1"),
            None,
            IngressContext::webrtc([0x42; 16]),
        );
        assert!(
            !session.is_transport_authenticated(),
            "the WebRTC carrier authenticates no initiator"
        );
        assert_eq!(session.peer, None);
    }

    #[test]
    fn webrtc_context_carries_the_link_and_no_interface() {
        let ctx = IngressContext::webrtc([2u8; 16]);
        assert_eq!(ctx.transport, TransportKind::WebRtc);
        assert_eq!(ctx.interface, None);
        assert_eq!(ctx.link, Some([2u8; 16]));
    }
}
