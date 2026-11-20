//! Carrying others' traffic: link bridges, the reverse table, and forwarding.

use alloc::vec::Vec;
use core::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::hash::AddressHash;
use crate::link;
use crate::packet::{MTU, Packet, PacketType};

use crate::transport::{
    self, BridgeRecord, BridgeUndo, Carriage, Refusal, Transit, forward_hop, record_reverse,
};

use super::announces::duration_ticks;
use super::interface::{InterfaceId, QueueAdmission};
use super::queue::TrafficClass;
use super::routing::RoutingPolicy;
use super::shared::Shared;

/// How long a validated bridge is remembered after its last packet: far longer than a live
/// link goes quiet. An unproved one lapses at its proof deadline.
pub(super) const LINK_TRANSPORT_TTL: Duration = Duration::from_secs(3600);

/// How long a validated link may go unheard before a new request may take its place in a full
/// table: RNS's `Transport.LINK_TIMEOUT`, as [`crate::node::LINK_TRANSPORT_IDLE`].
pub(super) const LINK_TRANSPORT_IDLE: Duration =
    Duration::from_millis(crate::node::LINK_TRANSPORT_IDLE);

/// The most links a transport node carries at once. A request arriving at the bound displaces
/// the stalest link still awaiting its proof, or else a validated one unheard for
/// [`LINK_TRANSPORT_IDLE`]; a recently heard validated link is never displaced, so forged
/// requests cannot evict carried links or grow the table.
#[cfg(not(test))]
pub(super) const LINK_TRANSPORT_CAPACITY: usize = 4096;
#[cfg(test)]
pub(super) const LINK_TRANSPORT_CAPACITY: usize = 4;

/// A link carried through this node, on the desktop's clock.
pub(super) type LinkBridge = transport::LinkBridge<Instant>;

/// How long a carried packet's return path is kept for its proof (RNS
/// `Transport.REVERSE_TIMEOUT`).
pub(super) const REVERSE_TIMEOUT: Duration = Duration::from_secs(8 * 60);

/// Carried packets whose return path is remembered at once. At capacity the oldest goes.
#[cfg(not(test))]
pub(super) const REVERSE_TABLE_CAPACITY: usize = 4096;
#[cfg(test)]
pub(super) const REVERSE_TABLE_CAPACITY: usize = 4;

/// The way back for a carried packet's proof, on the desktop's clock.
pub(super) type ReverseEntry = transport::ReverseEntry<Instant>;

/// Make room for one more entry in a bounded table: drop what `expired` says has lapsed,
/// then, still full, the entry with the smallest `age`, which is returned.
pub(super) fn make_room<V, A: Ord>(
    table: &mut HashMap<AddressHash, V>,
    capacity: usize,
    expired: impl Fn(&V) -> bool,
    age: impl Fn(&V) -> A,
) -> Option<V> {
    if table.len() < capacity {
        return None;
    }
    table.retain(|_, entry| !expired(entry));
    if table.len() < capacity {
        return None;
    }
    let oldest = table.iter().min_by_key(|(_, entry)| age(entry))?.0;
    let oldest = *oldest;
    table.remove(&oldest)
}

impl Shared {
    /// Remember the way back for a carried packet's proof (RNS `Transport.py` 2104-2110).
    #[cfg(test)]
    pub(super) fn remember_reverse(
        &self,
        packet: AddressHash,
        received: InterfaceId,
        outbound: InterfaceId,
    ) {
        self.send_with_reverse(packet, received, outbound, || true);
    }

    /// Record before output admission and restore every displaced row on refusal.
    fn send_with_reverse(
        &self,
        packet: AddressHash,
        received: InterfaceId,
        outbound: InterfaceId,
        send: impl FnOnce() -> bool,
    ) -> bool {
        let now = Instant::now();
        let mut table = self.reverse_table.lock().unwrap();
        let mut displaced = Vec::new();
        let entry = ReverseEntry {
            received,
            outbound,
            at: now,
        };
        let timeout = duration_ticks(REVERSE_TIMEOUT);
        record_reverse(
            &mut *table,
            packet,
            entry,
            REVERSE_TABLE_CAPACITY,
            timeout,
            |gone| displaced.push(gone),
        );
        let accepted = send();
        if !accepted {
            transport::restore_reverse(&mut *table, &packet, displaced.into_iter().rev());
        }
        accepted
    }

    /// Consume the return path for a proof addressed to `packet`, yielding the interface to
    /// carry it out on if it arrived on the one the packet left by (RNS `Transport.py`
    /// 2733-2744).
    #[cfg(test)]
    pub(super) fn take_reverse(
        &self,
        packet: AddressHash,
        arrived: InterfaceId,
    ) -> Option<InterfaceId> {
        let mut table = self.reverse_table.lock().unwrap();
        let timeout = duration_ticks(REVERSE_TIMEOUT);
        transport::take_reverse(&mut *table, &packet, arrived, Instant::now(), timeout)
    }

    /// Whether `proof`, heard on `arrived`, goes back along its return path, and out which
    /// interface ([`transport::reverse_proof`]).
    pub(super) fn carry_back(
        &self,
        proof: &Packet,
        arrived: InterfaceId,
        policy: &RoutingPolicy,
    ) -> Transit<InterfaceId> {
        let mut table = self.reverse_table.lock().unwrap();
        transport::reverse_proof(
            &mut *table,
            &proof.destination,
            arrived,
            false,
            proof.hops,
            Instant::now(),
            duration_ticks(REVERSE_TIMEOUT),
            &carriage(policy, arrived),
            |out| policy.allowed_egress.allows(out),
        )
    }
}

impl Shared {
    /// Drop the return paths that run through `iface`.
    pub(super) fn forget_reverse_via(&self, iface: InterfaceId) {
        let mut table = self.reverse_table.lock().unwrap();
        transport::forget_interface(&mut *table, iface, |entry| [entry.received, entry.outbound]);
    }
}

/// The desktop's carriage inputs for a packet heard on `iface`.
pub(super) fn carriage(policy: &RoutingPolicy, iface: InterfaceId) -> Carriage {
    Carriage {
        on: policy.forward_packets,
        ingress: policy.allowed_ingress.allows(iface),
        max_hops: policy.max_hops,
    }
}

/// Count a transit refusal as the desktop always has (D23). A duplicate was counted by the
/// packet filter that found it.
pub(super) fn count_refusal(shared: &Shared, refusal: Refusal) {
    let stats = &shared.routing_stats;
    let counter = match refusal {
        Refusal::ThirdInterface | Refusal::Unvalidated | Refusal::Ingress | Refusal::Egress => {
            &stats.policy_rejected
        }
        Refusal::HopLimit => &stats.hop_limit_dropped,
        Refusal::Unroutable => &stats.unroutable_packets,
        Refusal::Duplicate => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Count a packet that would have been carried with carriage off (D35).
pub(super) fn count_carriage_off(shared: &Shared) {
    shared
        .routing_stats
        .policy_rejected
        .fetch_add(1, Ordering::Relaxed);
}

/// Forward a header-type-2 packet addressed to us as a transport hop (`addressed`), toward
/// its destination. `from` is the interface it arrived on. Hands the packet back when it is
/// not transit, for local handling.
pub(super) fn forward(
    shared: &Arc<Shared>,
    from: InterfaceId,
    pkt: Packet,
    policy: &RoutingPolicy,
    addressed: bool,
) -> Option<Packet> {
    // One hash, taken when the filter is asked, serves it and the reverse entry.
    let hashed = Cell::new(None);
    let decision = forward_hop(
        addressed,
        pkt.hops,
        &carriage(policy, from),
        || {
            let hash = pkt.hash();
            hashed.set(Some(hash));
            shared.packet_is_new(pkt.context, hash)
        },
        || shared.path_entry(pkt.destination),
        |out| policy.allowed_egress.allows(out),
    );
    let route = match decision {
        Transit::Local => return Some(pkt),
        Transit::CarriageOff => {
            count_carriage_off(shared);
            return Some(pkt);
        }
        Transit::Refused(refusal) => {
            count_refusal(shared, refusal);
            return None;
        }
        Transit::Carry(route) => route,
    };
    let hash = hashed.get().unwrap_or_else(|| pkt.hash());
    let (dest, out) = (pkt.destination, route.iface);
    // A link request establishes a bridge for its proof and later link data.
    let mut pkt = pkt;
    let mut admitted = None;
    if pkt.packet_type == PacketType::LinkRequest {
        // A short request's link id covers all its bytes; its destination rejects it.
        let link_id = link::link_id(&pkt).unwrap_or(hash);
        let Some(undo) = admit_link_request(shared, from, out, route.hops, link_id, &mut pkt)
        else {
            shared
                .routing_stats
                .policy_rejected
                .fetch_add(1, Ordering::Relaxed);
            return None;
        };
        admitted = Some((link_id, undo));
    }
    // Addressed by the route read once (inventory row b).
    let via = Address::Via(route.transport);
    let accepted = if pkt.packet_type == PacketType::LinkRequest {
        carry_on(shared, out, pkt, policy, via)
    } else {
        shared.send_with_reverse(hash, from, out, || carry_on(shared, out, pkt, policy, via))
    };
    if accepted {
        shared.mark_path_used(dest);
    } else if let Some((link_id, undo)) = admitted {
        // An unsent request leaves the bridges as they were (D36).
        transport::unsent_request(&mut *shared.link_transport.lock().unwrap(), &link_id, undo);
    }
    None
}

/// Prepare to carry a link request from `from` to `out`: lower its signalled MTU to what both
/// interfaces carry, and record an unproved bridge with a proof deadline of the per-hop
/// allowance for each hop still ahead plus the outbound first-hop airtime. Refuses a request
/// whose MTU must be lowered under a link mode this node cannot encode, or a full table of
/// recently heard validated links (`None`); otherwise returns what recording displaced.
fn admit_link_request(
    shared: &Shared,
    from: InterfaceId,
    out: InterfaceId,
    hops: u8,
    link_id: AddressHash,
    pkt: &mut Packet,
) -> Option<BridgeUndo<Instant, ()>> {
    let limit = [from, out]
        .into_iter()
        .filter_map(|iface| shared.link_mtu_on(iface))
        .min();
    // RNS drops the trailer toward an interface that does not signal its MTU (`Transport.py`
    // 2069-2076); a carrier under the protocol MTU still needs the clamp.
    if shared.signals_mtu(out) == Some(false) && limit.is_none_or(|l| l >= MTU as u32) {
        link::strip_request_mtu(pkt);
    } else if let Some(limit) = limit
        && link::clamp_request_mtu(pkt, limit).is_err()
    {
        return None;
    }
    // RNS adds the outbound interface's MTU airtime (`Transport.py` 2059-2062, 3200-3202).
    let allowance =
        crate::node::transit_proof_timeout(hops).saturating_add(shared.first_hop_airtime(out));
    let now = Instant::now();
    let mut bridges = shared.link_transport.lock().unwrap();
    // Prune before inserting, so the work tracks the requests that cause growth.
    let ttl = duration_ticks(LINK_TRANSPORT_TTL);
    transport::sweep_bridges(&mut *bridges, now, ttl, |_, _| {});
    let (record, undo) = transport::admit_request(
        &mut *bridges,
        link_id,
        pkt.destination,
        from,
        out,
        now,
        now + Duration::from_millis(allowance),
        LINK_TRANSPORT_CAPACITY,
        duration_ticks(LINK_TRANSPORT_IDLE),
    );
    (record != BridgeRecord::Refused).then_some(undo)
}

/// Re-address a forwarded packet for the interface it leaves on (stripping our transport
/// stamp, so `send_on` re-adds the next hop's), bump hops, and send. Transit's single egress
/// point, where egress permission and the forwarded count are enforced.
pub(super) fn forward_on(
    shared: &Arc<Shared>,
    out: InterfaceId,
    pkt: Packet,
    policy: &RoutingPolicy,
) -> bool {
    // Address without touching the route; the caller commits its use after acceptance.
    let via = shared
        .path_table
        .lock()
        .unwrap()
        .get(&pkt.destination)
        .and_then(|entry| entry.transport);
    carry_on(shared, out, pkt, policy, Address::Via(via))
}

/// Carry a reverse proof with its original header and transport id.
pub(super) fn forward_reverse_proof_on(
    shared: &Arc<Shared>,
    out: InterfaceId,
    pkt: Packet,
    policy: &RoutingPolicy,
) -> bool {
    carry_on(shared, out, pkt, policy, Address::Keep)
}

/// How a carried packet is addressed for the interface it leaves on.
#[derive(Clone, Copy)]
enum Address {
    /// Its original header and transport id, as a reverse proof keeps them.
    Keep,
    /// Header type 1, or 2 through this next transport hop.
    Via(Option<AddressHash>),
}

fn carry_on(
    shared: &Arc<Shared>,
    out: InterfaceId,
    mut pkt: Packet,
    policy: &RoutingPolicy,
    address: Address,
) -> bool {
    if !policy.allowed_egress.allows(out) {
        shared
            .routing_stats
            .policy_rejected
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if pkt.hops.saturating_add(1) >= policy.max_hops {
        shared
            .routing_stats
            .hop_limit_dropped
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }
    pkt.hops += 1;
    if let Address::Via(via) = address {
        pkt.header_type = crate::packet::HeaderType::Type1;
        pkt.transport = None;
        pkt.address_via(via);
    }
    let accepted = matches!(
        shared.push_to(out, pkt, TrafficClass::Transit),
        QueueAdmission::Queued
    );
    if accepted {
        shared
            .routing_stats
            .forwarded_packets
            .fetch_add(1, Ordering::Relaxed);
    }
    accepted
}
