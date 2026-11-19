//! Behaviour shared by the two transport drivers.

use super::*;
use crate::transport::{Keyed, ReverseEntry};

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
fn type_two_route_misses_are_counted_once_and_filtered_before_lookup() {
    let mut relay = relay();
    let (_, destination) = pair();
    let mut data = packet(destination.destination());
    data.destination_type = crate::packet::DestinationType::Single;
    data.header_type = HeaderType::Type2;
    data.transport = Some(relay.identity.hash());
    assert!(relay.ingest(1, &data, 1).is_empty());
    assert_eq!(relay.transport_counters().unroutable_packets, 1);
    assert!(relay.ingest(1, &data, 2).is_empty());
    assert_eq!(relay.transport_counters().unroutable_packets, 1);
    relay.ingest(2, &destination.announce(&blob([2; RAND_HASH_LEN]), None), 3);
    assert!(relay.ingest(1, &data, 4).is_empty());
    assert_eq!(relay.transport_counters().unroutable_packets, 1);
    assert_eq!(relay.transport_counters().forwarded_packets, 0);
}

#[test]
fn return_paths_are_transactional_with_output() {
    for state in ["empty", "full", "replace"] {
        for accepted in [false, true] {
            let mut relay = relay();
            let (_, destination) = pair();
            relay.ingest(2, &destination.announce(&blob([2; RAND_HASH_LEN]), None), 0);
            let mut data = packet(destination.destination());
            data.header_type = HeaderType::Type2;
            data.transport = Some(relay.identity.hash());
            let hash = data.hash();
            if state != "empty" {
                for byte in 0..4 {
                    relay
                        .reverse
                        .push(Keyed {
                            key: if state == "replace" && byte == 1 {
                                hash
                            } else {
                                AddressHash::from_bytes([byte; 16])
                            },
                            entry: ReverseEntry {
                                received: 7,
                                outbound: 8,
                                at: u64::from(byte),
                            },
                        })
                        .unwrap();
                }
            }
            let before = relay.reverse.clone();
            let learned = relay.routes[0].entry.learned;
            let mut actions = Actions::new();
            if !accepted {
                for _ in 0..8 {
                    assert!(actions.push(Action::Send {
                        interface: 2,
                        packet: data.clone()
                    }));
                }
            }
            assert!(relay.forward_transport_packet_with(
                1,
                &data,
                10,
                &mut actions,
                |node, actions, action| {
                    let entry = node
                        .reverse
                        .iter()
                        .find(|entry| entry.key == hash)
                        .expect("return path exists before output admission");
                    assert_eq!(
                        (entry.entry.received, entry.entry.outbound, entry.entry.at),
                        (1, 2, 10)
                    );
                    assert_eq!(node.routes[0].entry.used, None, "route not yet marked");
                    actions.push(action)
                },
            ));
            assert_eq!(relay.routes[0].entry.used, accepted.then_some(10));
            assert_eq!(
                relay.routes[0].entry.learned, learned,
                "use moves nothing else"
            );
            assert_eq!(
                relay.transport_counters().forwarded_packets,
                u16::from(accepted)
            );
            if accepted {
                assert_eq!(relay.reverse.len(), if state == "empty" { 1 } else { 4 });
                assert!(relay.reverse.iter().any(|entry| entry.key == hash));
            } else {
                assert_eq!(
                    relay.reverse, before,
                    "{state}: refused send restores the table"
                );
            }
        }
    }
}

#[test]
fn interface_removal_purges_return_paths_and_pending_relays() {
    let mut relay = relay();
    let (source, destination) = pair();
    let lost = destination.announce(&blob([2; RAND_HASH_LEN]), None);
    let kept = source.announce(&blob([3; RAND_HASH_LEN]), None);
    relay.ingest(2, &lost, 0);
    relay.ingest(2, &kept, 0);
    // The pending relay was heard on 1, but its destination is still reachable on 2.
    relay.schedule_rebroadcast(1, lost.clone(), 0, 0);
    assert!(relay.rebroadcasts.answer(
        crate::rebroadcast::Rebroadcast {
            destination: kept.destination,
            packet: kept.clone(),
            interface: 2,
            emitted: 0,
            answer: Some(1),
        },
        0,
        0
    ));
    for (byte, received, outbound) in [(1, 1, 2), (2, 2, 1), (3, 2, 3)] {
        relay
            .reverse
            .push(Keyed {
                key: AddressHash::from_bytes([byte; 16]),
                entry: ReverseEntry {
                    received,
                    outbound,
                    at: 0,
                },
            })
            .unwrap();
    }
    let survivor = relay.reverse[2];
    relay.forget_interface(1);
    assert_eq!(relay.reverse.as_slice(), &[survivor]);
    assert_eq!(relay.next_hop(lost.destination, 1).unwrap().interface, 2);
    assert!(!relay.rebroadcasts.contains(lost.destination));
    let restored = relay.rebroadcasts.pop_due(REBROADCAST_WINDOW).unwrap();
    assert_eq!(
        (restored.destination, restored.interface, restored.answer),
        (kept.destination, 2, None)
    );
    relay.forget_interface(1);
    assert_eq!(relay.reverse.as_slice(), &[survivor]);
}

#[test]
fn interface_removal_purges_cap_queues_by_recorded_interface() {
    use crate::rebroadcast::{AnnounceCap, CapRate, Offer, Rebroadcast};
    let mut relay = relay();
    let on = |byte, interface, answer| Rebroadcast {
        destination: AddressHash::from_bytes([byte; 16]),
        packet: packet(AddressHash::from_bytes([byte; 16])),
        interface,
        emitted: 0,
        answer,
    };
    let rate = CapRate::new(100);
    for iface in [1, 2] {
        relay.set_first_hop_airtime(iface, 100).unwrap();
        let mut cap = AnnounceCap::new(4);
        assert!(matches!(cap.offer(on(0, 2, None), 0, rate), Offer::Send(_)));
        for pending in [on(1, 1, None), on(2, 2, Some(1)), on(3, 2, None)] {
            assert!(matches!(cap.offer(pending, 0, rate), Offer::Queued));
        }
        relay.announce_caps.push((iface, cap)).unwrap();
    }
    relay.forget_interface(1);
    assert_eq!(relay.announce_caps.len(), 1);
    let (iface, cap) = &mut relay.announce_caps[0];
    assert_eq!(*iface, 2);
    assert_eq!(cap.queued(), 1);
    let due = cap.next_due().unwrap();
    assert_eq!(
        cap.pop_due(due, rate).unwrap().destination,
        AddressHash::from_bytes([3; 16])
    );
    assert_eq!(cap.queued(), 0);
}

#[test]
fn transit_precedes_reverse_proofs_and_handles_once() {
    for bridged in [false, true] {
        let mut relay = relay();
        let id = AddressHash::from_bytes([0x72; 16]);
        relay
            .reverse
            .push(Keyed {
                key: id,
                entry: ReverseEntry {
                    received: 3,
                    outbound: 1,
                    at: 0,
                },
            })
            .unwrap();
        let mut proof = packet(id);
        proof.packet_type = PacketType::Proof;
        if bridged {
            assert!(relay.remember_bridge(id, id, 1, 2, 1000, 0));
            relay.bridges[0].entry.proof_deadline = None;
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
        relay.bridges[0].entry.proof_deadline = None;
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

#[test]
fn carried_links_refresh_only_on_accepted_send() {
    for refusal in [
        "accepted",
        "full",
        "hops",
        "size",
        "policy",
        "duplicate",
        "proof",
        "proof-full",
    ] {
        let mut relay = relay();
        let id = AddressHash::from_bytes([0x74; 16]);
        let proves = refusal.starts_with("proof");
        assert!(relay.remember_bridge(id, id, 1, 2, 1000, 10));
        if !proves {
            relay.bridges[0].entry.proof_deadline = None;
        }
        let mut data = packet(id);
        if proves {
            data.packet_type = PacketType::Proof;
            data.context = link::CTX_LRPROOF;
        }
        let from = if proves { 2 } else { 1 };
        let mut actions = Actions::new();
        match refusal {
            "full" | "proof-full" => {
                for _ in 0..8 {
                    actions.push(Action::Send {
                        interface: 3,
                        packet: data.clone(),
                    });
                }
            }
            "hops" => data.hops = relay.transport.max_hops - 1,
            "size" => data.payload = vec![0; relay.logical_mtu as usize],
            "policy" => relay.transport.relay_packets = false,
            "duplicate" => assert!(relay.transit_is_new(&data, 99)),
            _ => {}
        }
        relay.forward_bridged_packet(from, &data, 100, &mut actions);
        let accepted = matches!(refusal, "accepted" | "proof");
        assert_eq!(
            relay.bridges[0].entry.seen,
            if accepted { 100 } else { 10 },
            "{refusal}"
        );
        assert_eq!(
            relay.bridges[0].entry.proof_deadline.is_none(),
            !proves || accepted,
            "{refusal}"
        );
    }
}

#[test]
fn forwarded_count_tracks_only_accepted_output() {
    for refusal in ["accepted", "full", "size", "hops"] {
        let mut relay = relay();
        let id = AddressHash::from_bytes([0x75; 16]);
        assert!(relay.remember_bridge(id, id, 1, 2, 1000, 0));
        relay.bridges[0].entry.proof_deadline = None;
        let mut data = packet(id);
        let mut actions = Actions::new();
        match refusal {
            "full" => {
                for _ in 0..8 {
                    assert!(actions.push(Action::Send {
                        interface: 2,
                        packet: data.clone()
                    }));
                }
            }
            "size" => data.payload = vec![0; 600],
            "hops" => data.hops = relay.transport.max_hops - 1,
            _ => {}
        }
        relay.forward_bridged_packet(1, &data, 1, &mut actions);
        assert_eq!(
            relay.transport_counters().forwarded_packets,
            u16::from(refusal == "accepted"),
            "{refusal}"
        );
    }
}

#[test]
fn short_link_requests_are_bridged_and_carried() {
    for len in [1, 32, 63] {
        let mut relay = relay();
        let (_, destination) = pair();
        relay.ingest(2, &destination.announce(&blob([2; RAND_HASH_LEN]), None), 0);
        let mut request = packet(destination.destination());
        request.destination_type = crate::packet::DestinationType::Single;
        request.packet_type = PacketType::LinkRequest;
        request.header_type = HeaderType::Type2;
        request.transport = Some(relay.identity.hash());
        request.payload = vec![0x76; len];
        let id = request.hash();
        let carried = sent(&relay.ingest(1, &request, 1)).expect("short request is carried");
        assert_eq!(relay.bridges.len(), 1, "short request has a bridge");
        let bridge = relay.bridges[0];
        assert_eq!(
            (bridge.key, bridge.entry.from, bridge.entry.out),
            (id, 1, 2)
        );
        assert_eq!(bridge.entry.destination, request.destination);
        assert!(bridge.entry.proof_deadline.is_some());
        assert!(relay.reverse.is_empty());
        assert_eq!(carried.payload, request.payload);
        assert!(
            link::accept(
                &carried,
                &destination.identity,
                &[0x77; 64],
                LinkTrailer {
                    mode: LinkMode::Aes256Cbc,
                    mtu: 500,
                }
            )
            .is_err()
        );
    }
}

#[test]
fn a_repeated_announce_over_the_hop_ceiling_is_remembered_before_it_is_counted() {
    let mut relay = relay();
    let (_, destination) = pair();
    let mut announce = destination.announce(&blob([4; RAND_HASH_LEN]), None);
    announce.hops = relay.transport.max_hops - 1;
    relay.ingest(1, &announce, 0);
    assert_eq!(relay.transport_counters().hop_limit_dropped, 1);
    // Without a live route the copy is a first sighting and reaches the relay gate again.
    relay.routes.clear();
    relay.ingest(1, &announce, 1);
    assert_eq!(relay.route_count(), 1, "learned again");
    assert_eq!(
        relay.transport_counters().hop_limit_dropped,
        1,
        "the repeat is filtered, not counted again"
    );
    assert_eq!(relay.next_rebroadcast(), None);
}

#[test]
fn a_relay_leaves_on_every_permitted_interface_the_arrival_one_included() {
    let mut relay = relay();
    relay
        .set_interface_mode(3, InterfaceMode::AccessPoint)
        .unwrap();
    relay.poll(0, 2, None);
    let (_, destination) = pair();
    relay.ingest(1, &destination.announce(&blob([6; RAND_HASH_LEN]), None), 0);
    let due = relay.next_rebroadcast().unwrap();
    let mut out: Vec<InterfaceId> = relay
        .poll(due, 2, None)
        .iter()
        .filter_map(|action| match action {
            Action::Send { interface, packet }
                if packet.destination == destination.destination() =>
            {
                Some(*interface)
            }
            _ => None,
        })
        .collect();
    out.sort_unstable();
    assert_eq!(out, [1, 2], "not the access point");
    assert_eq!(
        relay.transport_counters().forwarded_announces,
        2,
        "counted once per accepted send"
    );
}

#[test]
fn with_carriage_off_a_packet_is_not_transit() {
    let mut relay = relay();
    relay.transport.relay_packets = false;
    let id = AddressHash::from_bytes([0x7b; 16]);
    relay
        .reverse
        .push(Keyed {
            key: id,
            entry: ReverseEntry {
                received: 2,
                outbound: 1,
                at: 0,
            },
        })
        .unwrap();
    let reverse = relay.reverse.clone();
    let link_id = AddressHash::from_bytes([0x7d; 16]);
    assert!(relay.remember_bridge(link_id, link_id, 1, 2, 1000, 0));
    relay.bridges[0].entry.proof_deadline = None;
    let bridges = relay.bridges.clone();
    let (_, destination) = pair();
    relay.ingest(2, &destination.announce(&blob([5; RAND_HASH_LEN]), None), 0);
    let counters = relay.transport_counters();

    let mut proof = packet(id);
    proof.packet_type = PacketType::Proof;
    let mut data = packet(destination.destination());
    data.destination_type = crate::packet::DestinationType::Single;
    data.header_type = HeaderType::Type2;
    data.transport = Some(relay.identity.hash());
    for packet in [proof, data, packet(link_id)] {
        assert!(sent(&relay.ingest(1, &packet, 1)).is_none());
    }
    assert_eq!(
        relay.reverse, reverse,
        "a proof's return path is kept, nor a new one recorded"
    );
    assert_eq!(relay.bridges, bridges);
    // The board counts nothing for it.
    assert_eq!(relay.transport_counters(), counters);
}

#[test]
fn reverse_proofs_keep_header_and_obey_hop_ceiling() {
    for hops in [2, 3] {
        let mut relay = relay();
        relay.transport.max_hops = 4;
        let id = AddressHash::from_bytes([0x79; 16]);
        relay
            .reverse
            .push(Keyed {
                key: id,
                entry: ReverseEntry {
                    received: 1,
                    outbound: 2,
                    at: 0,
                },
            })
            .unwrap();
        let mut proof = packet(id);
        proof.packet_type = PacketType::Proof;
        proof.hops = hops;
        let actions = relay.ingest(2, &proof, 1);
        assert!(relay.reverse.is_empty());
        if hops == 2 {
            let mut expected = proof.clone();
            expected.hops += 1;
            assert_eq!(sent(&actions).unwrap().encode(), expected.encode());
        } else {
            assert!(
                actions.is_empty(),
                "proof at the forwarding ceiling is refused"
            );
            assert_eq!(relay.transport_counters().hop_limit_dropped, 1);
        }
    }
}
