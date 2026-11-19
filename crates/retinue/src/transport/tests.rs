//! Every table operation against both containers, with the same assertions: each body is
//! generic over the store and instantiated once per container.

use core::fmt::Debug;
use std::vec::Vec;

use heapless::Vec as BoundedVec;

use super::*;
use crate::iface_mode::InterfaceMode;

mod memory;

const CAP: usize = 4;
const TTL: u64 = 100;
const IDLE: u64 = 50;
const TIMEOUT: u64 = 80;

/// Route lifetimes by mode: roaming routes live a third as long.
fn ttl(mode: InterfaceMode) -> u64 {
    if mode == InterfaceMode::Roaming {
        TTL / 3
    } else {
        TTL
    }
}

fn key(n: u8) -> AddressHash {
    AddressHash::from_bytes([n; 16])
}

fn route(iface: InterfaceId, learned: u64) -> RouteEntry<u64> {
    RouteEntry {
        iface,
        transport: None,
        hops: 1,
        learned,
        mode: InterfaceMode::Full,
        used: None,
    }
}

fn bridge(seen: u64, proof_deadline: Option<u64>) -> LinkBridge<u64> {
    LinkBridge {
        from: 1,
        out: 2,
        destination: key(0xEE),
        seen,
        proof_deadline,
    }
}

fn reverse(at: u64) -> ReverseEntry<u64> {
    ReverseEntry {
        received: 1,
        outbound: 2,
        at,
    }
}

/// A store the generic bodies can make, copy and compare.
trait TestStore<V>: Store<V> + Default + Clone + PartialEq + Debug {}
impl<V, S: Store<V> + Default + Clone + PartialEq + Debug> TestStore<V> for S {}

/// Run a generic body against the bounded array and, with `std`, the map.
macro_rules! both {
    ($name:ident, $entry:ty) => {
        mod $name {
            use super::*;

            #[test]
            fn array() {
                super::$name::<BoundedVec<Keyed<$entry>, CAP>>();
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn map() {
                super::$name::<std::collections::HashMap<AddressHash, $entry>>();
            }
        }
    };
}

fn filled<V: Copy, S: TestStore<V>>(make: impl Fn(u8) -> V) -> S {
    let mut store = S::default();
    for n in 1..=CAP as u8 {
        assert!(matches!(store.insert(key(n), make(n)), Ok(None)));
    }
    store
}

both!(routes_replace_then_evict_the_stalest, RouteEntry<u64>);
fn routes_replace_then_evict_the_stalest<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(1, u64::from(n)));
    let mut gone = Vec::new();
    let replaced = record_route(&mut store, key(2), route(3, 10), CAP, ttl, |k, _, why| {
        gone.push((k, why));
    });
    assert_eq!(replaced, Some(route(1, 2)), "a known key is replaced");
    assert!(gone.is_empty(), "replacing makes no room");
    record_route(&mut store, key(9), route(1, 11), CAP, ttl, |k, _, why| {
        gone.push((k, why));
    });
    assert_eq!(
        gone,
        [(key(1), Removal::Evicted)],
        "the stalest live route goes"
    );
    assert_eq!(store.len(), CAP);
    assert_eq!(store.get(&key(2)).map(|r| r.iface), Some(3));
}

both!(routes_full_of_dead_never_evict_the_living, RouteEntry<u64>);
fn routes_full_of_dead_never_evict_the_living<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(1, if n == 3 { 150 } else { 0 }));
    let mut gone = Vec::new();
    record_route(&mut store, key(9), route(1, 160), CAP, ttl, |k, _, why| {
        gone.push((k, why));
    });
    gone.sort_by_key(|(k, _)| *k);
    let expired = [1, 2, 4].map(|n| (key(n), Removal::Expired));
    assert_eq!(gone, expired);
    assert_eq!(store.len(), 2);
    assert!(store.get(&key(3)).is_some());
}

both!(route_lookups_check_age, RouteEntry<u64>);
fn route_lookups_check_age<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(1, u64::from(n) * 10));
    let live = |now| move |r: &RouteEntry<u64>| r.live(now, ttl);
    assert_eq!(find_live(&store, &key(1), live(109)), Some(&route(1, 10)));
    assert_eq!(
        find_live(&store, &key(1), live(110)),
        None,
        "past its lifetime"
    );
    assert_eq!(store.len(), CAP, "a read-only lookup removes nothing");
    assert_eq!(
        find_or_forget(&mut store, &key(1), live(110)),
        Lookup::Expired(route(1, 10))
    );
    assert_eq!(
        find_or_forget(&mut store, &key(1), live(110)),
        Lookup::Missing
    );
    assert_eq!(
        find_or_forget(&mut store, &key(2), live(110)),
        Lookup::Live(route(1, 20))
    );
    assert_eq!(store.len(), CAP - 1);
}

both!(routes_are_marked_used_only_when_live, RouteEntry<u64>);
fn routes_are_marked_used_only_when_live<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(1, u64::from(n) * 10));
    assert!(mark_route_used(&mut store, &key(2), 50, ttl));
    assert_eq!(store.get(&key(2)).map(|r| r.used), Some(Some(50)));
    assert_eq!(
        store.get(&key(2)).map(|r| r.learned),
        Some(20),
        "use does not extend a route's life"
    );
    assert!(
        !mark_route_used(&mut store, &key(1), 110, ttl),
        "a dead route stays dead"
    );
    assert_eq!(store.get(&key(1)).map(|r| r.used), Some(None));
    assert!(!mark_route_used(&mut store, &key(9), 50, ttl));
}

both!(proving_a_route_alive_starts_its_life_over, RouteEntry<u64>);
fn proving_a_route_alive_starts_its_life_over<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(u32::from(n), u64::from(n) * 10));
    let mode = |iface| {
        if iface == 2 {
            InterfaceMode::Roaming
        } else {
            InterfaceMode::Full
        }
    };
    assert!(prove_route_alive(&mut store, &key(2), 50, mode, ttl));
    let proved = RouteEntry {
        used: Some(50),
        mode: InterfaceMode::Roaming,
        ..route(2, 50)
    };
    assert_eq!(
        store.get(&key(2)),
        Some(&proved),
        "in its interface's mode now"
    );
    assert!(
        !prove_route_alive(&mut store, &key(1), 110, mode, ttl),
        "a dead route stays dead"
    );
    assert_eq!(store.get(&key(1)), Some(&route(1, 10)));
    assert!(!prove_route_alive(&mut store, &key(9), 50, mode, ttl));
}

both!(replacing_a_route_keeps_its_used_mark, RouteEntry<u64>);
fn replacing_a_route_keeps_its_used_mark<S: TestStore<RouteEntry<u64>>>() {
    let used = |n| RouteEntry {
        used: Some(n),
        ..route(1, 0)
    };
    let mut store = S::default();
    let _ = store.insert(key(1), used(5));
    let _ = store.insert(key(2), used(5));
    record_route(&mut store, key(1), route(2, 50), CAP, ttl, |_, _, _| {});
    assert_eq!(
        store.get(&key(1)),
        Some(&RouteEntry {
            used: Some(5),
            ..route(2, 50)
        })
    );
    record_route(&mut store, key(3), route(2, 50), CAP, ttl, |_, _, _| {});
    assert_eq!(
        store.get(&key(3)).map(|r| r.used),
        Some(None),
        "a new route is unused"
    );
    record_route(&mut store, key(2), route(2, TTL), CAP, ttl, |_, _, _| {});
    assert_eq!(
        store.get(&key(2)).map(|r| r.used),
        Some(None),
        "a route past its lifetime hands on nothing"
    );
}

both!(routes_mode_shortens_their_life, RouteEntry<u64>);
fn routes_mode_shortens_their_life<S: TestStore<RouteEntry<u64>>>() {
    let mut store = S::default();
    let roaming = RouteEntry {
        mode: InterfaceMode::Roaming,
        ..route(1, 0)
    };
    let _ = store.insert(key(1), roaming);
    let _ = store.insert(key(2), route(1, 0));
    let live = |now| move |r: &RouteEntry<u64>| r.live(now, ttl);
    assert!(find_live(&store, &key(1), live(TTL / 3 - 1)).is_some());
    assert!(find_live(&store, &key(1), live(TTL / 3)).is_none());
    assert!(find_live(&store, &key(2), live(TTL / 3)).is_some());
}

both!(route_sweep_reports_each_dead_route, RouteEntry<u64>);
fn route_sweep_reports_each_dead_route<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(1, u64::from(n) * 10));
    let mut gone = Vec::new();
    sweep_routes(&mut store, 125, ttl, |k, _| gone.push(k));
    gone.sort();
    assert_eq!(gone, [key(1), key(2)]);
    assert_eq!(store.len(), 2);
}

both!(routes_forget_an_interface, RouteEntry<u64>);
fn routes_forget_an_interface<S: TestStore<RouteEntry<u64>>>() {
    let mut store: S = filled(|n| route(u32::from(n % 2), 0));
    forget_interface(&mut store, 1, |r| [r.iface, r.iface]);
    assert_eq!(store.len(), 2);
    assert!(store.get(&key(1)).is_none() && store.get(&key(3)).is_none());
}

both!(bridges_follow_a_retransmitted_request, LinkBridge<u64>);
fn bridges_follow_a_retransmitted_request<S: TestStore<LinkBridge<u64>>>() {
    let mut store = S::default();
    let new = record_bridge(&mut store, key(1), bridge(0, Some(100)), CAP, IDLE);
    assert_eq!(new, BridgeRecord::New { evicted: None });
    let again = record_bridge(&mut store, key(1), bridge(5, Some(200)), CAP, IDLE);
    assert_eq!(again, BridgeRecord::Known);
    assert_eq!(
        store.get(&key(1)),
        Some(&bridge(5, Some(200))),
        "unproved follows"
    );
    assert!(refresh_bridge(&mut store, &key(1), 7, true));
    assert_eq!(
        store.get(&key(1)),
        Some(&bridge(7, None)),
        "the proof validates"
    );
    let known = record_bridge(&mut store, key(1), bridge(9, Some(300)), CAP, IDLE);
    assert_eq!(known, BridgeRecord::Known);
    assert_eq!(
        store.get(&key(1)),
        Some(&bridge(7, None)),
        "a validated bridge stands"
    );
    assert!(refresh_bridge(&mut store, &key(1), 8, false));
    assert_eq!(store.get(&key(1)), Some(&bridge(8, None)));
    assert!(!refresh_bridge(&mut store, &key(2), 8, false));
}

both!(bridges_make_room_from_unproved_then_idle, LinkBridge<u64>);
fn bridges_make_room_from_unproved_then_idle<S: TestStore<LinkBridge<u64>>>() {
    // Validated 1 and 2, unproved 3 (seen 30) and 4 (seen 20).
    let mut store: S = filled(|n| {
        let deadline = (n > 2).then_some(1000);
        bridge(if n == 4 { 20 } else { u64::from(n) * 10 }, deadline)
    });
    let unproved = record_bridge(&mut store, key(5), bridge(40, Some(1000)), CAP, IDLE);
    assert_eq!(
        unproved,
        BridgeRecord::New {
            evicted: Some(key(4))
        }
    );
    let unproved = record_bridge(&mut store, key(6), bridge(40, Some(1000)), CAP, IDLE);
    assert_eq!(
        unproved,
        BridgeRecord::New {
            evicted: Some(key(3))
        }
    );
    for n in [5, 6] {
        refresh_bridge(&mut store, &key(n), 40, true);
    }
    // Every slot validated; 1 and 2 were last heard at 10 and 20.
    let busy = record_bridge(&mut store, key(7), bridge(59, Some(1000)), CAP, IDLE);
    assert_eq!(
        busy,
        BridgeRecord::Refused,
        "no validated bridge is idle at 59"
    );
    let idle = record_bridge(&mut store, key(7), bridge(70, Some(1000)), CAP, IDLE);
    assert_eq!(
        idle,
        BridgeRecord::New {
            evicted: Some(key(1))
        }
    );
    assert_eq!(store.len(), CAP);
}

both!(bridges_lapse_at_deadline_or_ttl, LinkBridge<u64>);
fn bridges_lapse_at_deadline_or_ttl<S: TestStore<LinkBridge<u64>>>() {
    let mut store: S = filled(|n| match n {
        1 => bridge(0, Some(50)),
        2 => bridge(0, Some(500)),
        3 => bridge(0, None),
        _ => bridge(450, None),
    });
    let live = |now| move |b: &LinkBridge<u64>| !b.lapsed(now, 500);
    assert!(find_live(&store, &key(1), live(49)).is_some());
    assert!(
        find_live(&store, &key(1), live(50)).is_none(),
        "at its proof deadline"
    );
    let mut gone = Vec::new();
    sweep_bridges(&mut store, 500, 500, |k, _| gone.push(k));
    gone.sort();
    assert_eq!(gone, [key(1), key(2), key(3)]);
    assert_eq!(store.len(), 1);
    forget_interface(&mut store, 2, |b| [b.from, b.out]);
    assert_eq!(store.len(), 0);
}

/// Record into `store`, then undo, and check nothing moved.
fn record_then_restore<S: TestStore<ReverseEntry<u64>>>(mut store: S, key: AddressHash, now: u64) {
    let before = store.clone();
    let mut displaced = Vec::new();
    record_reverse(&mut store, key, reverse(now), CAP, TIMEOUT, |d| {
        displaced.push(d);
    });
    assert_eq!(store.get(&key), Some(&reverse(now)));
    assert!(store.len() <= CAP);
    restore_reverse(&mut store, &key, displaced.into_iter().rev());
    assert_eq!(store, before, "a refused send leaves the table as it was");
}

both!(reverse_paths_restore_exactly, ReverseEntry<u64>);
fn reverse_paths_restore_exactly<S: TestStore<ReverseEntry<u64>>>() {
    record_then_restore(S::default(), key(9), 10);
    let full = || filled::<_, S>(|n| reverse(u64::from(n)));
    record_then_restore(full(), key(2), 10);
    record_then_restore(full(), key(9), 10);
    // Two expired (1 and 2 at 82), then still room: nothing live is evicted.
    record_then_restore(full(), key(9), 82);
    record_then_restore(full(), key(9), 1000);
}

both!(reverse_paths_make_room_dead_first, ReverseEntry<u64>);
fn reverse_paths_make_room_dead_first<S: TestStore<ReverseEntry<u64>>>() {
    let mut store: S = filled(|n| reverse(u64::from(n)));
    let mut displaced = Vec::new();
    record_reverse(&mut store, key(9), reverse(10), CAP, TIMEOUT, |d| {
        displaced.push(d.key);
    });
    assert_eq!(displaced, [key(1)], "the oldest goes when all are live");
    displaced.clear();
    record_reverse(&mut store, key(8), reverse(83), CAP, TIMEOUT, |d| {
        displaced.push(d.key);
    });
    displaced.sort();
    assert_eq!(
        displaced,
        [key(2), key(3)],
        "the expired go, and nothing live"
    );
    assert_eq!(store.len(), 3);
}

both!(reverse_paths_are_consumed_by_their_proof, ReverseEntry<u64>);
fn reverse_paths_are_consumed_by_their_proof<S: TestStore<ReverseEntry<u64>>>() {
    let mut store: S = filled(|n| reverse(u64::from(n)));
    assert_eq!(take_reverse(&mut store, &key(1), 2, 10, TIMEOUT), Some(1));
    assert_eq!(
        take_reverse(&mut store, &key(1), 2, 10, TIMEOUT),
        None,
        "consumed"
    );
    assert_eq!(
        take_reverse(&mut store, &key(2), 1, 10, TIMEOUT),
        None,
        "wrong side"
    );
    assert!(store.get(&key(2)).is_none(), "consumed either way");
    assert_eq!(
        take_reverse(&mut store, &key(3), 2, 83, TIMEOUT),
        None,
        "expired"
    );
    sweep_reverse(&mut store, 84, TIMEOUT);
    assert_eq!(store.len(), 0);
    let _ = store.insert(key(5), reverse(0));
    forget_interface(&mut store, 1, |r| [r.received, r.outbound]);
    assert_eq!(store.len(), 0);
}

/// The board's ties go to the earliest slot, and its removals swap the last slot in.
#[test]
fn the_array_keeps_the_boards_order() {
    let mut store: BoundedVec<Keyed<ReverseEntry<u64>>, CAP> = filled(|_| reverse(5));
    assert_eq!(
        store.least(|r| Some(r.at)),
        Some(key(1)),
        "first of equal ranks"
    );
    let mut displaced = None;
    record_reverse(&mut store, key(9), reverse(6), CAP, TIMEOUT, |d| {
        displaced = Some(d);
    });
    assert_eq!(displaced.map(|d| d.place), Some(Position::Swapped(0)));
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    assert_eq!(keys, [key(4), key(2), key(3), key(9)]);
    let mut store: BoundedVec<Keyed<RouteEntry<u64>>, CAP> = filled(|n| route(1, u64::from(n)));
    sweep_routes(&mut store, 102, ttl, |_, _| {});
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    assert_eq!(keys, [key(4), key(3)], "each dead slot takes the last");
    let mut store: BoundedVec<Keyed<RouteEntry<u64>>, CAP> = filled(|n| route(u32::from(n), 0));
    forget_interface(&mut store, 2, |r| [r.iface, r.iface]);
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    assert_eq!(keys, [key(1), key(3), key(4)], "retain keeps order");
}

/// A full array refuses a new key rather than growing.
#[test]
fn a_full_array_refuses_a_new_key() {
    let mut store: BoundedVec<Keyed<LinkBridge<u64>>, CAP> = filled(|_| bridge(0, None));
    let refused = Store::insert(&mut store, key(9), bridge(1, None));
    assert_eq!(refused, Err(bridge(1, None)));
    // A capacity above the array's bound cannot make room either.
    let over = record_bridge(&mut store, key(9), bridge(1, None), CAP + 1, IDLE);
    assert_eq!(over, BridgeRecord::Refused);
}

#[cfg(feature = "tokio")]
#[test]
fn instants_count_whole_milliseconds() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let later = start + Duration::from_micros(2_999);
    assert_eq!(later.millis_since(start), 2);
    assert_eq!(start.millis_since(later), 0, "never negative");
    let bridge = LinkBridge {
        seen: start,
        proof_deadline: None,
        ..super::tests::bridge_at_instant(start)
    };
    assert!(!bridge.lapsed(start + Duration::from_micros(2_999), 3));
    assert!(bridge.lapsed(start + Duration::from_millis(3), 3));
}

#[cfg(feature = "tokio")]
fn bridge_at_instant(at: std::time::Instant) -> LinkBridge<std::time::Instant> {
    LinkBridge {
        from: 1,
        out: 2,
        destination: key(0xEE),
        seen: at,
        proof_deadline: Some(at),
    }
}
