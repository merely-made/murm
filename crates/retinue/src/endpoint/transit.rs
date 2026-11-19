//! Carrying others' traffic: link bridges, the reverse table, and forwarding.

use alloc::vec::Vec;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::address_book::AddressBook;
use crate::hash::AddressHash;
use crate::link;
use crate::packet::{MTU, Packet, PacketType};

use crate::transport::{self, BridgeRecord, record_bridge, record_reverse};

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

impl LinkBridge {
    /// Whether the bridge has lapsed at `now`, under [`LINK_TRANSPORT_TTL`].
    pub(super) fn lapsed_at(&self, now: Instant) -> bool {
        self.lapsed(now, duration_ticks(LINK_TRANSPORT_TTL))
    }

    /// Whether `pkt`, heard on `iface`, may cross: `None` if not, else whether it is the proof
    /// that validates the bridge once carried. Only the two sides may use it, nothing crosses
    /// before the proof, and the proof must come from the destination's side under its
    /// signature (the side alone when its identity is unknown).
    pub(super) fn admit(
        &self,
        iface: InterfaceId,
        pkt: &Packet,
        book: &Mutex<AddressBook>,
    ) -> Option<bool> {
        if iface != self.from && iface != self.out {
            return None;
        }
        if pkt.packet_type != PacketType::Proof || pkt.context != link::CTX_LRPROOF {
            return self.proof_deadline.is_none().then_some(false);
        }
        let signed = iface == self.out
            && book
                .lock()
                .unwrap()
                .resolve(self.destination)
                .is_none_or(|peer| link::proof_is_signed_by(pkt, &peer.identity));
        signed.then_some(true)
    }
}

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
    pub(super) fn take_reverse(
        &self,
        packet: AddressHash,
        arrived: InterfaceId,
    ) -> Option<InterfaceId> {
        let mut table = self.reverse_table.lock().unwrap();
        let timeout = duration_ticks(REVERSE_TIMEOUT);
        transport::take_reverse(&mut *table, &packet, arrived, Instant::now(), timeout)
    }
}

impl Shared {
    /// Drop the return paths that run through `iface`.
    pub(super) fn forget_reverse_via(&self, iface: InterfaceId) {
        let mut table = self.reverse_table.lock().unwrap();
        transport::forget_interface(&mut *table, iface, |entry| [entry.received, entry.outbound]);
    }
}

/// Forward a header-type-2 packet addressed to us as a transport hop, toward its
/// destination. `from` is the interface it arrived on.
pub(super) fn forward(
    shared: &Arc<Shared>,
    from: InterfaceId,
    pkt: Packet,
    policy: &RoutingPolicy,
) {
    if pkt.hops.saturating_add(1) >= policy.max_hops {
        shared
            .routing_stats
            .hop_limit_dropped
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    // A copy already carried is a loop or a second path, never new work.
    // One hash serves the packet filter and the reverse entry.
    let hash = pkt.hash();
    if !shared.packet_is_new(pkt.context, hash) {
        return;
    }
    let dest = pkt.destination;

    let next = shared.path_iface(dest);
    if let Some(out) = next {
        // Refuse before recording a bridge that policy would never carry.
        if !policy.allowed_egress.allows(out) {
            shared
                .routing_stats
                .policy_rejected
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        // A link request establishes a bridge for its proof and later link data.
        let mut pkt = pkt;
        let mut admitted = None;
        if pkt.packet_type == PacketType::LinkRequest {
            // A short request's link id covers all its bytes; its destination rejects it.
            let link_id = link::link_id(&pkt).unwrap_or(hash);
            match admit_link_request(shared, from, out, link_id, &mut pkt) {
                BridgeAdmission::Refused => {
                    shared
                        .routing_stats
                        .policy_rejected
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                BridgeAdmission::New => admitted = Some(link_id),
                BridgeAdmission::Known => {}
            }
        }
        let accepted = if pkt.packet_type == PacketType::LinkRequest {
            forward_on(shared, out, pkt, policy)
        } else {
            shared.send_with_reverse(hash, from, out, || forward_on(shared, out, pkt, policy))
        };
        if accepted {
            shared.mark_path_used(dest);
        } else if let Some(link_id) = admitted {
            // An unsent request must not hold its new bridge until the deadline.
            shared.link_transport.lock().unwrap().remove(&link_id);
        }
    } else {
        shared
            .routing_stats
            .unroutable_packets
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// What [`admit_link_request`] made of a request.
enum BridgeAdmission {
    /// Drop it.
    Refused,
    /// Carry it; its bridge is new.
    New,
    /// Carry it; it retransmits a request already bridged.
    Known,
}

/// Prepare to carry a link request from `from` to `out`: lower its signalled MTU to what both
/// interfaces carry, and record an unproved bridge with a proof deadline of the per-hop
/// allowance for each hop still ahead plus the outbound first-hop airtime. Refuses a request
/// whose MTU must be lowered under a link mode this node cannot encode, or a full table of
/// recently heard validated links.
fn admit_link_request(
    shared: &Shared,
    from: InterfaceId,
    out: InterfaceId,
    link_id: AddressHash,
    pkt: &mut Packet,
) -> BridgeAdmission {
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
        return BridgeAdmission::Refused;
    }
    let hops = shared
        .path_table
        .lock()
        .unwrap()
        .get(&pkt.destination)
        .map_or(0, |entry| entry.hops);
    // RNS adds the outbound interface's MTU airtime (`Transport.py` 2059-2062, 3200-3202).
    let allowance =
        crate::node::transit_proof_timeout(hops).saturating_add(shared.first_hop_airtime(out));
    let now = Instant::now();
    let bridge = LinkBridge {
        from,
        out,
        destination: pkt.destination,
        seen: now,
        proof_deadline: Some(now + Duration::from_millis(allowance)),
    };
    let mut bridges = shared.link_transport.lock().unwrap();
    // Prune before inserting, so the work tracks the requests that cause growth.
    let ttl = duration_ticks(LINK_TRANSPORT_TTL);
    transport::sweep_bridges(&mut *bridges, now, ttl, |_, _| {});
    let idle = duration_ticks(LINK_TRANSPORT_IDLE);
    match record_bridge(
        &mut *bridges,
        link_id,
        bridge,
        LINK_TRANSPORT_CAPACITY,
        idle,
    ) {
        BridgeRecord::Known => BridgeAdmission::Known,
        BridgeRecord::New { .. } => BridgeAdmission::New,
        BridgeRecord::Refused => BridgeAdmission::Refused,
    }
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
    carry_on(shared, out, pkt, policy, true)
}

/// Carry a reverse proof with its original header and transport id.
pub(super) fn forward_reverse_proof_on(
    shared: &Arc<Shared>,
    out: InterfaceId,
    pkt: Packet,
    policy: &RoutingPolicy,
) -> bool {
    carry_on(shared, out, pkt, policy, false)
}

fn carry_on(
    shared: &Arc<Shared>,
    out: InterfaceId,
    mut pkt: Packet,
    policy: &RoutingPolicy,
    readdress: bool,
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
    if readdress {
        pkt.header_type = crate::packet::HeaderType::Type1;
        pkt.transport = None;
        // Address without touching the route; the caller commits its use after acceptance.
        let via = shared
            .path_table
            .lock()
            .unwrap()
            .get(&pkt.destination)
            .and_then(|entry| entry.transport);
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
