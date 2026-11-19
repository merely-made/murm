//! Behaviour shared by the two transport drivers.

use super::*;
use crate::packet::HeaderType;

#[tokio::test]
async fn transit_precedes_reverse_proofs_and_handles_once() {
    for bridged in [false, true] {
        let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x71; 64]));
        let a = ep.attach_interface();
        let b = ep.attach_interface();
        let c = ep.attach_interface();
        ep.enable_routing();
        let id = AddressHash::from_bytes([0x72; 16]);
        ep.shared.remember_reverse(id, c.id(), a.id());
        let mut proof = single_packet(id, vec![0x71; 48]);
        proof.packet_type = PacketType::Proof;
        if bridged {
            ep.shared.link_transport.lock().unwrap().insert(
                id,
                LinkBridge {
                    from: a.id(),
                    out: b.id(),
                    destination: id,
                    seen: Instant::now(),
                    proof_deadline: None,
                },
            );
        } else {
            proof.header_type = HeaderType::Type2;
            proof.transport = Some(ep.identity().hash());
        }
        route(&ep.shared, a.id(), proof);
        assert!(ep.shared.reverse_table.lock().unwrap().contains_key(&id));
        assert!(a.outbound.queues.pop().is_none());
        assert!(c.outbound.queues.pop().is_none());
        assert_eq!(b.outbound.queues.pop().is_some(), bridged);
    }
}

#[tokio::test]
async fn a_local_type_two_link_request_bypasses_transit_policy() {
    for routing in [false, true] {
        let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x71; 64]));
        let a = ep.attach_interface();
        if routing {
            ep.enable_routing();
        }
        let name = DestinationName::new("retinue", ["local"]);
        let dest = name.destination_hash(ep.identity());
        ep.register(name, b"");
        while a.outbound.queues.pop().is_some() {
            a.outbound.queues.delivery_complete();
        }
        let (pending, mut request) = link::PendingLink::open(
            dest,
            *ep.identity(),
            &[0x73; 64],
            LinkTrailer {
                mode: LinkMode::Aes256Cbc,
                mtu: 500,
            },
        );
        request.header_type = HeaderType::Type2;
        request.transport = Some(ep.identity().hash());
        route(&ep.shared, a.id(), request);
        let proof = a.outbound.queues.pop().expect("local destination accepts");
        assert!(pending.prove(&proof).is_ok());
        assert!(ep.shared.link_transport.lock().unwrap().is_empty());
        assert_eq!(ep.routing_counters().forwarded_packets, 0);
        assert_eq!(ep.routing_counters().policy_rejected, 0);
    }
}

#[tokio::test]
async fn carried_packets_filter_duplicates_only_after_bridge_admission() {
    for context in 0..=u8::MAX {
        let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x71; 64]));
        let a = ep.attach_interface();
        let b = ep.attach_interface();
        let c = ep.attach_interface();
        ep.enable_routing();
        let id = AddressHash::from_bytes([0x73; 16]);
        ep.shared.link_transport.lock().unwrap().insert(
            id,
            LinkBridge {
                from: a.id(),
                out: b.id(),
                destination: id,
                seen: Instant::now(),
                proof_deadline: Some(Instant::now() + Duration::from_secs(10)),
            },
        );
        let mut data = single_packet(id, vec![0x71; 48]);
        data.destination_type = DestinationType::Link;
        data.context = context;
        route(&ep.shared, c.id(), data.clone());
        route(&ep.shared, a.id(), data.clone());
        assert!(b.outbound.queues.pop().is_none());
        ep.shared
            .link_transport
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .proof_deadline = None;
        route(&ep.shared, a.id(), data.clone());
        assert!(b.outbound.queues.pop().is_some());
        b.outbound.queues.delivery_complete();
        route(&ep.shared, b.id(), data);
        let exempt = !crate::node::is_deduplicated_link_context(context);
        assert_eq!(
            a.outbound.queues.pop().is_some(),
            exempt,
            "context {context}"
        );
        assert_eq!(
            ep.routing_counters().forwarded_packets,
            1 + u64::from(exempt)
        );
    }
}
