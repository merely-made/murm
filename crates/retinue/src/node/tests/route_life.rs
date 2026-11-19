//! Route life as stock RNS 1.6.0: what marks a route used, and what moves its expiry.

use super::routes::used;
use super::*;

fn relay() -> Node<32, 8, 4, 4> {
    Node::new(
        PrivateIdentity::from_secret_bytes(&[0x73; 64]),
        DestinationName::new("retinue", ["relay"]).name_hash(),
    )
    .with_transport_config(TransportConfig::transit())
}

/// A link packet for the carried link `link_id`.
fn link_packet(link_id: AddressHash) -> Packet {
    Packet {
        header_type: HeaderType::Type1,
        transport: None,
        packet_type: PacketType::Data,
        destination_type: crate::packet::DestinationType::Link,
        destination: link_id,
        context: 0,
        payload: vec![0x73; 48],
        ..fixture("announce_appdata.bin")
    }
}

/// An accepted announce writes a new route, keeping the used mark of the one it replaces
/// (RNS `Transport.py` 2510, 2726). A new destination starts never used.
#[test]
fn an_announce_that_replaces_a_route_keeps_its_used_mark() {
    let (mut node, peer) = pair();
    node.ingest(IFACE, &peer.announce(&blob([0x60; RAND_HASH_LEN]), None), 0);
    assert_eq!(
        used(&node, peer.destination()),
        Some(None),
        "a new route is unused"
    );
    node.open_link(peer.destination(), IFACE, &[0x61; 64], 5);
    node.ingest(
        IFACE,
        &peer.announce(&blob([0x62; RAND_HASH_LEN]), None),
        10,
    );
    let route = node
        .routes
        .iter()
        .find(|r| r.key == peer.destination())
        .unwrap()
        .entry;
    assert_eq!((route.learned, route.used), (10, Some(5)));
}

/// Carrying a packet across a bridge marks the route of the bridge's destination used once
/// the send is accepted (RNS `Transport.py` 2440), and moves nothing else.
#[test]
fn carrying_across_a_bridge_marks_the_destination_route_used() {
    for accepted in [false, true] {
        let mut relay = relay();
        let (_, destination) = pair();
        relay.ingest(2, &destination.announce(&blob([2; RAND_HASH_LEN]), None), 0);
        let id = AddressHash::from_bytes([0x74; 16]);
        assert!(relay.remember_bridge(id, destination.destination(), 1, 2, 1000, 10));
        relay.bridges[0].entry.proof_deadline = None;
        let data = link_packet(id);
        let mut actions = Actions::new();
        if !accepted {
            while actions.push(Action::Send {
                interface: 3,
                packet: data.clone(),
            }) {}
        }
        relay.forward_bridged_packet(1, &data, 100, &mut actions);
        assert_eq!(
            used(&relay, destination.destination()),
            Some(accepted.then_some(100)),
            "accepted: {accepted}"
        );
        assert_eq!(relay.routes[0].entry.learned, 0);
    }
}

/// The relay's route to a destination whose link it carried, once the destination's signed
/// proof has come back and its send was `accepted` or refused.
fn route_after_a_carried_proof(accepted: bool) -> crate::transport::RouteEntry<u64> {
    let mut relay = relay();
    let (mut source, mut destination) = pair();
    let announce = destination.announce(&blob([0x65; RAND_HASH_LEN]), None);
    source.ingest(IFACE, &relayed(&mut relay, 2, &announce, 0).unwrap(), 1);
    relay
        .set_interface_mode(2, InterfaceMode::AccessPoint)
        .unwrap();
    let request = sent(
        &source
            .open_link(destination.destination(), IFACE, &[0x66; 64], 10)
            .unwrap(),
    )
    .unwrap();
    let forwarded = sent(&relay.ingest(1, &request, 10)).unwrap();
    let proof = sent(&destination.ingest(IFACE, &forwarded, 11)).unwrap();
    let mut actions = Actions::new();
    if !accepted {
        while actions.push(Action::Send {
            interface: 3,
            packet: proof.clone(),
        }) {}
    }
    assert!(relay.forward_bridged_packet(2, &proof, 20, &mut actions));
    relay
        .routes
        .iter()
        .find(|r| r.key == destination.destination())
        .unwrap()
        .entry
}

/// Carrying a link proof whose signature this node verified proves the destination alive
/// once the proof is sent (RNS `Transport.py` 2928-2938): the route's life starts over in its
/// interface's current mode. A proof passed on the side check alone, the destination's
/// identity unknown, marks the route used and moves nothing else.
#[test]
fn a_verified_carried_link_proof_proves_its_destination_alive() {
    let route = route_after_a_carried_proof(true);
    assert_eq!(
        (route.learned, route.used, route.mode),
        (20, Some(20), InterfaceMode::AccessPoint)
    );
    let route = route_after_a_carried_proof(false);
    assert_eq!(
        (route.learned, route.used, route.mode),
        (0, Some(10), InterfaceMode::Full),
        "a refused send proves nothing"
    );

    let mut relay = relay();
    let destination = AddressHash::from_bytes([0x75; 16]);
    relay.learn_route(destination, 2, 1, None, 0);
    let id = AddressHash::from_bytes([0x76; 16]);
    assert!(relay.remember_bridge(id, destination, 1, 2, 1000, 10));
    let mut proof = link_packet(id);
    proof.packet_type = PacketType::Proof;
    proof.context = link::CTX_LRPROOF;
    relay.forward_bridged_packet(2, &proof, 100, &mut Actions::new());
    assert_eq!(relay.bridges[0].entry.proof_deadline, None, "carried");
    let route = relay.routes[0].entry;
    assert_eq!(
        (route.learned, route.used),
        (0, Some(100)),
        "an unverified proof only marks"
    );
}

/// Establishing a link as its initiator proves the destination alive (RNS `Link.py` 513,
/// `Transport.py` 3632-3641): the route's life starts over in its interface's current mode,
/// and it is marked used.
#[test]
fn an_own_link_proves_its_destination_alive() {
    let (mut source, mut destination) = pair();
    let peer = destination.destination();
    source.ingest(
        IFACE,
        &destination.announce(&blob([0x63; RAND_HASH_LEN]), None),
        0,
    );
    source
        .set_interface_mode(IFACE, InterfaceMode::Roaming)
        .unwrap();
    let request = sent(&source.open_link(peer, IFACE, &[0x64; 64], 100).unwrap()).unwrap();
    let proof = sent(&destination.ingest(IFACE, &request, 101)).unwrap();
    assert!(link_up(&source.ingest(IFACE, &proof, 200)).is_some());
    let route = source.routes[0].entry;
    assert_eq!(
        (route.learned, route.used, route.mode),
        (200, Some(200), InterfaceMode::Roaming)
    );
    assert!(source.next_hop(peer, 200 + ROAMING_ROUTE_TTL - 1).is_some());
    assert!(source.next_hop(peer, 200 + ROAMING_ROUTE_TTL).is_none());
}

/// A full table drops, after the routes past their lifetime, the never-used routes, the one
/// that would expire soonest first, and then the route used longest ago (D30).
#[test]
fn a_full_table_drops_never_used_then_least_recently_used() {
    let mut node = Node::<16, 8, 4, 3>::new(
        PrivateIdentity::from_secret_bytes(&[0x77; 64]),
        DestinationName::new("retinue", ["full"]).name_hash(),
    );
    let peers: Vec<_> = (1..=6_u8)
        .map(|seed| {
            Node::<16, 8, 4, 3>::new(
                PrivateIdentity::from_secret_bytes(&[seed; 64]),
                DestinationName::new("retinue", ["peer"]).name_hash(),
            )
        })
        .collect();
    let learn = |node: &mut Node<16, 8, 4, 3>, n: usize, at| {
        let announce = peers[n].announce(&blob([n as u8 + 1; RAND_HASH_LEN]), None);
        node.ingest(IFACE, &announce, at);
    };
    let held = |node: &Node<16, 8, 4, 3>| {
        let mut held: Vec<_> = (0..6)
            .filter(|n| used(node, peers[*n].destination()).is_some())
            .collect();
        held.sort_unstable();
        held
    };
    for n in 0..3 {
        learn(&mut node, n, n as u64);
    }
    node.open_link(peers[0].destination(), IFACE, &[0x78; 64], 5);
    learn(&mut node, 3, 6);
    assert_eq!(
        held(&node),
        [0, 2, 3],
        "the never-used route learned first goes"
    );
    node.open_link(peers[2].destination(), IFACE, &[0x79; 64], 7);
    learn(&mut node, 4, 8);
    assert_eq!(
        held(&node),
        [0, 2, 4],
        "a never-used route goes before used ones"
    );
    node.open_link(peers[4].destination(), IFACE, &[0x7A; 64], 9);
    learn(&mut node, 5, 10);
    assert_eq!(held(&node), [2, 4, 5], "then the route used longest ago");
    assert_eq!(node.transport_counters().evicted_routes, 3);
}

/// The board keeps a never-used route until it expires: it takes no linger (D29).
#[test]
fn a_never_used_route_lives_to_its_expiry() {
    let (mut node, peer) = pair();
    node.ingest(IFACE, &peer.announce(&blob([0x67; RAND_HASH_LEN]), None), 0);
    let late = DEFAULT_ROUTE_TTL - 1;
    assert_eq!(
        node.route_to(peer.destination(), late)
            .map(|(_, hops)| hops),
        Some(0)
    );
    assert_eq!(used(&node, peer.destination()), Some(None));
    let _ = node.poll(late, IFACE, None);
    assert_eq!(node.route_count(), 1, "a sweep keeps it too");
    assert!(
        node.route_to(peer.destination(), DEFAULT_ROUTE_TTL)
            .is_none()
    );
}
