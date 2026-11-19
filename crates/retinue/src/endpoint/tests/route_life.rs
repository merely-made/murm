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

/// Carrying a link proof whose signature this node verified proves the destination alive
/// once the proof is sent (RNS `Transport.py` 2928-2938): the route's life starts over in its
/// interface's current mode. A proof passed on the side check alone, the destination's
/// identity unknown, marks the route used and moves nothing else.
#[tokio::test]
async fn a_verified_carried_link_proof_proves_its_destination_alive() {
    for (known, accepted) in [(true, true), (true, false), (false, true)] {
        let (ep, a, b, responder, _, request) = super::transit::transit_fixture();
        let dest = request.destination;
        if !known {
            ep.shared.address_book.lock().unwrap().forget(dest);
        }
        let aged = Instant::now() - Duration::from_secs(60);
        ep.shared
            .path_table
            .lock()
            .unwrap()
            .get_mut(&dest)
            .unwrap()
            .learned = aged;
        assert!(ep.set_interface_mode(b.id(), InterfaceMode::AccessPoint));
        route(&ep.shared, a.id(), request);
        let carried = b.outbound.queues.pop().unwrap();
        b.outbound.queues.delivery_complete();
        let trailer = link::request_trailer(&carried).unwrap().unwrap();
        let (_, proof) = link::accept(&carried, &responder, &[0x99; 64], trailer).unwrap();
        if !accepted {
            while ep
                .shared
                .try_send_on_class(a.id(), proof.clone(), TrafficClass::Transit)
            {}
        }
        route(&ep.shared, b.id(), proof);
        let route = ep.shared.path_table.lock().unwrap()[&dest];
        let case = std::format!("known {known}, accepted {accepted}");
        if known && accepted {
            assert!(route.learned > aged, "{case}");
            assert_eq!(route.used, Some(route.learned), "{case}");
            assert_eq!(route.mode, InterfaceMode::AccessPoint, "{case}");
        } else {
            assert_eq!(route.learned, aged, "{case}");
            assert_eq!(route.mode, InterfaceMode::default(), "{case}");
        }
    }
}

/// Establishing a link as its initiator proves the destination alive (RNS `Link.py` 513,
/// `Transport.py` 3632-3641): the route's life starts over in its interface's current mode,
/// and it is marked used.
#[tokio::test]
async fn an_own_link_proves_its_destination_alive() {
    let ep = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x65; 64]));
    let a = ep.attach_interface();
    let responder = PrivateIdentity::from_secret_bytes(&[0x66; 64]);
    let (_, announce) = freshness_announce(&responder, "alive", 0, 1, 1, 0);
    let dest = announce.destination;
    let aged = Instant::now() - Duration::from_secs(60);
    ep.shared.learn_path_at(dest, a.id(), 1, None, aged);
    assert!(ep.set_interface_mode(a.id(), InterfaceMode::Roaming));
    let answer = async {
        let request = loop {
            if let Some(packet) = a.outbound.queues.pop() {
                break packet;
            }
            tokio::task::yield_now().await;
        };
        a.outbound.queues.delivery_complete();
        let trailer = link::request_trailer(&request).unwrap().unwrap();
        let (_, proof) = link::accept(&request, &responder, &[0x67; 64], trailer).unwrap();
        route(&ep.shared, a.id(), proof);
    };
    let (established, ()) = tokio::join!(ep.establish(dest, *responder.public()), answer);
    established.unwrap();
    let route = ep.shared.path_table.lock().unwrap()[&dest];
    assert!(route.learned > aged);
    assert_eq!(route.used, Some(route.learned));
    assert_eq!(route.mode, InterfaceMode::Roaming);
}
