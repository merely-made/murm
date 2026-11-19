//! The forwarding decisions both drivers make: crossing a link bridge, carrying a
//! header-type-2 packet and the link request it may bridge, carrying a proof back, and the
//! announce relay gate at receipt and at release.
//!
//! Each returns a decision with its reason; the driver supplies the inputs (carriage, its
//! selectors, limits, `now`) and carries the decision out with its own locks, queues and
//! counters (D23). A step that touches a table the decision does not hold comes in as a
//! closure, so the desktop never holds two of its table locks at once.

#![allow(dead_code, reason = "the drivers move onto these in T3 B2 and B3")]

use super::{
    BridgeRecord, InterfaceId, LinkBridge, Lookup, ReverseEntry, RouteEntry, RouteLife, Store,
    Tick, find_live, find_or_forget, record_bridge, refresh_bridge, take_reverse,
};
use crate::hash::AddressHash;
use crate::iface_mode::{InterfaceMode, ModeFlags, announce_permitted};

/// The carriage policy a driver holds for one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Carriage {
    /// Whether this node carries others' packets at all.
    pub(crate) on: bool,
    /// Whether the interface the packet arrived on may bring transit (the desktop's ingress
    /// selector; always on the board).
    pub(crate) ingress: bool,
    /// The hop ceiling: a packet goes on only while its forwarded hop count stays below it.
    pub(crate) max_hops: u8,
}

impl Carriage {
    fn below_ceiling(&self, hops: u8) -> bool {
        hops.saturating_add(1) < self.max_hops
    }
}

/// Why a transit packet is not carried. Each driver counts these as it does today (D23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// It arrived on neither side of its bridge.
    ThirdInterface,
    /// Link traffic ahead of the destination's proof, or a proof not from the destination.
    Unvalidated,
    /// The ingress selector refused its interface.
    Ingress,
    /// The egress selector refused the interface it would leave by.
    Egress,
    /// Forwarded, it would reach the hop ceiling.
    HopLimit,
    /// It was carried already.
    Duplicate,
    /// Its destination has no live route.
    Unroutable,
}

/// What to do with a packet that may be transit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transit<C> {
    /// Not transit: local handling.
    Local,
    /// It would be transit, but carriage is off: local handling, tables untouched (D24).
    CarriageOff,
    /// Transit, dropped.
    Refused(Refusal),
    /// Transit, to be carried as `C` says.
    Carry(C),
}

/// What a bridge lets cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Crossing {
    /// A packet of a validated link.
    Traffic,
    /// The destination's proof, which validates the bridge once carried; `verified` when its
    /// signature was checked against the destination's identity.
    Proof { verified: bool },
}

/// A packet admitted across a bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cross {
    /// The side it leaves by.
    pub(crate) out: InterfaceId,
    /// The destination the link request named.
    pub(crate) destination: AddressHash,
    pub(crate) crossing: Crossing,
}

/// How a carried packet touches its destination's route once sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteTouch {
    /// A verified link proof proves the destination alive (`Transport.py` 2928-2938).
    ProveAlive(AddressHash),
    /// The route is in use (`Transport.py` 2387, 2440).
    MarkUsed(AddressHash),
}

/// Whether a packet for `link_id`, heard on `arrived`, crosses a live bridge, and as what.
/// Only the two sides may use it, nothing crosses before the proof, and the proof must come
/// from the destination's side under its signature; `signed` checks it against the
/// destination's identity, `None` when that is unknown, and the side check stands alone.
/// `proves` marks a link proof.
#[allow(clippy::too_many_arguments, reason = "the driver's inputs, each named")]
pub(crate) fn admit_crossing<T: Tick, S: Store<LinkBridge<T>>>(
    bridges: &S,
    link_id: &AddressHash,
    arrived: InterfaceId,
    proves: bool,
    now: T,
    ttl: u64,
    carriage: &Carriage,
    signed: impl FnOnce(AddressHash) -> Option<bool>,
) -> Transit<Cross> {
    let Some(bridge) = find_live(bridges, link_id, |bridge| !bridge.lapsed(now, ttl)) else {
        return Transit::Local;
    };
    if !carriage.on {
        return Transit::CarriageOff;
    }
    let out = if arrived == bridge.from {
        bridge.out
    } else if arrived == bridge.out {
        bridge.from
    } else {
        return Transit::Refused(Refusal::ThirdInterface);
    };
    let crossing = if proves {
        let checked = signed(bridge.destination);
        if arrived != bridge.out || checked == Some(false) {
            return Transit::Refused(Refusal::Unvalidated);
        }
        Crossing::Proof {
            verified: checked == Some(true),
        }
    } else if bridge.proof_deadline.is_some() {
        return Transit::Refused(Refusal::Unvalidated);
    } else {
        Crossing::Traffic
    };
    Transit::Carry(Cross {
        out,
        destination: bridge.destination,
        crossing,
    })
}

/// The gate an admitted crossing passes before it is sent: the ingress selector, the
/// duplicate memory while below the ceiling, the egress selector, then the ceiling.
pub(crate) fn cross_gate(
    carriage: &Carriage,
    hops: u8,
    out: InterfaceId,
    is_new: impl FnOnce() -> bool,
    egress: impl FnOnce(InterfaceId) -> bool,
) -> Result<(), Refusal> {
    if !carriage.ingress {
        return Err(Refusal::Ingress);
    }
    if carriage.below_ceiling(hops) && !is_new() {
        return Err(Refusal::Duplicate);
    }
    if !egress(out) {
        return Err(Refusal::Egress);
    }
    if !carriage.below_ceiling(hops) {
        return Err(Refusal::HopLimit);
    }
    Ok(())
}

/// A crossing was sent at `now`: refresh its bridge, validating it on a proof, and say how
/// its destination's route is touched.
pub(crate) fn crossed<T: Tick, S: Store<LinkBridge<T>>>(
    bridges: &mut S,
    link_id: &AddressHash,
    now: T,
    cross: &Cross,
) -> RouteTouch {
    refresh_bridge(bridges, link_id, now, cross.crossing != Crossing::Traffic);
    match cross.crossing {
        Crossing::Proof { verified: true } => RouteTouch::ProveAlive(cross.destination),
        _ => RouteTouch::MarkUsed(cross.destination),
    }
}

/// Whether a packet that names this node as its transport hop, `addressed` (header type 2,
/// our transport id, not one of our destinations), is carried, and by which route: carriage,
/// the ingress selector, the ceiling, the duplicate memory, a live route read once, then the
/// egress selector. A packet is remembered before its route is looked up (D5).
pub(crate) fn forward_hop<T: Tick>(
    addressed: bool,
    hops: u8,
    carriage: &Carriage,
    is_new: impl FnOnce() -> bool,
    route: impl FnOnce() -> Option<RouteEntry<T>>,
    egress: impl FnOnce(InterfaceId) -> bool,
) -> Transit<RouteEntry<T>> {
    if !addressed {
        return Transit::Local;
    }
    if !carriage.on {
        return Transit::CarriageOff;
    }
    if !carriage.ingress {
        return Transit::Refused(Refusal::Ingress);
    }
    if !carriage.below_ceiling(hops) {
        return Transit::Refused(Refusal::HopLimit);
    }
    if !is_new() {
        return Transit::Refused(Refusal::Duplicate);
    }
    let Some(route) = route() else {
        return Transit::Refused(Refusal::Unroutable);
    };
    if !egress(route.iface) {
        return Transit::Refused(Refusal::Egress);
    }
    Transit::Carry(route)
}

/// A destination's route if live at `now`, a lapsed one removed, read once per carried packet.
pub(crate) fn live_route<T: Tick, S: Store<RouteEntry<T>>>(
    routes: &mut S,
    destination: &AddressHash,
    now: T,
    life: impl RouteLife,
) -> Lookup<RouteEntry<T>> {
    find_or_forget(routes, destination, |route| route.live(now, &life))
}

/// Record the unproved bridge for a link request about to be carried by `route` from `from`,
/// lapsing at `deadline` without the proof (the per-hop allowance for each hop ahead plus the
/// outgoing interface's first-hop airtime). What it decided goes to [`unsent_request`] if the
/// request is then not sent.
#[allow(clippy::too_many_arguments, reason = "the driver's inputs, each named")]
pub(crate) fn admit_request<T: Tick, S: Store<LinkBridge<T>>>(
    bridges: &mut S,
    link_id: AddressHash,
    destination: AddressHash,
    from: InterfaceId,
    route: &RouteEntry<T>,
    now: T,
    deadline: T,
    capacity: usize,
    idle: u64,
) -> BridgeRecord {
    let bridge = LinkBridge {
        from,
        out: route.iface,
        destination,
        seen: now,
        proof_deadline: Some(deadline),
    };
    record_bridge(bridges, link_id, bridge, capacity, idle)
}

/// A link request admitted as `record` was not sent: a new bridge goes (inventory row c).
pub(crate) fn unsent_request<T, S: Store<LinkBridge<T>>>(
    bridges: &mut S,
    link_id: &AddressHash,
    record: BridgeRecord,
) {
    if matches!(record, BridgeRecord::New { .. }) {
        bridges.remove(link_id);
    }
}

/// Whether a proof for `key`, heard on `arrived`, is carried back along its return path, and
/// out which interface. `link_proof` marks a link or resource proof, which never is. With
/// carriage on the path is consumed either way (`Transport.py` 2733-2744); with it off the
/// path is kept (`3009`). Then the ingress selector, the egress selector and the ceiling.
#[allow(clippy::too_many_arguments, reason = "the driver's inputs, each named")]
pub(crate) fn reverse_proof<T: Tick, S: Store<ReverseEntry<T>>>(
    reverse: &mut S,
    key: &AddressHash,
    arrived: InterfaceId,
    link_proof: bool,
    hops: u8,
    now: T,
    timeout: u64,
    carriage: &Carriage,
    egress: impl FnOnce(InterfaceId) -> bool,
) -> Transit<InterfaceId> {
    if link_proof {
        return Transit::Local;
    }
    if !carriage.on {
        let usable = find_live(reverse, key, |entry| {
            !entry.expired(now, timeout) && entry.outbound == arrived
        });
        return if usable.is_some() {
            Transit::CarriageOff
        } else {
            Transit::Local
        };
    }
    let Some(back) = take_reverse(reverse, key, arrived, now, timeout) else {
        return Transit::Local;
    };
    if !carriage.ingress {
        return Transit::Refused(Refusal::Ingress);
    }
    if !egress(back) {
        return Transit::Refused(Refusal::Egress);
    }
    if !carriage.below_ceiling(hops) {
        return Transit::Refused(Refusal::HopLimit);
    }
    Transit::Carry(back)
}

/// Why a verified announce is not scheduled for relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelayRefusal {
    /// A path response answers one requester and is never relayed.
    PathResponse,
    /// This node does not relay it: relaying is off, the ingress selector refuses its
    /// interface, or it is our own.
    Disabled,
    /// Its hash is remembered.
    Duplicate,
    /// Its destination is over the announce rate.
    RateLimited,
    /// Relayed, it would reach the hop ceiling.
    HopLimit,
}

/// The announce relay gate at receipt, in the desktop's order (D26): a path response stops
/// first; the rate is observed for every other announce, relayed or not (`Transport.py`
/// 2299-2333); then relaying, the duplicate memory, the rate verdict and the ceiling.
pub(crate) fn relay_gate(
    path_response: bool,
    rate_blocked: impl FnOnce() -> bool,
    enabled: bool,
    is_new: impl FnOnce() -> bool,
    hops: u8,
    max_hops: u8,
) -> Result<(), RelayRefusal> {
    if path_response {
        return Err(RelayRefusal::PathResponse);
    }
    let blocked = rate_blocked();
    if !enabled {
        return Err(RelayRefusal::Disabled);
    }
    if !is_new() {
        return Err(RelayRefusal::Duplicate);
    }
    if blocked {
        return Err(RelayRefusal::RateLimited);
    }
    if hops.saturating_add(1) >= max_hops {
        return Err(RelayRefusal::HopLimit);
    }
    Ok(())
}

/// The interface a due relay's destination is reached by at release, if its route is live
/// (D3: without one the relay is refused).
pub(crate) fn release_from<T: Tick, S: Store<RouteEntry<T>>>(
    routes: &S,
    destination: &AddressHash,
    now: T,
    life: impl RouteLife,
) -> Option<InterfaceId> {
    find_live(routes, destination, |route| route.live(now, &life)).map(|route| route.iface)
}

/// The interfaces a due relay goes to at release: each of `interfaces` (with its mode and
/// switches) that the mode rule permits for a route learned in `from` mode, limited to `only`
/// when an earlier release still owes some (D2, D22). `from` is `None` without a live route,
/// and then none is permitted.
pub(crate) fn relay_targets<'a>(
    from: Option<InterfaceMode>,
    interfaces: impl IntoIterator<Item = (InterfaceId, InterfaceMode, ModeFlags)> + 'a,
    only: Option<&'a [InterfaceId]>,
) -> impl Iterator<Item = InterfaceId> + 'a {
    interfaces
        .into_iter()
        .filter(move |(id, mode, flags)| {
            only.is_none_or(|only| only.contains(id))
                && announce_permitted(*mode, from, false, *flags)
        })
        .map(|(id, _, _)| id)
}
