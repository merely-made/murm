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
