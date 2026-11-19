//! Carrying traffic for other destinations: bridges, the reverse table and relaying. The
//! decisions are the shared ones in [`crate::transport`]; this file supplies the board's
//! inputs and carries them out with its actions and counters.

use heapless::Vec as BoundedVec;

use super::tables::{HashMemory, is_deduplicated_link_context};
use super::{
    Action, Actions, InterfaceId, LINK_TRANSPORT_IDLE, Node, REVERSE_TIMEOUT,
    TRANSPORT_DEDUP_HASHES, transit_proof_timeout,
};
use crate::hash::AddressHash;
use crate::link;
use crate::packet::{HeaderType, Packet, PacketType};
use crate::transport::{
    BridgeRecord, Carriage, Refusal, RelayRefusal, ReverseEntry, RouteTouch, Transit,
    admit_crossing, admit_request, cross_gate, crossed, find_live, forward_hop, record_reverse,
    relay_gate, restore_reverse, reverse_proof, unsent_request,
};
#[cfg(test)]
use crate::transport::{LinkBridge, record_bridge};

impl<const PEERS: usize, const ACTIONS: usize, const LINKS: usize, const ROUTES: usize>
    Node<PEERS, ACTIONS, LINKS, ROUTES>
{
    /// Whether this is a fresh packet for a shared-radio relay. Resource parts and keepalives
    /// legitimately repeat their hash, so they are never filtered, as RNS exempts them
    /// (`Transport.py` 1635-1640). Channel is filtered here, unlike RNS: see N10.
    pub(super) fn transit_is_new(&mut self, packet: &Packet, now: u64) -> bool {
        transit_hash_is_new(&mut self.transit_filter, packet.context, packet.hash(), now)
    }

    /// The board's carriage inputs: no ingress or egress selector.
    fn carriage(&self) -> Carriage {
        Carriage {
            on: self.transport.relay_packets,
            ingress: true,
            max_hops: self.transport.max_hops,
        }
    }

    fn count_hop_limit(&mut self) {
        self.transport_counters.hop_limit_dropped =
            self.transport_counters.hop_limit_dropped.saturating_add(1);
    }

    /// Record a link request as the carrying path would, for tests that set bridges up:
    /// unvalidated until `proof_deadline`, counting an eviction or a refusal. Returns whether
    /// it may be carried.
    #[cfg(test)]
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
        let carriage = self.carriage();
        // The proof validates the bridge only once it is on its way to the initiator.
        let proves = packet.packet_type == PacketType::Proof && packet.context == link::CTX_LRPROOF;
        let book = &self.book;
        let cross = match admit_crossing(
            &self.bridges,
            &packet.destination,
            interface,
            proves,
            now,
            self.transport.bridge_ttl,
            &carriage,
            |destination| {
                book.resolve(destination)
                    .map(|peer| link::proof_is_signed_by(packet, &peer.identity))
            },
        ) {
            Transit::Local | Transit::CarriageOff => return false,
            Transit::Refused(Refusal::Unvalidated) => {
                self.transport_counters.unvalidated_link_packets = self
                    .transport_counters
                    .unvalidated_link_packets
                    .saturating_add(1);
                return true;
            }
            // A third interface cannot extend this bridge's lifetime or poison the relay
            // de-duplication cache for a later packet from a real endpoint.
            Transit::Refused(_) => return true,
            Transit::Carry(cross) => cross,
        };
        let filter = &mut self.transit_filter;
        let gate = cross_gate(
            &carriage,
            packet.hops,
            cross.out,
            || transit_hash_is_new(filter, packet.context, packet.hash(), now),
            |_| true,
        );
        match gate {
            Ok(()) => {}
            Err(Refusal::HopLimit) => {
                self.count_hop_limit();
                return true;
            }
            Err(_) => return true,
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
            interface: cross.out,
            packet: forwarded,
        }) {
            match crossed(&mut self.bridges, &packet.destination, now, &cross) {
                RouteTouch::ProveAlive(destination) => self.prove_route_alive(destination, now),
                RouteTouch::MarkUsed(destination) => self.mark_route_used(destination, now),
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
        let carriage = self.carriage();
        let addressed = packet.header_type == HeaderType::Type2
            && packet.transport == Some(self.identity.hash())
            && packet.destination != self.destination();
        // One hash, taken when the memory is asked, serves it and the return path.
        let hashed = core::cell::Cell::new(None);
        let route_ttl = self.transport.route_ttl;
        let (filter, routes) = (&mut self.transit_filter, &self.routes);
        let decision = forward_hop(
            addressed,
            packet.hops,
            &carriage,
            || {
                let hash = packet.hash();
                hashed.set(Some(hash));
                transit_hash_is_new(filter, packet.context, hash, now)
            },
            || {
                find_live(routes, &packet.destination, |route| {
                    route.live(now, &super::routes::lifetimes(route_ttl))
                })
                .copied()
            },
            |_| true,
        );
        let route = match decision {
            Transit::Local | Transit::CarriageOff => return false,
            Transit::Refused(Refusal::HopLimit) => {
                self.count_hop_limit();
                return true;
            }
            Transit::Refused(Refusal::Unroutable) => {
                self.transport_counters.unroutable_packets =
                    self.transport_counters.unroutable_packets.saturating_add(1);
                return true;
            }
            Transit::Refused(_) => return true,
            Transit::Carry(route) => route,
        };
        let hash = hashed.get().unwrap_or_else(|| packet.hash());
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
        // A request records its bridge only when the send can fit, so a full action list
        // costs no bridge counter; one refused anyway is undone below (D36).
        let mut bridge = None;
        if packet.packet_type == PacketType::LinkRequest && actions.len() < ACTIONS {
            // A short request's link id covers all its bytes; its destination rejects it.
            let link_id = link::link_id(packet).unwrap_or(hash);
            let deadline = now
                .saturating_add(transit_proof_timeout(route.hops))
                .saturating_add(self.first_hop_airtime(route.iface));
            let (record, undo) = admit_request(
                &mut self.bridges,
                link_id,
                packet.destination,
                interface,
                route.iface,
                now,
                deadline,
                ROUTES,
                LINK_TRANSPORT_IDLE,
            );
            let evicted = match record {
                BridgeRecord::Refused => {
                    self.transport_counters.refused_bridges =
                        self.transport_counters.refused_bridges.saturating_add(1);
                    return true;
                }
                BridgeRecord::New { evicted } => evicted.is_some(),
                BridgeRecord::Known => false,
            };
            bridge = Some((link_id, undo, evicted));
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
            if let Some((_, _, true)) = bridge {
                // An eviction counts once the request that caused it is sent (D36).
                self.transport_counters.evicted_bridges =
                    self.transport_counters.evicted_bridges.saturating_add(1);
            }
            // A route carrying transit is in use (`Transport.py` 2387).
            self.mark_route_used(packet.destination, now);
            self.transport_counters.forwarded_packets =
                self.transport_counters.forwarded_packets.saturating_add(1);
        } else {
            if needs_reverse {
                restore_reverse(&mut self.reverse, &hash, displaced.into_iter().rev());
            }
            if let Some((link_id, undo, _)) = bridge {
                unsent_request(&mut self.bridges, &link_id, undo);
            }
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
        let carriage = self.carriage();
        let link_proof = matches!(packet.context, link::CTX_LRPROOF | link::CTX_RESOURCE_PRF);
        let received = match reverse_proof(
            &mut self.reverse,
            &packet.destination,
            interface,
            link_proof,
            packet.hops,
            now,
            REVERSE_TIMEOUT,
            &carriage,
            |_| true,
        ) {
            Transit::Local | Transit::CarriageOff => return false,
            Transit::Refused(Refusal::HopLimit) => {
                self.count_hop_limit();
                return true;
            }
            Transit::Refused(_) => return true,
            Transit::Carry(received) => received,
        };
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
    /// hop. It leaves from [`Node::poll`] on every known interface its mode rules permit.
    pub(super) fn relay_announce(
        &mut self,
        interface: InterfaceId,
        packet: &Packet,
        emitted: u64,
        now: u64,
    ) {
        // A path response answers one requester. RNS learns from it but never queues it for
        // rebroadcast, so one path request cannot flood the mesh. The board has no announce
        // rate rule.
        let path_response = packet.context == crate::path::CTX_PATH_RESPONSE;
        let enabled = self.transport.relay_announces && packet.destination != self.destination();
        let max_hops = self.transport.max_hops;
        let gate = relay_gate(
            path_response,
            || false,
            enabled,
            || self.transit_is_new(packet, now),
            packet.hops,
            max_hops,
        );
        match gate {
            Ok(()) => {}
            Err(RelayRefusal::HopLimit) => {
                self.count_hop_limit();
                return;
            }
            Err(_) => return,
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

/// Whether `hash` is new to the board's transit memory; a context exempt from it always is.
fn transit_hash_is_new(
    filter: &mut HashMemory<TRANSPORT_DEDUP_HASHES>,
    context: u8,
    hash: AddressHash,
    now: u64,
) -> bool {
    !is_deduplicated_link_context(context) || filter.insert(hash, now)
}
