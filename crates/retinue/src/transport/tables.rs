//! The operations both drivers perform on one table. Capacities and lifetimes are the
//! caller's, in milliseconds; what an operation removed comes back so the caller can count it.

use super::{InterfaceId, LinkBridge, ReverseEntry, RouteEntry, Store, Tick};
use crate::hash::AddressHash;
use crate::iface_mode::InterfaceMode;

/// Why an entry left a table while another was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    /// It had outlived its lifetime.
    Expired,
    /// It was live, and the least ranked in a full table.
    Evicted,
}

/// What a keyed lookup found.
#[cfg_attr(
    not(feature = "tokio"),
    allow(dead_code, reason = "used by the endpoint")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lookup<V> {
    Live(V),
    /// An entry past its lifetime, now removed.
    Expired(V),
    Missing,
}

/// The entry for `key` if `live` accepts it. Nothing is removed.
pub(crate) fn find_live<'a, V, S: Store<V>>(
    store: &'a S,
    key: &AddressHash,
    live: impl FnOnce(&V) -> bool,
) -> Option<&'a V> {
    store.get(key).filter(|entry| live(entry))
}

/// The entry for `key` if `live` accepts it; one it refuses is removed and reported.
#[cfg_attr(
    not(feature = "tokio"),
    allow(dead_code, reason = "used by the endpoint")
)]
pub(crate) fn find_or_forget<V: Copy, S: Store<V>>(
    store: &mut S,
    key: &AddressHash,
    live: impl FnOnce(&V) -> bool,
) -> Lookup<V> {
    match store.get(key).copied() {
        Some(entry) if live(&entry) => Lookup::Live(entry),
        Some(entry) => {
            store.remove(key);
            Lookup::Expired(entry)
        }
        None => Lookup::Missing,
    }
}

/// Drop every entry held on `iface`, keeping the order of the rest.
pub(crate) fn forget_interface<V, S: Store<V>>(
    store: &mut S,
    iface: InterfaceId,
    on: impl Fn(&V) -> [InterfaceId; 2],
) {
    store.retain(|_, entry| !on(entry).contains(&iface), |_, _, _| {});
}

/// Record a route, replacing any for `key`. A live route it replaces hands on its used mark
/// (RNS `Transport.py` 2510, 2726); a new one starts as `route` has it. A new key in a full
/// table first makes room ([`make_route_room`]). Returns the replaced route.
pub(crate) fn record_route<T: Tick, S: Store<RouteEntry<T>>>(
    store: &mut S,
    key: AddressHash,
    mut route: RouteEntry<T>,
    capacity: usize,
    lifetime: impl Fn(InterfaceMode) -> u64,
    mut removed: impl FnMut(AddressHash, RouteEntry<T>, Removal),
) -> Option<RouteEntry<T>> {
    let now = route.learned;
    if let Some(old) = store.get(&key).filter(|old| old.live(now, &lifetime)) {
        route.used = old.used;
    }
    if store.get(&key).is_none() {
        make_route_room(store, now, capacity, &lifetime, &mut removed);
    }
    store.insert(key, route).ok().flatten()
}

/// Which route a full table gives up first: one never used, the soonest to expire first,
/// then the one used longest ago (D30). `left` is in whole milliseconds, so `learned` breaks
/// its ties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Standing<T> {
    NeverUsed { left: u64, learned: T },
    Used(T),
}

/// Make room for one more route in a full table at `now`: drop the routes past their
/// lifetime, then, still full, the route that stands lowest ([`Standing`]). Each route
/// taken out goes to `removed`.
pub(crate) fn make_route_room<T: Tick, S: Store<RouteEntry<T>>>(
    store: &mut S,
    now: T,
    capacity: usize,
    lifetime: impl Fn(InterfaceMode) -> u64,
    mut removed: impl FnMut(AddressHash, RouteEntry<T>, Removal),
) {
    if store.len() < capacity {
        return;
    }
    // The dead first: a table full of expired routes must never evict a live one.
    store.retain(
        |_, entry| entry.live(now, &lifetime),
        |gone, entry, _| removed(gone, entry, Removal::Expired),
    );
    let standing = |entry: &RouteEntry<T>| {
        Some(entry.used.map_or_else(
            || Standing::NeverUsed {
                left: lifetime(entry.mode).saturating_sub(now.millis_since(entry.learned)),
                learned: entry.learned,
            },
            Standing::Used,
        ))
    };
    if store.len() >= capacity
        && let Some(lowest) = store.least(standing)
        && let Some((entry, _)) = store.remove(&lowest)
    {
        removed(lowest, entry, Removal::Evicted);
    }
}

/// Mark a live route used at `now`. Its life does not move: RNS sets only the path's used
/// mark when it sends or carries by one (`Transport.py` 1618, 2387, 2440). Returns whether
/// it was marked.
pub(crate) fn mark_route_used<T: Tick, S: Store<RouteEntry<T>>>(
    store: &mut S,
    key: &AddressHash,
    now: T,
    lifetime: impl Fn(InterfaceMode) -> u64,
) -> bool {
    match store.get_mut(key) {
        Some(entry) if entry.live(now, lifetime) => {
            entry.used = Some(now);
            true
        }
        _ => false,
    }
}

/// Prove a live route alive at `now`, as RNS does only on events that cryptographically
/// prove the destination's recent liveness (`Transport.py` 3632-3641): its life starts over,
/// in `mode` of its interface now, and it is marked used. Returns whether it was proved.
pub(crate) fn prove_route_alive<T: Tick, S: Store<RouteEntry<T>>>(
    store: &mut S,
    key: &AddressHash,
    now: T,
    mode: impl FnOnce(InterfaceId) -> InterfaceMode,
    lifetime: impl Fn(InterfaceMode) -> u64,
) -> bool {
    match store.get_mut(key) {
        Some(entry) if entry.live(now, lifetime) => {
            entry.learned = now;
            entry.used = Some(now);
            entry.mode = mode(entry.iface);
            true
        }
        _ => false,
    }
}

/// Remove the routes past their lifetime at `now`, handing each to `expired`.
pub(crate) fn sweep_routes<T: Tick, S: Store<RouteEntry<T>>>(
    store: &mut S,
    now: T,
    lifetime: impl Fn(InterfaceMode) -> u64,
    expired: impl FnMut(AddressHash, RouteEntry<T>),
) {
    store.sweep(|entry| !entry.live(now, &lifetime), expired);
}

/// What [`record_bridge`] made of a link request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BridgeRecord {
    /// A retransmitted request for a bridge already held. An unproved bridge was replaced
    /// by the new one; a validated one stands.
    Known,
    /// A new bridge, which took the slot of `evicted` if the table was full.
    New { evicted: Option<AddressHash> },
    /// The table is full of recently heard validated links.
    Refused,
}

/// Record an unproved bridge for a link request about to be carried. In a full table the
/// stalest unproved bridge makes room, or failing one, a bridge unheard for `idle`
/// milliseconds. A recently heard validated link is never displaced, so forged requests
/// cannot evict carried links. Lapsed bridges are not pruned here: that is the caller's sweep.
pub(crate) fn record_bridge<T: Tick, S: Store<LinkBridge<T>>>(
    store: &mut S,
    link_id: AddressHash,
    bridge: LinkBridge<T>,
    capacity: usize,
    idle: u64,
) -> BridgeRecord {
    if let Some(existing) = store.get_mut(&link_id) {
        if existing.proof_deadline.is_some() {
            *existing = bridge;
        }
        return BridgeRecord::Known;
    }
    let now = bridge.seen;
    let mut evicted = None;
    if store.len() >= capacity {
        let Some(stalest) = store
            .least(|entry| entry.proof_deadline.is_some().then_some(entry.seen))
            .or_else(|| store.least(|entry| entry.idle(now, idle).then_some(entry.seen)))
        else {
            return BridgeRecord::Refused;
        };
        store.remove(&stalest);
        evicted = Some(stalest);
    }
    match store.insert(link_id, bridge) {
        Ok(_) => BridgeRecord::New { evicted },
        Err(_) => BridgeRecord::Refused,
    }
}

/// Note that a packet crossed a bridge at `now`; a carried proof validates it. Returns
/// whether the bridge was held.
pub(crate) fn refresh_bridge<T: Tick, S: Store<LinkBridge<T>>>(
    store: &mut S,
    link_id: &AddressHash,
    now: T,
    proves: bool,
) -> bool {
    let Some(bridge) = store.get_mut(link_id) else {
        return false;
    };
    if proves {
        bridge.proof_deadline = None;
    }
    bridge.seen = now;
    true
}

/// Remove the bridges lapsed at `now`, handing each to `lapsed`.
pub(crate) fn sweep_bridges<T: Tick, S: Store<LinkBridge<T>>>(
    store: &mut S,
    now: T,
    ttl: u64,
    lapsed: impl FnMut(AddressHash, LinkBridge<T>),
) {
    store.sweep(|bridge| bridge.lapsed(now, ttl), lapsed);
}

/// A return path that [`record_reverse`] took out of the table, with where it stood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Displaced<T, P> {
    pub(crate) key: AddressHash,
    pub(crate) entry: ReverseEntry<T>,
    pub(crate) place: P,
}

/// Record the way back for a carried packet's proof (RNS `Transport.py` 2104-2110),
/// replacing any for `key`. A new key in a full table first drops the expired paths, then
/// the oldest. Everything taken out goes to `displaced`, in order, so a refused send can
/// [`restore_reverse`] the table exactly (D15).
pub(crate) fn record_reverse<T: Tick, S: Store<ReverseEntry<T>>>(
    store: &mut S,
    key: AddressHash,
    entry: ReverseEntry<T>,
    capacity: usize,
    timeout: u64,
    mut displaced: impl FnMut(Displaced<T, S::Place>),
) {
    let now = entry.at;
    if let Some((old, place)) = store.remove(&key) {
        displaced(Displaced {
            key,
            entry: old,
            place,
        });
    } else if store.len() >= capacity {
        store.retain(
            |_, old| !old.expired(now, timeout),
            |key, entry, place| displaced(Displaced { key, entry, place }),
        );
        if store.len() >= capacity
            && let Some(oldest) = store.least(|old| Some(old.at))
            && let Some((entry, place)) = store.remove(&oldest)
        {
            displaced(Displaced {
                key: oldest,
                entry,
                place,
            });
        }
    }
    let _ = store.insert(key, entry);
}

/// Undo a [`record_reverse`] of `key`: remove the new path and put back what it displaced.
/// `displaced` runs newest first, the reverse of the order it was handed out in.
pub(crate) fn restore_reverse<T, S: Store<ReverseEntry<T>>>(
    store: &mut S,
    key: &AddressHash,
    displaced: impl IntoIterator<Item = Displaced<T, S::Place>>,
) {
    store.remove(key);
    for old in displaced {
        store.restore(old.key, old.entry, old.place);
    }
}

/// Consume the return path for a proof addressed to `key`, yielding the interface to carry
/// it out on if it is unexpired and the proof `arrived` on the one the packet left by (RNS
/// `Transport.py` 2733-2744). The path is consumed either way.
pub(crate) fn take_reverse<T: Tick, S: Store<ReverseEntry<T>>>(
    store: &mut S,
    key: &AddressHash,
    arrived: InterfaceId,
    now: T,
    timeout: u64,
) -> Option<InterfaceId> {
    let (entry, _) = store.remove(key)?;
    (!entry.expired(now, timeout) && entry.outbound == arrived).then_some(entry.received)
}

/// Remove the return paths expired at `now`, keeping the order of the rest.
pub(crate) fn sweep_reverse<T: Tick, S: Store<ReverseEntry<T>>>(
    store: &mut S,
    now: T,
    timeout: u64,
) {
    store.retain(|_, entry| !entry.expired(now, timeout), |_, _, _| {});
}
