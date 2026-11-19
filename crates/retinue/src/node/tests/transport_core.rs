//! Behaviour shared by the two transport drivers.

use super::super::tables::ReverseEntry;
use super::*;

fn relay() -> Node<32, 8, 4, 4> {
    Node::new(
        PrivateIdentity::from_secret_bytes(&[0x71; 64]),
        DestinationName::new("retinue", ["relay"]).name_hash(),
    )
    .with_transport_config(TransportConfig::transit())
}

fn packet(destination: AddressHash) -> Packet {
    Packet {
        header_type: HeaderType::Type1,
        transport: None,
        packet_type: PacketType::Data,
        destination_type: crate::packet::DestinationType::Link,
        destination,
        context: 0,
        payload: vec![0x71; 48],
        ..fixture("announce_appdata.bin")
    }
}

#[test]
fn transit_precedes_reverse_proofs_and_handles_once() {
    for bridged in [false, true] {
        let mut relay = relay();
        let id = AddressHash::from_bytes([0x72; 16]);
        relay
            .reverse
            .push(ReverseEntry {
                packet: id,
                received: 3,
                outbound: 1,
                seen: 0,
            })
            .unwrap();
        let mut proof = packet(id);
        proof.packet_type = PacketType::Proof;
        if bridged {
            assert!(relay.remember_bridge(id, id, 1, 2, 1000, 0));
            relay.bridges[0].proof_deadline = None;
        } else {
            proof.header_type = HeaderType::Type2;
            proof.transport = Some(relay.identity.hash());
        }
        let actions = relay.ingest(1, &proof, 1);
        assert_eq!(
            relay.reverse.len(),
            1,
            "transit leaves the reverse entry alone"
        );
        assert_eq!(actions.len(), usize::from(bridged));
        if bridged {
            assert!(matches!(
                actions.iter().next(),
                Some(Action::Send { interface: 2, .. })
            ));
        }
    }
}

#[test]
fn a_local_type_two_link_request_bypasses_transit_policy() {
    for routing in [false, true] {
        let mut node = relay();
        node.transport.relay_packets = routing;
        let (pending, mut request) = link::PendingLink::open(
            node.destination(),
            *node.identity.public(),
            &[0x73; 64],
            LinkTrailer {
                mode: LinkMode::Aes256Cbc,
                mtu: 500,
            },
        );
        request.header_type = HeaderType::Type2;
        request.transport = Some(node.identity.hash());
        let proof = sent(&node.ingest(1, &request, 0)).expect("local destination accepts");
        assert!(pending.prove(&proof).is_ok());
        assert!(node.bridges.is_empty());
        assert_eq!(node.transport_counters().forwarded_packets, 0);
        assert_eq!(node.transport_counters().unroutable_packets, 0);
    }
}

#[test]
fn carried_packets_filter_duplicates_only_after_bridge_admission() {
    for context in 0..=u8::MAX {
        let mut relay = relay();
        let id = AddressHash::from_bytes([0x73; 16]);
        assert!(relay.remember_bridge(id, id, 1, 2, 1000, 0));
        let mut data = packet(id);
        data.context = context;
        assert!(relay.ingest(3, &data, 1).is_empty());
        assert!(relay.ingest(1, &data, 2).is_empty());
        relay.bridges[0].proof_deadline = None;
        assert!(sent(&relay.ingest(1, &data, 3)).is_some());
        let exempt = !crate::node::is_deduplicated_link_context(context);
        assert_eq!(
            sent(&relay.ingest(2, &data, 4)).is_some(),
            exempt,
            "context {context}"
        );
        assert_eq!(
            relay.transport_counters().forwarded_packets,
            1 + u16::from(exempt)
        );
    }
}
