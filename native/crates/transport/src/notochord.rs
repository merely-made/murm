// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The one audited conversion from an acceptance record into carrier facts.
//!
//! Notochord N1. Before this existed, every service that wanted to run the
//! session handshake hand-built the same five lines, and a five-line copy is
//! exactly where the D4 rule ("transport facts are facts, not claims") gets
//! quietly broken: one call site reads a subject out of an application frame
//! instead of off the connection and nothing catches it. There is one
//! construction site now, and it is this file.
//!
//! The dependency deliberately runs this way: `notochord` never learns
//! about iroh, p2panda, or retinue, so the policy core stays carrier-neutral
//! and no cycle can form.

use notochord::{CarrierKind, IngressFacts, ProofBinding, SessionFacts};

use crate::{AcceptedSession, Alpn, PeerID, TransportKind};

fn carrier_of(transport: TransportKind) -> CarrierKind {
    match transport {
        TransportKind::Memory => CarrierKind::Memory,
        TransportKind::P2panda => CarrierKind::P2panda,
        TransportKind::Reticulum => CarrierKind::Reticulum,
        // notochord has no Noise-specific vocabulary, and inventing one here
        // would claim knowledge that crate does not have.
        TransportKind::Noise => CarrierKind::Other,
        // notochord has no WebRTC-specific vocabulary yet either, and
        // inventing a variant would claim knowledge that crate does not have.
        TransportKind::WebRtc => CarrierKind::Other,
    }
}

impl<S> AcceptedSession<S> {
    /// What the carrier observed about this session.
    ///
    /// Every field is read off the acceptance record. Nothing here can be
    /// influenced by the bytes the initiator later sends, because this is
    /// built before a single application byte is read: an authenticated peer
    /// is `Some` only where the carrier itself proved one (p2panda, Memory),
    /// and Reticulum's honest `None` is preserved rather than being filled in
    /// from a claim.
    ///
    /// The local interface id is carried for the owner's policy and never
    /// enters a proof; see `notochord::ProofBinding`.
    pub fn session_facts(&self) -> SessionFacts {
        SessionFacts {
            protocol: self.protocol.as_bytes().to_vec(),
            transport: carrier_of(self.ingress.transport),
            authenticated_initiator: self.peer.map(|peer| peer.to_bytes()),
            ingress: IngressFacts {
                local_interface: self.ingress.interface.map(|iface| iface.0),
                shared_link: self.ingress.link,
            },
        }
    }

    /// Consume the record, yielding the stream and its carrier facts.
    ///
    /// The shape an accept path wants: run the handshake on the stream, and
    /// hand it to the application only if the facts and the proof admit it.
    pub fn into_session(self) -> (S, SessionFacts) {
        let facts = self.session_facts();
        (self.stream, facts)
    }
}

/// The binding an **initiator** signs, for a carrier that authenticates peers.
///
/// `local_peer` is this node's own identity, not the peer it is dialling. The
/// responder will derive the same value from the peer its carrier
/// authenticated, and the proof only verifies because the two agree.
pub fn initiator_binding(protocol: &Alpn, local_peer: PeerID) -> ProofBinding {
    ProofBinding::initiator(
        protocol.as_bytes().to_vec(),
        Some(local_peer.to_bytes()),
        None,
    )
}

/// The binding an **initiator** signs on a link-oriented carrier that cannot
/// authenticate peers, such as Reticulum.
///
/// There is no transport identity to bind, so the shared link carries the
/// weight: both ends of a retinue link compute the same id, which is what
/// stops a captured hello from being replayed onto another link.
pub fn initiator_link_binding(protocol: &Alpn, shared_link: [u8; 16]) -> ProofBinding {
    ProofBinding::initiator(protocol.as_bytes().to_vec(), None, Some(shared_link))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IngressContext, IngressInterfaceId};
    use identity::{IdentityProvider, InMemoryProvider};

    fn peer(seed: u8) -> PeerID {
        PeerID::from_public_key(InMemoryProvider::from_seed([seed; 32]).master_public_key())
    }

    fn accepted(peer: Option<PeerID>, ingress: IngressContext) -> AcceptedSession<()> {
        AcceptedSession::new((), Alpn::new("mere/murm/v1"), peer, ingress)
    }

    #[test]
    fn p2panda_facts_carry_the_authenticated_initiator() {
        let facts = accepted(Some(peer(4)), IngressContext::p2panda()).session_facts();
        assert_eq!(facts.transport, CarrierKind::P2panda);
        assert_eq!(facts.authenticated_initiator, Some(peer(4).to_bytes()));
        assert_eq!(facts.ingress.shared_link, None);
        assert_eq!(facts.protocol, b"mere/murm/v1".to_vec());
    }

    #[test]
    fn memory_facts_carry_its_constructed_counterparty() {
        let facts = accepted(Some(peer(9)), IngressContext::memory()).session_facts();
        assert_eq!(facts.transport, CarrierKind::Memory);
        assert_eq!(facts.authenticated_initiator, Some(peer(9).to_bytes()));
    }

    #[test]
    fn reticulum_facts_keep_the_honest_none_and_the_bearer_detail() {
        let facts = accepted(
            None,
            IngressContext::reticulum(IngressInterfaceId(7), [0xab; 16]),
        )
        .session_facts();
        assert_eq!(facts.transport, CarrierKind::Reticulum);
        assert_eq!(
            facts.authenticated_initiator, None,
            "best-effort acceptance must not invent an initiator"
        );
        assert_eq!(facts.ingress.local_interface, Some(7));
        assert_eq!(facts.ingress.shared_link, Some([0xab; 16]));
    }

    #[test]
    fn the_local_interface_is_carried_but_never_signed() {
        let here = accepted(
            None,
            IngressContext::reticulum(IngressInterfaceId(1), [0xcd; 16]),
        )
        .session_facts();
        let there = accepted(
            None,
            IngressContext::reticulum(IngressInterfaceId(9_999), [0xcd; 16]),
        )
        .session_facts();

        assert_ne!(here.ingress.local_interface, there.ingress.local_interface);
        assert_eq!(
            here.proof_binding(),
            there.proof_binding(),
            "a purely local number cannot change what a peer has to sign"
        );
    }

    #[test]
    fn the_two_roles_derive_the_same_binding() {
        let alpn = Alpn::new("mere/murm/v1");
        let initiator = peer(4);
        // Responder: off its own acceptance. Initiator: from its own identity.
        let responder = accepted(Some(initiator), IngressContext::p2panda())
            .session_facts()
            .proof_binding();
        assert_eq!(responder, initiator_binding(&alpn, initiator));
    }

    #[test]
    fn a_link_binding_pins_the_link_and_names_no_peer() {
        let alpn = Alpn::new("mere/murm/v1");
        let responder = accepted(
            None,
            IngressContext::reticulum(IngressInterfaceId(3), [0x11; 16]),
        )
        .session_facts()
        .proof_binding();
        assert_eq!(responder, initiator_link_binding(&alpn, [0x11; 16]));
        assert_ne!(
            responder,
            initiator_link_binding(&alpn, [0x22; 16]),
            "a different link is a different binding"
        );
    }

    #[test]
    fn webrtc_facts_keep_the_honest_none_and_the_bearer_detail() {
        let facts = accepted(None, IngressContext::webrtc([0x11; 16])).session_facts();
        assert_eq!(facts.transport, CarrierKind::Other);
        assert_eq!(
            facts.authenticated_initiator, None,
            "the WebRTC carrier authenticates no initiator"
        );
        assert_eq!(facts.ingress.local_interface, None);
        assert_eq!(facts.ingress.shared_link, Some([0x11; 16]));
    }

    #[test]
    fn a_webrtc_binding_pins_the_link_and_names_no_peer() {
        let alpn = Alpn::new("mere/murm/v1");
        let responder = accepted(None, IngressContext::webrtc([0x11; 16]))
            .session_facts()
            .proof_binding();
        assert_eq!(responder, initiator_link_binding(&alpn, [0x11; 16]));
        assert_ne!(
            responder,
            initiator_link_binding(&alpn, [0x22; 16]),
            "a different link is a different binding"
        );
    }

    #[test]
    fn into_session_hands_back_the_stream_and_the_same_facts() {
        let session = accepted(Some(peer(4)), IngressContext::p2panda());
        let expected = session.session_facts();
        let ((), facts) = session.into_session();
        assert_eq!(facts, expected);
    }
}
