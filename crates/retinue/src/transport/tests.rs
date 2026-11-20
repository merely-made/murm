//! Every table operation against both containers, with the same assertions: each body is
//! generic over the store and instantiated once per container. A body that fills its table
//! also runs at each driver's real size: the board's array at 16, the desktop's map at 4096.

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

/// The board's table size: `Node`'s default `ROUTES`, which bounds its routes, bridges and
/// return paths.
#[cfg(feature = "alloc")]
const BOARD: usize = routes_of(core::marker::PhantomData::<crate::node::Node>);

#[cfg(feature = "alloc")]
const fn routes_of<const P: usize, const A: usize, const L: usize, const R: usize>(
    _: core::marker::PhantomData<crate::node::Node<P, A, L, R>>,
) -> usize {
    R
}

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

/// The `n`th key: [`key`] below 256, and a distinct key for every `n` below 2^24.
fn nth(n: usize) -> AddressHash {
    let [low, mid, high, ..] = n.to_le_bytes();
    let mut bytes = [low; 16];
    bytes[0] ^= mid;
    bytes[1] ^= high;
    AddressHash::from_bytes(bytes)
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

/// Run a generic body against the bounded array and, with `std`, the map. With a desktop
/// size named, the body takes its capacity and also runs over the array at the board's size
/// and over the map at that one.
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
    ($name:ident, $entry:ty, $desktop:ident) => {
        mod $name {
            use super::*;

            #[test]
            fn array() {
                super::$name::<BoundedVec<Keyed<$entry>, CAP>>(CAP);
            }

            #[cfg(feature = "alloc")]
            #[test]
            fn board() {
                super::$name::<BoundedVec<Keyed<$entry>, BOARD>>(BOARD);
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn map() {
                super::$name::<std::collections::HashMap<AddressHash, $entry>>(CAP);
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn desktop() {
                super::$name::<std::collections::HashMap<AddressHash, $entry>>(
                    crate::endpoint::$desktop,
                );
            }
        }
    };
}

/// A table of `cap` entries, keyed `nth(1)` to `nth(cap)`.
fn filled<V: Copy, S: TestStore<V>>(cap: usize, make: impl Fn(usize) -> V) -> S {
    let mut store = S::default();
    for n in 1..=cap {
        assert!(matches!(store.insert(nth(n), make(n)), Ok(None)));
    }
    store
}

both!(
    routes_replace_then_evict_the_soonest_to_expire,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn routes_replace_then_evict_the_soonest_to_expire<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(1, n as u64));
    let mut gone = Vec::new();
    let replaced = record_route(&mut store, key(2), route(3, 10), cap, ttl, |k, _, why| {
        gone.push((k, why));
    });
    assert_eq!(replaced, Some(route(1, 2)), "a known key is replaced");
    assert!(gone.is_empty(), "replacing makes no room");
    record_route(&mut store, nth(cap + 5), route(1, 11), cap, ttl, |k, _, why| {
        gone.push((k, why));
    });
    assert_eq!(
        gone,
        [(key(1), Removal::Evicted)],
        "the never-used route soonest to expire goes"
    );
    assert_eq!(store.len(), cap);
    assert_eq!(store.get(&key(2)).map(|r| r.iface), Some(3));
}

both!(
    full_routes_drop_never_used_then_least_recently_used,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn full_routes_drop_never_used_then_least_recently_used<S: TestStore<RouteEntry<u64>>>(
    cap: usize,
) {
    let recent = RouteEntry {
        used: Some(31),
        ..route(1, 10)
    };
    // 1 and 2 used at 30 and 20; 3 never used, learned at 3; 4 never used, roaming, learned
    // at 4, so it expires first though it was learned last. Any further slots hold routes
    // used later than all four.
    let mut store: S = filled(cap, |n| match n {
        1 => RouteEntry {
            used: Some(30),
            ..route(1, 1)
        },
        2 => RouteEntry {
            used: Some(20),
            ..route(1, 2)
        },
        3 => route(1, 3),
        4 => RouteEntry {
            mode: InterfaceMode::Roaming,
            ..route(1, 4)
        },
        _ => recent,
    });
    let mut gone = Vec::new();
    for n in cap + 1..=cap + 4 {
        record_route(&mut store, nth(n), recent, cap, ttl, |k, _, why| {
            gone.push((k, why));
        });
    }
    let evicted = [4, 3, 2, 1].map(|n| (key(n), Removal::Evicted));
    assert_eq!(
        gone, evicted,
        "never used, soonest to expire first; then least recently used"
    );
}

both!(
    routes_full_of_dead_never_evict_the_living,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn routes_full_of_dead_never_evict_the_living<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(1, if n == 3 { 150 } else { 0 }));
    let mut gone = Vec::new();
    record_route(&mut store, nth(cap + 5), route(1, 160), cap, ttl, |k, _, why| {
        gone.push((k, why));
    });
    gone.sort_by_key(|(k, _)| *k);
    let mut expired: Vec<_> = (1..=cap)
        .filter(|&n| n != 3)
        .map(|n| (nth(n), Removal::Expired))
        .collect();
    expired.sort_by_key(|(k, _)| *k);
    assert_eq!(gone, expired);
    assert_eq!(store.len(), 2);
    assert!(store.get(&key(3)).is_some());
}

both!(
    route_lookups_check_age,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn route_lookups_check_age<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(1, n as u64 * 10));
    let live = |now| move |r: &RouteEntry<u64>| r.live(now, &ttl);
    assert_eq!(find_live(&store, &key(1), live(109)), Some(&route(1, 10)));
    assert_eq!(
        find_live(&store, &key(1), live(110)),
        None,
        "past its lifetime"
    );
    assert_eq!(store.len(), cap, "a read-only lookup removes nothing");
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
    assert_eq!(store.len(), cap - 1);
}

both!(
    routes_are_marked_used_only_when_live,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn routes_are_marked_used_only_when_live<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(1, n as u64 * 10));
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
    assert!(!mark_route_used(&mut store, &nth(cap + 5), 50, ttl));
}

both!(
    proving_a_route_alive_starts_its_life_over,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn proving_a_route_alive_starts_its_life_over<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(n as u32, n as u64 * 10));
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
    assert!(!prove_route_alive(&mut store, &nth(cap + 5), 50, mode, ttl));
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
    let live = |now| move |r: &RouteEntry<u64>| r.live(now, &ttl);
    assert!(find_live(&store, &key(1), live(TTL / 3 - 1)).is_some());
    assert!(find_live(&store, &key(1), live(TTL / 3)).is_none());
    assert!(find_live(&store, &key(2), live(TTL / 3)).is_some());
}

/// Lifetimes as [`ttl`], with a linger of 50 on interface 1 and none elsewhere.
#[derive(Clone, Copy)]
struct Lingering;

impl RouteLife for Lingering {
    fn lifetime(&self, mode: InterfaceMode) -> u64 {
        ttl(mode)
    }
    fn linger(&self, iface: InterfaceId) -> Option<u64> {
        (iface == 1).then_some(50)
    }
}

both!(
    never_used_routes_live_only_as_long_as_their_linger,
    RouteEntry<u64>
);
fn never_used_routes_live_only_as_long_as_their_linger<S: TestStore<RouteEntry<u64>>>() {
    let mut store = S::default();
    let used = RouteEntry {
        used: Some(1),
        ..route(1, 0)
    };
    for (n, entry) in [(1, route(1, 0)), (2, used), (3, route(2, 0))] {
        let _ = store.insert(key(n), entry);
    }
    let live = |n, now| find_live(&store, &key(n), |r| r.live(now, &Lingering)).is_some();
    assert!(
        live(1, 49) && !live(1, 50),
        "never used: gone at its linger"
    );
    assert!(live(2, TTL - 1), "a used route keeps its lifetime");
    assert!(live(3, TTL - 1), "no linger, no cull");
    assert!(
        !mark_route_used(&mut store, &key(1), 50, Lingering),
        "a culled route stays dead"
    );
    let mut gone = Vec::new();
    sweep_routes(&mut store, 50, Lingering, |k, _| gone.push(k));
    assert_eq!(gone, [key(1)]);
}

both!(
    route_sweep_reports_each_dead_route,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn route_sweep_reports_each_dead_route<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route(1, n as u64 * 10));
    let mut gone = Vec::new();
    sweep_routes(&mut store, 125, ttl, |k, _| gone.push(k));
    gone.sort();
    assert_eq!(gone, [key(1), key(2)]);
    assert_eq!(store.len(), cap - 2);
}

both!(
    routes_forget_an_interface,
    RouteEntry<u64>,
    DESKTOP_PATH_TABLE_CAPACITY
);
fn routes_forget_an_interface<S: TestStore<RouteEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| route((n % 2) as u32, 0));
    forget_interface(&mut store, 1, |r| [r.iface, r.iface]);
    assert_eq!(store.len(), cap / 2);
    assert!(store.get(&key(1)).is_none() && store.get(&key(3)).is_none());
}

/// The board's ties go to the earliest slot, and its removals swap the last slot in.
#[test]
fn the_array_keeps_the_boards_order() {
    the_array_keeps_the_boards_order_at::<CAP>();
}

#[cfg(feature = "alloc")]
#[test]
fn the_array_keeps_the_boards_order_at_the_boards_size() {
    the_array_keeps_the_boards_order_at::<BOARD>();
}

fn the_array_keeps_the_boards_order_at<const N: usize>() {
    let mut store: BoundedVec<Keyed<ReverseEntry<u64>>, N> = filled(N, |_| reverse(5));
    assert_eq!(
        store.least(|r| Some(r.at)),
        Some(key(1)),
        "first of equal ranks"
    );
    let mut displaced = None;
    record_reverse(&mut store, nth(N + 5), reverse(6), N, TIMEOUT, |d| {
        displaced = Some(d);
    });
    assert_eq!(displaced.map(|d| d.place), Some(Position::Swapped(0)));
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    let swapped: Vec<_> = [N].into_iter().chain(2..N).chain([N + 5]).map(nth).collect();
    assert_eq!(keys, swapped);
    let mut store: BoundedVec<Keyed<RouteEntry<u64>>, N> = filled(N, |n| route(1, n as u64));
    sweep_routes(&mut store, 102, ttl, |_, _| {});
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    let swept: Vec<_> = [N, N - 1].into_iter().chain(3..N - 1).map(nth).collect();
    assert_eq!(keys, swept, "each dead slot takes the last");
    let mut store: BoundedVec<Keyed<RouteEntry<u64>>, N> = filled(N, |n| route(n as u32, 0));
    forget_interface(&mut store, 2, |r| [r.iface, r.iface]);
    let keys: Vec<_> = store.iter().map(|slot| slot.key).collect();
    let kept: Vec<_> = [1].into_iter().chain(3..=N).map(nth).collect();
    assert_eq!(keys, kept, "retain keeps order");
}

/// A full array refuses a new key rather than growing.
#[test]
fn a_full_array_refuses_a_new_key() {
    a_full_array_refuses_a_new_key_at::<CAP>();
}

#[cfg(feature = "alloc")]
#[test]
fn a_full_array_refuses_a_new_key_at_the_boards_size() {
    a_full_array_refuses_a_new_key_at::<BOARD>();
}

fn a_full_array_refuses_a_new_key_at<const N: usize>() {
    let mut store: BoundedVec<Keyed<LinkBridge<u64>>, N> = filled(N, |_| bridge(0, None));
    let refused = Store::insert(&mut store, nth(N + 5), bridge(1, None));
    assert_eq!(refused, Err(bridge(1, None)));
    // A capacity above the array's bound cannot make room either.
    let over = record_bridge(&mut store, nth(N + 5), bridge(1, None), N + 1, IDLE);
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

// After `both!`, which they use.
mod bridges;
mod forward;
