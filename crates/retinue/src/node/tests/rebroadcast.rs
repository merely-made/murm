//! Relayed announces: jitter, the retry, neighbour suppression and the announce cap.

use super::*;

fn transit(seed: u8, name: &str) -> Node<32, 8, 4, 4> {
    Node::new(
        PrivateIdentity::from_secret_bytes(&[seed; 64]),
        DestinationName::new("retinue", [name]).name_hash(),
    )
    .with_transport_config(TransportConfig::transit())
}

fn announcer(seed: u8) -> Node<32, 8, 4> {
    Node::new(
        PrivateIdentity::from_secret_bytes(&[seed; 64]),
        DestinationName::new("retinue", ["announcer"]).name_hash(),
    )
}

const RETRY: u64 = REBROADCAST_GRACE + REBROADCAST_WINDOW;

#[test]
fn relayed_announces_require_a_live_route_at_release() {
    for expired in [false, true] {
        let destination = announcer(0x6F);
        let mut relay = transit(0x5F, "relay");
        let announce = destination.announce(&blob([0x6F; RAND_HASH_LEN]), None);
        relay.ingest(IFACE, &announce, 1_000);
        let due = relay.next_rebroadcast().expect("scheduled");
        assert!(relay.next_hop(destination.destination(), 1_000).is_some());
        if expired {
            relay.transport.route_ttl = 1;
        } else {
            relay.routes.clear();
        }

        assert!(relay.poll(due.max(1_001), IFACE, None).is_empty());
        assert!(relay.poll(due + RETRY, IFACE, None).is_empty());
        assert_eq!(relay.transport_counters().forwarded_announces, 0);
    }
}

#[test]
fn relayed_announces_set_the_complete_transport_header() {
    let destination = announcer(0x70);
    let mut relay = transit(0x60, "relay");
    let announce = destination.announce(&blob([0x70; RAND_HASH_LEN]), None);
    assert_eq!(announce.header_type, HeaderType::Type1);
    assert_eq!(announce.propagation, crate::packet::Propagation::Broadcast);

    let forwarded = relayed(&mut relay, IFACE, &announce, 0).expect("relayed");
    let decoded = Packet::decode(&forwarded.encode()).expect("wire packet");
    assert_eq!(decoded.header_type, HeaderType::Type2);
    assert_eq!(decoded.propagation, crate::packet::Propagation::Transport);
    assert_eq!(decoded.transport, Some(relay.identity.hash()));
    assert_eq!(decoded.hops, announce.hops + 1);
}

/// RNS holds a relayed announce for a random 0-0.5 s, sends it on every interface including
/// the one it came from, and tries once more after the 5 s grace (`Transport.py` 765-829).
#[test]
fn a_relay_sends_after_a_jitter_on_every_known_interface_then_retries_once() {
    let destination = announcer(0x71);
    let mut relay = transit(0x61, "relay");
    let announce = destination.announce(&blob([0x71; RAND_HASH_LEN]), None);

    assert!(sent(&relay.ingest(IFACE + 1, &announce, 1_000)).is_none());
    let due = relay.next_rebroadcast().expect("scheduled");
    assert!((1_000..=1_000 + REBROADCAST_WINDOW).contains(&due));
    if due > 1_000 {
        assert!(relay.poll(due - 1, IFACE, None).is_empty());
    }
    let first = relay.poll(due, IFACE, None);
    let Some(Action::Send { interface, packet }) = first.iter().next() else {
        panic!("the rebroadcast is due");
    };
    assert_eq!(*interface, IFACE + 1, "out the interface it was heard on");
    assert_eq!(
        sends(&first),
        [IFACE + 1, IFACE],
        "and on the one the poll named"
    );
    assert_eq!(packet.hops, 1);
    assert_eq!(packet.transport, Some(relay.identity.hash()));

    assert_eq!(relay.next_rebroadcast(), Some(due + RETRY));
    assert!(relay.poll(due + RETRY - 1, IFACE, None).is_empty());
    assert_eq!(
        sent(&relay.poll(due + RETRY, IFACE, None)).as_ref(),
        Some(packet)
    );
    assert_eq!(relay.next_rebroadcast(), None, "one retry only");
    // Counted once per accepted send (D23).
    assert_eq!(relay.transport_counters().forwarded_announces, 4);
}

/// Two neighbours heard relaying the announce at our hop count end our retry
/// (`Transport.py` 2183-2194). Before our first send they only count, as in RNS.
#[test]
fn heard_neighbour_rebroadcasts_cancel_our_retry() {
    let destination = announcer(0x72);
    let mut relay = transit(0x62, "relay");
    let announce = destination.announce(&blob([0x72; RAND_HASH_LEN]), None);
    let first_neighbour = relayed(&mut transit(0x63, "n1"), IFACE, &announce, 0).unwrap();
    let second_neighbour = relayed(&mut transit(0x64, "n2"), IFACE, &announce, 0).unwrap();

    relay.ingest(IFACE, &announce, 0);
    let due = relay.next_rebroadcast().unwrap();
    relay.ingest(IFACE, &first_neighbour, due);
    assert!(
        sent(&relay.poll(due, IFACE, None)).is_some(),
        "a neighbour heard before our first send does not cancel it"
    );

    relay.ingest(IFACE, &second_neighbour, due + 1);
    assert_eq!(
        relay.next_rebroadcast(),
        None,
        "the second neighbour ends it"
    );
    assert!(relay.poll(due + RETRY, IFACE, None).is_empty());
    let counters = relay.transport_counters();
    assert_eq!(counters.suppressed_rebroadcasts, 1);
    assert_eq!(counters.forwarded_announces, 1);
}

/// Our rebroadcast heard passed on with one more hop ends the retry (`Transport.py`
/// 2196-2201).
#[test]
fn our_rebroadcast_passed_on_cancels_the_retry() {
    let destination = announcer(0x73);
    let mut relay = transit(0x65, "relay");
    let announce = destination.announce(&blob([0x73; RAND_HASH_LEN]), None);
    let ours = relayed(&mut relay, IFACE, &announce, 0).unwrap();
    let onward = relayed(&mut transit(0x66, "downstream"), IFACE, &ours, 1).unwrap();
    assert_eq!(onward.hops, ours.hops + 1);

    assert!(relay.next_rebroadcast().is_some());
    relay.ingest(IFACE, &onward, 600);
    assert_eq!(relay.next_rebroadcast(), None);
    assert_eq!(relay.transport_counters().suppressed_rebroadcasts, 1);
}

/// With the interface's airtime known, relayed announces keep to 2 % of it: the rest wait in
/// the queue rather than being dropped (`Transport.py` 1522-1585).
#[test]
fn the_announce_cap_queues_rather_than_drops() {
    let mut relay = transit(0x67, "relay");
    relay
        .set_first_hop_airtime(IFACE, first_hop_airtime(62_500))
        .unwrap();
    for seed in 0x74..0x77 {
        let announce = announcer(seed).announce(&blob([seed; RAND_HASH_LEN]), None);
        relay.ingest(IFACE, &announce, 0);
    }

    assert_eq!(relay.poll(REBROADCAST_WINDOW, IFACE, None).len(), 1);
    assert_eq!(relay.transport_counters().capped_announces, 2);
    let mut at = REBROADCAST_WINDOW;
    for _ in 0..2 {
        let next = relay.next_rebroadcast().unwrap();
        assert!(
            next > at + 1_000,
            "about a second of budget per 2 % announce"
        );
        assert!(relay.poll(next - 1, IFACE, None).is_empty());
        assert_eq!(relay.poll(next, IFACE, None).len(), 1);
        at = next;
    }
    let counters = relay.transport_counters();
    assert_eq!(counters.forwarded_announces, 3);
    assert_eq!(counters.dropped_announces, 0);
}

/// A flood cannot displace announces still waiting for their first send.
#[test]
fn a_full_rebroadcast_table_refuses_rather_than_displaces() {
    let mut relay = transit(0x68, "relay");
    for seed in 0..=REBROADCAST_SLOTS as u8 {
        let announce = announcer(0x80 + seed).announce(&blob([seed; RAND_HASH_LEN]), None);
        relay.ingest(IFACE, &announce, 0);
    }
    assert_eq!(relay.transport_counters().refused_rebroadcasts, 1);
    assert_eq!(
        relay.poll(REBROADCAST_WINDOW, IFACE, None).len(),
        3,
        "the refused newcomer holds the fourth live route without a pending relay"
    );
}

/// A destination re-announcing while its relay waits for the retry takes over that retry, so
/// repeats of a few destinations cannot keep the bounded table full of unsent entries.
#[test]
fn re_announcing_cannot_pin_the_rebroadcast_table() {
    let mut relay = transit(0x69, "relay");
    let announce = |seed: u8, ordinal| {
        let blob = AnnounceBlob::mint([seed; 5], ordinal).unwrap();
        announcer(0x90 + seed).announce(&blob, None)
    };
    for seed in 0..REBROADCAST_SLOTS as u8 {
        relay.ingest(IFACE, &announce(seed, 1), 0);
    }
    let first = relay.poll(REBROADCAST_WINDOW, IFACE, None);
    assert_eq!(first.len(), 4, "only destinations with live routes can relay");
    for seed in 0..REBROADCAST_SLOTS as u8 {
        relay.ingest(IFACE, &announce(seed, 2), REBROADCAST_WINDOW + 1);
    }

    let newcomer = announcer(0x9F).announce(&blob([0x9F; RAND_HASH_LEN]), None);
    relay.ingest(IFACE, &newcomer, REBROADCAST_WINDOW + 2);
    assert_eq!(relay.transport_counters().refused_rebroadcasts, 0);
}

/// A board whose poll holds one action, for relays that do not fit.
fn narrow(seed: u8) -> Node<32, 1, 4, 4> {
    Node::new(
        PrivateIdentity::from_secret_bytes(&[seed; 64]),
        DestinationName::new("retinue", ["narrow"]).name_hash(),
    )
    .with_transport_config(TransportConfig::transit())
}

/// The interfaces a set of actions sends on, in order.
fn sends<const N: usize>(actions: &Actions<N>) -> alloc::vec::Vec<InterfaceId> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Send { interface, .. } => Some(*interface),
            _ => None,
        })
        .collect()
}

/// A relay sends on as many interfaces as one poll holds and keeps the rest owed for the next
/// (D22). An interface whose cap queues it is owed nothing more.
#[test]
fn a_relay_that_does_not_fit_carries_its_remainder() {
    use crate::rebroadcast::{AnnounceCap, CapRate, Offer, Rebroadcast};
    let mut relay = narrow(0x6B);
    relay.poll(0, 2, None);
    relay.poll(0, 3, None);
    // Interface 3's budget is spent for a long while.
    let airtime = 1_000_000;
    relay.set_first_hop_airtime(3, airtime).unwrap();
    let spender = announcer(0x7A).announce(&blob([0x7A; RAND_HASH_LEN]), None);
    let mut cap = AnnounceCap::new(QUEUED_ANNOUNCES);
    let spent = Rebroadcast {
        destination: spender.destination,
        packet: spender,
        interface: 3,
        emitted: 0,
        answer: None,
    };
    assert!(matches!(
        cap.offer(spent, 0, CapRate::new(airtime)),
        Offer::Send(_)
    ));
    relay.announce_caps.push((3, cap)).unwrap();

    let announce = announcer(0x7B).announce(&blob([0x7B; RAND_HASH_LEN]), None);
    relay.ingest(1, &announce, 0);
    let due = relay.next_rebroadcast().unwrap();
    assert_eq!(
        sends(&relay.poll(due, 1, None)),
        [2],
        "the first named fits"
    );
    assert_eq!(
        relay.next_rebroadcast(),
        Some(due),
        "the remainder is due at once"
    );
    assert_eq!(sends(&relay.poll(due, 1, None)), [1]);
    let counters = relay.transport_counters();
    assert_eq!(counters.capped_announces, 1, "3 queued it instead");
    assert_eq!(counters.forwarded_announces, 2);
    assert_eq!(relay.next_rebroadcast(), Some(due + RETRY), "nothing owed");
    assert!(relay.poll(due, 1, None).is_empty());
}

/// An owed interface that is forgotten is owed nothing; forgetting the interface a relay was
/// heard on purges the relay (D12).
#[test]
fn forgetting_an_interface_settles_what_a_relay_owes_it() {
    for heard_on_forgotten in [false, true] {
        let mut relay = narrow(0x6C);
        relay.poll(0, 2, None);
        relay.poll(0, 3, None);
        let source = announcer(0x7C);
        relay.ingest(1, &source.announce(&blob([0x7C; RAND_HASH_LEN]), None), 0);
        let due = relay.next_rebroadcast().unwrap();
        assert_eq!(sends(&relay.poll(due, 1, None)), [2]);
        relay.forget_interface(if heard_on_forgotten { 1 } else { 3 });
        if heard_on_forgotten {
            assert!(!relay.rebroadcasts.contains(source.destination()));
            assert!(relay.poll(due, 2, None).is_empty());
        } else {
            assert_eq!(sends(&relay.poll(due, 1, None)), [1]);
            assert!(relay.poll(due, 1, None).is_empty(), "nothing more owed");
        }
    }
}

/// The table holds the first interfaces named. One named after it is full is served, its
/// announces learned and relayed, but relays do not go out on it (D34).
#[test]
fn an_interface_past_the_table_is_served_but_not_a_relay_target() {
    let mut relay = transit(0x6D, "relay");
    let extra = KNOWN_INTERFACES as InterfaceId + 1;
    for iface in 1..extra {
        relay.poll(0, iface, None);
    }
    let source = announcer(0x7D);
    relay.ingest(
        extra,
        &source.announce(&blob([0x7D; RAND_HASH_LEN]), None),
        0,
    );
    assert_eq!(relay.route_to(source.destination(), 0).unwrap().0, extra);
    let due = relay.next_rebroadcast().unwrap();
    let mut out = sends(&relay.poll(due, extra, None));
    out.sort_unstable();
    assert_eq!(out, (1..extra).collect::<alloc::vec::Vec<_>>());
}

/// A shell that wakes at `next_rebroadcast` and polls always makes progress: the next wake is
/// later, or there is none, so a deadline-driven board cannot spin.
#[test]
fn polling_at_the_next_rebroadcast_moves_it_on() {
    let mut relay = transit(0x6A, "relay");
    relay
        .set_first_hop_airtime(IFACE, first_hop_airtime(62_500))
        .unwrap();
    for seed in 0xA0..0xA6 {
        let announce = announcer(seed).announce(&blob([seed; RAND_HASH_LEN]), None);
        relay.ingest(IFACE, &announce, 0);
    }
    let mut wakes = 0;
    while let Some(at) = relay.next_rebroadcast() {
        relay.poll(at, IFACE, None);
        assert!(relay.next_rebroadcast().is_none_or(|next| next > at));
        wakes += 1;
        assert!(wakes < 64, "bounded");
    }
    let counters = relay.transport_counters();
    assert!(counters.forwarded_announces >= 6, "each sent at least once");
    assert_eq!(counters.dropped_announces, 0);
}
