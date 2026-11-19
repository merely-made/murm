//! Route life as stock RNS 1.6.0: what marks a route used, and what moves its expiry.

use super::*;

/// An accepted announce writes a new route, keeping the used mark of the one it replaces
/// (RNS `Transport.py` 2510, 2726). A new destination starts never used.
#[tokio::test]
async fn an_announce_that_replaces_a_route_keeps_its_used_mark() {
    let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x64; 64]));
    let a = ep.attach_interface();
    let entry = |dest| ep.shared.path_table.lock().unwrap()[&dest];
    let [dest, fresh] = [0xB4, 0xB5].map(|n| AddressHash::from_bytes([n; 16]));
    let aged = Instant::now() - Duration::from_secs(60);
    ep.shared.learn_path_at(dest, a.id(), 1, None, aged);
    assert_eq!(entry(dest).used, None, "a new route is unused");
    ep.shared
        .queue_single(dest, single_packet(dest, vec![1; 64]));
    let marked = entry(dest).used;
    assert!(marked.is_some());
    let later = aged + Duration::from_secs(1);
    ep.shared.learn_path_at(dest, a.id(), 2, None, later);
    let route = entry(dest);
    assert_eq!((route.learned, route.used, route.hops), (later, marked, 2));
    ep.shared.learn_path_at(fresh, a.id(), 1, None, later);
    assert_eq!(entry(fresh).used, None);
}

/// Carrying a packet across a bridge marks the route of the bridge's destination used once
/// the send is accepted (RNS `Transport.py` 2440), and moves nothing else.
#[tokio::test]
async fn carrying_across_a_bridge_marks_the_destination_route_used() {
    for accepted in [false, true] {
        let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x73; 64]));
        let a = ep.attach_interface();
        let b = ep.attach_interface();
        ep.enable_routing();
        let dest = AddressHash::from_bytes([0x7d; 16]);
        let aged = Instant::now() - Duration::from_secs(10);
        ep.shared.learn_path_at(dest, b.id(), 0, None, aged);
        let id = AddressHash::from_bytes([0x74; 16]);
        ep.shared.link_transport.lock().unwrap().insert(
            id,
            LinkBridge {
                from: a.id(),
                out: b.id(),
                destination: dest,
                seen: Instant::now(),
                proof_deadline: None,
            },
        );
        let mut data = single_packet(id, vec![0x73; 48]);
        data.destination_type = DestinationType::Link;
        if !accepted {
            while ep
                .shared
                .try_send_on_class(b.id(), data.clone(), TrafficClass::Transit)
            {}
        }
        route(&ep.shared, a.id(), data);
        let route = ep.shared.path_table.lock().unwrap()[&dest];
        assert_eq!(route.used.is_some(), accepted, "accepted: {accepted}");
        assert_eq!(route.learned, aged);
    }
}
