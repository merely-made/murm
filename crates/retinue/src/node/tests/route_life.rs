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
