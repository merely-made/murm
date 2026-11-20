//! Link bridges and return paths, against both containers as in the route tests.

use super::*;

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

both!(
    bridges_make_room_from_unproved_then_idle,
    LinkBridge<u64>,
    DESKTOP_LINK_TRANSPORT_CAPACITY
);
fn bridges_make_room_from_unproved_then_idle<S: TestStore<LinkBridge<u64>>>(cap: usize) {
    // Validated 1 and 2, unproved 3 (seen 30) and 4 (seen 20). Any further slots hold
    // validated bridges heard at 40, as 5 and 6 will be.
    let mut store: S = filled(cap, |n| match n {
        1..=4 => {
            let deadline = (n > 2).then_some(1000);
            bridge(if n == 4 { 20 } else { n as u64 * 10 }, deadline)
        }
        _ => bridge(40, None),
    });
    let [five, six, seven] = [1, 2, 3].map(|n| nth(cap + n));
    let unproved = record_bridge(&mut store, five, bridge(40, Some(1000)), cap, IDLE);
    assert_eq!(
        unproved,
        BridgeRecord::New {
            evicted: Some(key(4))
        }
    );
    let unproved = record_bridge(&mut store, six, bridge(40, Some(1000)), cap, IDLE);
    assert_eq!(
        unproved,
        BridgeRecord::New {
            evicted: Some(key(3))
        }
    );
    for n in [five, six] {
        refresh_bridge(&mut store, &n, 40, true);
    }
    // Every slot validated; 1 and 2 were last heard at 10 and 20.
    let busy = record_bridge(&mut store, seven, bridge(59, Some(1000)), cap, IDLE);
    assert_eq!(
        busy,
        BridgeRecord::Refused,
        "no validated bridge is idle at 59"
    );
    let idle = record_bridge(&mut store, seven, bridge(70, Some(1000)), cap, IDLE);
    assert_eq!(
        idle,
        BridgeRecord::New {
            evicted: Some(key(1))
        }
    );
    assert_eq!(store.len(), cap);
}

both!(
    bridges_lapse_at_deadline_or_ttl,
    LinkBridge<u64>,
    DESKTOP_LINK_TRANSPORT_CAPACITY
);
fn bridges_lapse_at_deadline_or_ttl<S: TestStore<LinkBridge<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| match n {
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
    assert_eq!(store.len(), cap - 3);
    forget_interface(&mut store, 2, |b| [b.from, b.out]);
    assert_eq!(store.len(), 0);
}

/// Record into `store`, then undo, and check nothing moved.
fn record_then_restore<S: TestStore<ReverseEntry<u64>>>(
    mut store: S,
    key: AddressHash,
    now: u64,
    cap: usize,
) {
    let before = store.clone();
    let mut displaced = Vec::new();
    record_reverse(&mut store, key, reverse(now), cap, TIMEOUT, |d| {
        displaced.push(d);
    });
    assert_eq!(store.get(&key), Some(&reverse(now)));
    assert!(store.len() <= cap);
    restore_reverse(&mut store, &key, displaced.into_iter().rev());
    assert_eq!(store, before, "a refused send leaves the table as it was");
}

both!(
    reverse_paths_restore_exactly,
    ReverseEntry<u64>,
    DESKTOP_REVERSE_TABLE_CAPACITY
);
fn reverse_paths_restore_exactly<S: TestStore<ReverseEntry<u64>>>(cap: usize) {
    let new = nth(cap + 5);
    record_then_restore(S::default(), new, 10, cap);
    let full = || filled::<_, S>(cap, |n| reverse(n as u64));
    record_then_restore(full(), key(2), 10, cap);
    record_then_restore(full(), new, 10, cap);
    // Two expired (1 and 2 at 82), then still room: nothing live is evicted.
    record_then_restore(full(), new, 82, cap);
    record_then_restore(full(), new, 1000, cap);
}

both!(
    reverse_paths_make_room_dead_first,
    ReverseEntry<u64>,
    DESKTOP_REVERSE_TABLE_CAPACITY
);
fn reverse_paths_make_room_dead_first<S: TestStore<ReverseEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| reverse(n as u64));
    let mut displaced = Vec::new();
    record_reverse(&mut store, nth(cap + 5), reverse(10), cap, TIMEOUT, |d| {
        displaced.push(d.key);
    });
    assert_eq!(displaced, [key(1)], "the oldest goes when all are live");
    displaced.clear();
    record_reverse(&mut store, nth(cap + 4), reverse(83), cap, TIMEOUT, |d| {
        displaced.push(d.key);
    });
    displaced.sort();
    assert_eq!(
        displaced,
        [key(2), key(3)],
        "the expired go, and nothing live"
    );
    assert_eq!(store.len(), cap - 1);
}

both!(
    reverse_paths_are_consumed_by_their_proof,
    ReverseEntry<u64>,
    DESKTOP_REVERSE_TABLE_CAPACITY
);
fn reverse_paths_are_consumed_by_their_proof<S: TestStore<ReverseEntry<u64>>>(cap: usize) {
    let mut store: S = filled(cap, |n| reverse(n as u64));
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
    // At 84 the path recorded at 4 has expired; any recorded later have not.
    sweep_reverse(&mut store, 84, TIMEOUT);
    assert_eq!(store.len(), cap - 4);
    let _ = store.insert(nth(cap + 1), reverse(0));
    forget_interface(&mut store, 1, |r| [r.received, r.outbound]);
    assert_eq!(store.len(), 0);
}
