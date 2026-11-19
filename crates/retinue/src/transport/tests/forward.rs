//! The forwarding decisions, against both containers where they hold a table.

use core::cell::Cell;

use super::*;
use crate::iface_mode::ModeFlags;

const ON: Carriage = Carriage {
    on: true,
    ingress: true,
    max_hops: 4,
};

fn off() -> Carriage {
    Carriage { on: false, ..ON }
}

both!(crossings_are_admitted_by_side_and_proof, LinkBridge<u64>);
fn crossings_are_admitted_by_side_and_proof<S: TestStore<LinkBridge<u64>>>() {
    let link = key(1);
    let admit = |store: &S, arrived, proves, signed: Option<bool>, carriage: &Carriage| {
        admit_crossing(store, &link, arrived, proves, 10, TTL, carriage, |_| signed)
    };
    let mut store = S::default();
    assert_eq!(
        admit(&store, 1, false, None, &ON),
        Transit::Local,
        "no bridge"
    );
    store.insert(link, bridge(0, Some(5))).unwrap();
    assert_eq!(admit(&store, 1, false, None, &ON), Transit::Local, "lapsed");
    store.insert(link, bridge(0, Some(50))).unwrap();
    assert_eq!(admit(&store, 1, false, None, &off()), Transit::CarriageOff);
    assert_eq!(
        admit(&store, 3, false, None, &ON),
        Transit::Refused(Refusal::ThirdInterface)
    );
    assert_eq!(
        admit(&store, 1, false, None, &ON),
        Transit::Refused(Refusal::Unvalidated),
        "nothing before the proof"
    );
    for (arrived, signed, expected) in [
        (1, None, Transit::Refused(Refusal::Unvalidated)),
        (2, Some(false), Transit::Refused(Refusal::Unvalidated)),
        (2, None, cross(1, Crossing::Proof { verified: false })),
        (2, Some(true), cross(1, Crossing::Proof { verified: true })),
    ] {
        assert_eq!(admit(&store, arrived, true, signed, &ON), expected);
    }
    store.insert(link, bridge(0, None)).unwrap();
    assert_eq!(
        admit(&store, 1, false, None, &ON),
        cross(2, Crossing::Traffic)
    );
    assert_eq!(
        admit(&store, 2, false, None, &ON),
        cross(1, Crossing::Traffic)
    );
}

fn cross(out: InterfaceId, crossing: Crossing) -> Transit<Cross> {
    Transit::Carry(Cross {
        out,
        destination: key(0xEE),
        crossing,
    })
}

#[test]
fn the_crossing_gate_keeps_its_order() {
    let refused = Carriage {
        ingress: false,
        ..ON
    };
    assert_eq!(
        cross_gate(&refused, 0, 2, || true, |_| true),
        Err(Refusal::Ingress)
    );
    // At the ceiling the memory is not consulted, and egress is asked first.
    let ran = Cell::new(false);
    let remembered = || {
        ran.set(true);
        true
    };
    assert_eq!(
        cross_gate(&ON, 3, 2, remembered, |_| false),
        Err(Refusal::Egress)
    );
    assert!(!ran.get());
    assert_eq!(
        cross_gate(&ON, 3, 2, || true, |_| true),
        Err(Refusal::HopLimit)
    );
    let asked = Cell::new(false);
    assert_eq!(
        cross_gate(
            &ON,
            2,
            2,
            || false,
            |_| {
                asked.set(true);
                true
            }
        ),
        Err(Refusal::Duplicate)
    );
    assert!(!asked.get(), "a duplicate is not offered to egress");
    assert_eq!(cross_gate(&ON, 2, 2, || true, |out| out == 2), Ok(()));
}

both!(a_sent_crossing_refreshes_its_bridge, LinkBridge<u64>);
fn a_sent_crossing_refreshes_its_bridge<S: TestStore<LinkBridge<u64>>>() {
    for (crossing, touch) in [
        (Crossing::Traffic, RouteTouch::MarkUsed(key(0xEE))),
        (
            Crossing::Proof { verified: false },
            RouteTouch::MarkUsed(key(0xEE)),
        ),
        (
            Crossing::Proof { verified: true },
            RouteTouch::ProveAlive(key(0xEE)),
        ),
    ] {
        let mut store = S::default();
        let deadline = (crossing != Crossing::Traffic).then_some(50);
        store.insert(key(1), bridge(0, deadline)).unwrap();
        let sent = Cross {
            out: 1,
            destination: key(0xEE),
            crossing,
        };
        assert_eq!(crossed(&mut store, &key(1), 20, &sent), touch);
        assert_eq!(store.get(&key(1)), Some(&bridge(20, None)));
    }
}

#[test]
fn the_hop_sequence_keeps_its_order() {
    let routed = route(2, 0);
    let go = |addressed, hops, carriage: &Carriage, new: bool, route: Option<_>, egress: bool| {
        let (looked, asked) = (Cell::new(false), Cell::new(false));
        let decision = forward_hop(
            addressed,
            hops,
            carriage,
            || new,
            || {
                looked.set(true);
                route
            },
            |_| {
                asked.set(true);
                egress
            },
        );
        (decision, looked.get(), asked.get())
    };
    let refused = Carriage {
        ingress: false,
        ..ON
    };
    let cases = [
        (go(false, 0, &ON, true, Some(routed), true), Transit::Local),
        (
            go(true, 0, &off(), true, Some(routed), true),
            Transit::CarriageOff,
        ),
        (
            go(true, 0, &refused, true, Some(routed), true),
            Transit::Refused(Refusal::Ingress),
        ),
        (
            go(true, 3, &ON, true, Some(routed), true),
            Transit::Refused(Refusal::HopLimit),
        ),
        (
            go(true, 0, &ON, false, Some(routed), true),
            Transit::Refused(Refusal::Duplicate),
        ),
        (
            go(true, 0, &ON, true, None, true),
            Transit::Refused(Refusal::Unroutable),
        ),
        (
            go(true, 0, &ON, true, Some(routed), false),
            Transit::Refused(Refusal::Egress),
        ),
        (
            go(true, 0, &ON, true, Some(routed), true),
            Transit::Carry(routed),
        ),
    ];
    for (n, ((decision, looked, asked), expected)) in cases.into_iter().enumerate() {
        assert_eq!(decision, expected, "case {n}");
        // The route is read only after the memory, and egress only with a route.
        assert_eq!(looked, n >= 5, "case {n}: route read");
        assert_eq!(asked, n >= 6, "case {n}: egress asked");
    }
}

both!(a_carried_packet_reads_a_live_route_once, RouteEntry<u64>);
fn a_carried_packet_reads_a_live_route_once<S: TestStore<RouteEntry<u64>>>() {
    let mut store = S::default();
    store.insert(key(1), route(2, 0)).unwrap();
    store.insert(key(2), route(2, 0)).unwrap();
    assert_eq!(
        live_route(&mut store, &key(1), TTL - 1, ttl),
        Lookup::Live(route(2, 0))
    );
    assert_eq!(
        live_route(&mut store, &key(2), TTL, ttl),
        Lookup::Expired(route(2, 0))
    );
    assert_eq!(store.get(&key(2)), None, "a lapsed route is removed");
    assert_eq!(live_route(&mut store, &key(3), 0, ttl), Lookup::Missing);
}

both!(
    an_unsent_request_gives_up_only_a_new_bridge,
    LinkBridge<u64>
);
fn an_unsent_request_gives_up_only_a_new_bridge<S: TestStore<LinkBridge<u64>>>() {
    let mut store = S::default();
    let by = route(2, 0);
    let record = admit_request(&mut store, key(1), key(0xEE), 1, &by, 10, 40, CAP, IDLE);
    assert_eq!(record, BridgeRecord::New { evicted: None });
    assert_eq!(store.get(&key(1)), Some(&bridge(10, Some(40))));
    let again = admit_request(&mut store, key(1), key(0xEE), 1, &by, 12, 42, CAP, IDLE);
    assert_eq!(again, BridgeRecord::Known);
    unsent_request(&mut store, &key(1), again);
    assert_eq!(
        store.get(&key(1)),
        Some(&bridge(12, Some(42))),
        "a retransmission keeps its bridge"
    );
    unsent_request(&mut store, &key(1), record);
    assert_eq!(store.get(&key(1)), None);
}

both!(proofs_go_back_by_their_return_path, ReverseEntry<u64>);
fn proofs_go_back_by_their_return_path<S: TestStore<ReverseEntry<u64>>>() {
    let refused = Carriage {
        ingress: false,
        ..ON
    };
    // (link proof, arrived, hops, carriage, egress, decision, path kept)
    let cases = [
        (true, 2, 0, ON, true, Transit::Local, true),
        (false, 2, 0, off(), true, Transit::CarriageOff, true),
        (false, 1, 0, off(), true, Transit::Local, true),
        (false, 1, 0, ON, true, Transit::Local, false),
        (
            false,
            2,
            0,
            refused,
            true,
            Transit::Refused(Refusal::Ingress),
            false,
        ),
        (
            false,
            2,
            0,
            ON,
            false,
            Transit::Refused(Refusal::Egress),
            false,
        ),
        (
            false,
            2,
            3,
            ON,
            true,
            Transit::Refused(Refusal::HopLimit),
            false,
        ),
        (false, 2, 0, ON, true, Transit::Carry(1), false),
    ];
    for (n, (link_proof, arrived, hops, carriage, egress, expected, kept)) in
        cases.into_iter().enumerate()
    {
        let mut store = S::default();
        store.insert(key(1), reverse(0)).unwrap();
        let decision = reverse_proof(
            &mut store,
            &key(1),
            arrived,
            link_proof,
            hops,
            10,
            TIMEOUT,
            &carriage,
            |_| egress,
        );
        assert_eq!(decision, expected, "case {n}");
        assert_eq!(store.get(&key(1)).is_some(), kept, "case {n}: path kept");
    }
    let mut store = S::default();
    store.insert(key(1), reverse(0)).unwrap();
    assert_eq!(
        reverse_proof(
            &mut store,
            &key(1),
            2,
            false,
            0,
            TIMEOUT,
            TIMEOUT,
            &off(),
            |_| true
        ),
        Transit::Local,
        "an expired path would not carry"
    );
}

#[test]
fn the_relay_gate_keeps_the_desktop_order() {
    let gate = |path_response, enabled, new: bool, blocked: bool, hops| {
        let observed = Cell::new(false);
        let decision = relay_gate(
            path_response,
            || {
                observed.set(true);
                blocked
            },
            enabled,
            || new,
            hops,
            4,
        );
        (decision, observed.get())
    };
    assert_eq!(
        gate(true, true, true, false, 0),
        (Err(RelayRefusal::PathResponse), false),
        "a path response is not rate-counted"
    );
    assert_eq!(
        gate(false, false, true, false, 0),
        (Err(RelayRefusal::Disabled), true),
        "the rate is observed whether or not this node relays"
    );
    assert_eq!(
        gate(false, true, false, true, 3),
        (Err(RelayRefusal::Duplicate), true),
        "remembered first (D26)"
    );
    assert_eq!(
        gate(false, true, true, true, 3),
        (Err(RelayRefusal::RateLimited), true)
    );
    assert_eq!(
        gate(false, true, true, false, 3),
        (Err(RelayRefusal::HopLimit), true)
    );
    assert_eq!(gate(false, true, true, false, 2), (Ok(()), true));
}

both!(a_relay_needs_a_live_route_at_release, RouteEntry<u64>);
fn a_relay_needs_a_live_route_at_release<S: TestStore<RouteEntry<u64>>>() {
    let mut store = S::default();
    store.insert(key(1), route(3, 0)).unwrap();
    assert_eq!(release_from(&store, &key(1), TTL - 1, ttl), Some(3));
    assert_eq!(release_from(&store, &key(1), TTL, ttl), None);
    assert_eq!(release_from(&store, &key(2), 0, ttl), None);
}

#[test]
fn a_relay_goes_to_every_permitted_interface() {
    use InterfaceMode::{AccessPoint, Full, Roaming};
    let flags = ModeFlags::default();
    let interfaces = [
        (1, Full, flags),
        (2, AccessPoint, flags),
        (3, Roaming, flags),
    ];
    let targets = |from, only: Option<&[InterfaceId]>| {
        relay_targets(from, interfaces, only).collect::<Vec<_>>()
    };
    assert_eq!(targets(Some(Full), None), [1, 3], "not the access point");
    assert_eq!(
        targets(Some(Roaming), None),
        [1],
        "roaming to roaming is not"
    );
    assert_eq!(targets(Some(Full), Some(&[3])), [3], "only what is owed");
    assert!(targets(None, None).is_empty(), "no live route, no relay");
}
