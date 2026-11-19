//! Carrying traffic for other destinations: bridges, the reverse table and relaying.

use heapless::Vec as BoundedVec;

use super::tables::is_deduplicated_link_context;
use super::{
    Action, Actions, InterfaceId, LINK_TRANSPORT_IDLE, Node, REVERSE_TIMEOUT, transit_proof_timeout,
};
use crate::hash::AddressHash;
use crate::link;
use crate::packet::{HeaderType, Packet, PacketType};
use crate::transport::{
    BridgeRecord, LinkBridge, ReverseEntry, find_live, record_bridge, record_reverse,
    refresh_bridge, restore_reverse, take_reverse,
};

impl<const PEERS: usize, const ACTIONS: usize, const LINKS: usize, const ROUTES: usize>
    Node<PEERS, ACTIONS, LINKS, ROUTES>
{
    /// Whether this is a fresh packet for a shared-radio relay. Resource parts and keepalives
    /// legitimately repeat their hash, so they are never filtered, as RNS exempts them
    /// (`Transport.py` 1635-1640). Channel is filtered here, unlike RNS: see N10.
    pub(super) fn transit_is_new(&mut self, packet: &Packet, now: u64) -> bool {
        self.transit_hash_is_new(packet.context, packet.hash(), now)
    }

    /// [`Self::transit_is_new`] for a packet whose hash is already in hand.
    fn transit_hash_is_new(&mut self, context: u8, hash: AddressHash, now: u64) -> bool {
        !is_deduplicated_link_context(context) || self.transit_filter.insert(hash, now)
    }

    /// Record a link request this node is about to carry, unvalidated until `proof_deadline`.
    /// At capacity the stalest unvalidated bridge makes room, or failing one, a validated
    /// bridge unheard for [`LINK_TRANSPORT_IDLE`]. A recently heard validated link is never
    /// evicted for a request, so forged requests cannot displace carried links; with every
    /// slot holding one, the request is refused. Returns whether it may be carried.
    pub(super) fn remember_bridge(
        &mut self,
        link_id: AddressHash,
        destination: AddressHash,
        from: InterfaceId,
        out: InterfaceId,
        proof_deadline: u64,
        now: u64,
    ) -> bool {
        let bridge = LinkBridge {
            from,
            out,
            destination,
            seen: now,
            proof_deadline: Some(proof_deadline),
        };
        let counters = &mut self.transport_counters;
        match record_bridge(
            &mut self.bridges,
            link_id,
            bridge,
            ROUTES,
            LINK_TRANSPORT_IDLE,
        ) {
            BridgeRecord::Known => true,
            BridgeRecord::New { evicted } => {
                if evicted.is_some() {
                    counters.evicted_bridges = counters.evicted_bridges.saturating_add(1);
                }
                true
            }
            BridgeRecord::Refused => {
                counters.refused_bridges = counters.refused_bridges.saturating_add(1);
                false
            }
        }
    }

    /// Relay a packet already associated with a carried link. Link proofs and data name the
    /// link id rather than the original destination, so this lookup precedes normal transit
    /// routing.
    pub(super) fn forward_bridged_packet(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        now: u64,
        actions: &mut Actions<ACTIONS>,
    ) -> bool {
        if !self.transport.relay_packets {
            return false;
        }
        let bridge_ttl = self.transport.bridge_ttl;
        let Some(bridge) = find_live(&self.bridges, &packet.destination, |b| {
            !b.lapsed(now, bridge_ttl)
        })
        .copied() else {
            return false;
        };
        let out = if interface == bridge.from {
            bridge.out
        } else if interface == bridge.out {
            bridge.from
        } else {
            // A third interface cannot extend this bridge's lifetime or poison the
            // relay de-duplication cache for a later packet from a real endpoint.
            return true;
        };
        // Nothing crosses a bridge before the destination's proof, and the proof itself
        // must come from the destination's side under its signature. With its identity
        // unknown here, the side check stands alone. The proof validates the bridge only
        // once it is on its way to the initiator.
        let proves = packet.packet_type == PacketType::Proof && packet.context == link::CTX_LRPROOF;
        // Whether the proof's signature was checked against the destination's identity.
        let mut verified = false;
        if proves {
            let checked = self
                .book
                .resolve(bridge.destination)
                .map(|peer| link::proof_is_signed_by(packet, &peer.identity));
            verified = checked == Some(true);
            let signed = interface == bridge.out && checked != Some(false);
            if !signed {
                self.transport_counters.unvalidated_link_packets = self
                    .transport_counters
                    .unvalidated_link_packets
                    .saturating_add(1);
                return true;
            }
        } else if bridge.proof_deadline.is_some() {
            self.transport_counters.unvalidated_link_packets = self
                .transport_counters
                .unvalidated_link_packets
                .saturating_add(1);
            return true;
        }
        if packet.hops.saturating_add(1) >= self.transport.max_hops {
            self.transport_counters.hop_limit_dropped =
                self.transport_counters.hop_limit_dropped.saturating_add(1);
            return true;
        }
        if !self.transit_is_new(packet, now) {
            return true;
        }
        let mut forwarded = packet.clone();
        forwarded.hops = forwarded.hops.saturating_add(1);
        forwarded.header_type = HeaderType::Type1;
        forwarded.transport = None;
        if forwarded.encoded_len() > self.logical_mtu as usize {
            self.refused_payloads = self.refused_payloads.saturating_add(1);
            return true;
        }
        if actions.push(Action::Send {
            interface: out,
            packet: forwarded,
        }) {
            refresh_bridge(&mut self.bridges, &packet.destination, now, proves);
            if verified {
                // A verified proof proves its destination alive (`Transport.py` 2928-2938).
                self.prove_route_alive(bridge.destination, now);
            } else {
                // The link's destination route is in use (`Transport.py` 2440).
                self.mark_route_used(bridge.destination, now);
            }
            self.transport_counters.forwarded_packets =
                self.transport_counters.forwarded_packets.saturating_add(1);
        }
        true
    }

    /// Carry a header-type-2 packet addressed to this node towards its learned destination.
    pub(super) fn forward_transport_packet(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        now: u64,
        actions: &mut Actions<ACTIONS>,
    ) -> bool {
        self.forward_transport_packet_with(interface, packet, now, actions, |_, actions, action| {
            actions.push(action)
        })
    }

    /// Keep output admission observable while the return path is being prepared.
    pub(super) fn forward_transport_packet_with(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        now: u64,
        actions: &mut Actions<ACTIONS>,
        send: impl FnOnce(&Self, &mut Actions<ACTIONS>, Action) -> bool,
    ) -> bool {
        if !self.transport.relay_packets
            || packet.header_type != HeaderType::Type2
            || packet.transport != Some(self.identity.hash())
            || packet.destination == self.destination()
        {
            return false;
        }
        if packet.hops.saturating_add(1) >= self.transport.max_hops {
            self.transport_counters.hop_limit_dropped =
                self.transport_counters.hop_limit_dropped.saturating_add(1);
            return true;
        }
        // Remember eligible packets even when their destination has no route.
        let hash = packet.hash();
        if !self.transit_hash_is_new(packet.context, hash, now) {
            return true;
        }
        let Some(route) = self
            .live_route_slot(packet.destination, now)
            .map(|slot| slot.entry)
        else {
            self.transport_counters.unroutable_packets =
                self.transport_counters.unroutable_packets.saturating_add(1);
            return true;
        };
        let mut forwarded = packet.clone();
        forwarded.hops = forwarded.hops.saturating_add(1);
        forwarded.header_type = HeaderType::Type1;
        forwarded.transport = None;
        if let Some(next_transport) = route.transport {
            forwarded.header_type = HeaderType::Type2;
            forwarded.transport = Some(next_transport);
        }
        // Every interface here carries `logical_mtu`, so it bounds the link on both sides.
        if packet.packet_type == PacketType::LinkRequest
            && link::clamp_request_mtu(&mut forwarded, self.logical_mtu).is_err()
        {
            self.transport_counters.undecodable_link_requests = self
                .transport_counters
                .undecodable_link_requests
                .saturating_add(1);
            return true;
        }
        if forwarded.encoded_len() > self.logical_mtu as usize {
            self.refused_payloads = self.refused_payloads.saturating_add(1);
            return true;
        }
        // Only a request that will be sent takes a bridge slot: an unsent one would hold it,
        // or evict another, until its deadline. A full action list refuses the send below.
        if packet.packet_type == PacketType::LinkRequest && actions.len() < ACTIONS {
            // A short request's link id covers all its bytes; its destination rejects it.
            let link_id = link::link_id(packet).unwrap_or(hash);
            let proof_deadline = now
                .saturating_add(transit_proof_timeout(route.hops))
                .saturating_add(self.first_hop_airtime(route.iface));
            if !self.remember_bridge(
                link_id,
                packet.destination,
                interface,
                route.iface,
                proof_deadline,
                now,
            ) {
                return true;
            }
        }
        let needs_reverse = packet.packet_type != PacketType::LinkRequest;
        // Everything recording the return path displaced, so a refused send puts it back.
        let mut displaced = BoundedVec::<_, ROUTES>::new();
        if needs_reverse {
            let entry = ReverseEntry {
                received: interface,
                outbound: route.iface,
                at: now,
            };
            record_reverse(
                &mut self.reverse,
                hash,
                entry,
                ROUTES,
                REVERSE_TIMEOUT,
                |d| {
                    let _ = displaced.push(d);
                },
            );
        }
        if send(
            self,
            actions,
            Action::Send {
                interface: route.iface,
                packet: forwarded,
            },
        ) {
            // A route carrying transit is in use (`Transport.py` 2387).
            self.mark_route_used(packet.destination, now);
            self.transport_counters.forwarded_packets =
                self.transport_counters.forwarded_packets.saturating_add(1);
        } else if needs_reverse {
            restore_reverse(&mut self.reverse, &hash, displaced.into_iter().rev());
        }
        true
    }

    /// Carry a delivery proof back along a remembered reverse path. The entry is consumed
    /// either way; a proof arriving on any interface but the one the packet left by is not
    /// carried (RNS `Transport.py` 2733-2744). Returns whether the proof was carried.
    pub(super) fn forward_reverse_proof(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        now: u64,
        actions: &mut Actions<ACTIONS>,
    ) -> bool {
        if !self.transport.relay_packets
            || matches!(packet.context, link::CTX_LRPROOF | link::CTX_RESOURCE_PRF)
        {
            return false;
        }
        let reverse = &mut self.reverse;
        let Some(received) = take_reverse(
            reverse,
            &packet.destination,
            interface,
            now,
            REVERSE_TIMEOUT,
        ) else {
            return false;
        };
        if packet.hops.saturating_add(1) >= self.transport.max_hops {
            self.transport_counters.hop_limit_dropped =
                self.transport_counters.hop_limit_dropped.saturating_add(1);
            return true;
        }
        let mut forwarded = packet.clone();
        forwarded.hops = forwarded.hops.saturating_add(1);
        if forwarded.encoded_len() > self.logical_mtu as usize {
            self.refused_payloads = self.refused_payloads.saturating_add(1);
            return true;
        }
        if actions.push(Action::Send {
            interface: received,
            packet: forwarded,
        }) {
            self.transport_counters.forwarded_packets =
                self.transport_counters.forwarded_packets.saturating_add(1);
        }
        true
    }

    /// Schedule a verified announce for rebroadcast with this node recorded as the transport
    /// hop. It leaves from [`Node::poll`], back out the interface it was heard on.
    pub(super) fn relay_announce(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        emitted: u64,
        now: u64,
    ) {
        // A path response answers one requester. RNS learns from it but never queues it for
        // rebroadcast, so one path request cannot flood the mesh.
        if !self.transport.relay_announces
            || packet.destination == self.destination()
            || packet.context == crate::path::CTX_PATH_RESPONSE
        {
            return;
        }
        // Remembered first, as stock (`Transport.py` 2235), so a repeat is filtered, not
        // counted again at the ceiling.
        if !self.transit_is_new(packet, now) {
            return;
        }
        if packet.hops.saturating_add(1) >= self.transport.max_hops {
            self.transport_counters.hop_limit_dropped =
                self.transport_counters.hop_limit_dropped.saturating_add(1);
            return;
        }
        let mut forwarded = packet.clone();
        forwarded.hops = forwarded.hops.saturating_add(1);
        forwarded.header_type = HeaderType::Type2;
        forwarded.propagation = crate::packet::Propagation::Transport;
        forwarded.transport = Some(self.identity.hash());
        if forwarded.encoded_len() > self.logical_mtu as usize {
            self.refused_payloads = self.refused_payloads.saturating_add(1);
            return;
        }
        self.schedule_rebroadcast(interface, forwarded, emitted, now);
    }
}
