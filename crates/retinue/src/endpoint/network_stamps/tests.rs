use super::*;
use crate::announce::{self, AnnounceBlob};
use crate::endpoint::router::route;
use crate::packet::PacketType;

#[tokio::test]
async fn enforcement_rejects_unsigned_stamp_metadata_before_learning_or_publication() {
    let ep = Endpoint::new(crate::PrivateIdentity::from_secret_bytes(&[1; 64]));
    let wire = ep.attach_interface();
    ep.set_network_stamp_policy(NetworkStampPolicy {
        enforce: true,
        destination_cost: StampCost::ZERO,
        ..Default::default()
    });
    let peer = crate::PrivateIdentity::from_secret_bytes(&[2; 64]);
    let name = DestinationName::new("retinue", ["stamp-admission"]);
    let blob = AnnounceBlob::from_wire([3; 10]);
    let plain = announce::build(&peer, name.name_hash(), &blob, None, b"authenticated-app");
    let destination = plain.destination;
    route(&ep.shared, wire.id(), plain);
    assert!(ep.resolve(destination).is_none());
    assert!(ep.route_to(destination).is_none());
    assert!(ep.destination_link_cost(destination).is_none());
    let signed = announce::build_stamped(
        &peer,
        name.name_hash(),
        &blob,
        None,
        b"authenticated-app",
        NetworkStamp {
            stamp: [4; 14],
            link_cost: StampCost::from_fp(896),
        },
    );
    route(&ep.shared, wire.id(), signed);
    assert_eq!(ep.resolve(destination), Some(*peer.public()));
    assert_eq!(
        ep.destination_link_cost(destination),
        Some(StampCost::from_fp(896))
    );
    assert_eq!(
        ep.next_announcement().await.unwrap().app_data,
        b"authenticated-app"
    );
}

#[tokio::test]
async fn a_rejected_unstamped_path_does_not_poison_the_same_target_tag() {
    let ep = Endpoint::new(crate::PrivateIdentity::from_secret_bytes(&[5; 64]));
    let name = DestinationName::new("retinue", ["stamped-path"]);
    let destination = name.destination_hash(ep.identity());
    ep.register(name, b"path-app");
    let wire = ep.attach_interface();
    ep.set_network_stamp_policy(NetworkStampPolicy {
        enforce: true,
        path_cost: StampCost::ZERO,
        ..Default::default()
    });
    let tag = [6; 16];
    let request = crate::path::path_request(destination, &tag);
    route(&ep.shared, wire.id(), request);
    assert!(wire.outbound.queues.pop().is_none());
    let request = crate::path::stamped_path_request(
        destination,
        AddressHash::from_bytes([7; 16]),
        &tag,
        &[8; 14],
    );
    route(&ep.shared, wire.id(), request);
    let response = wire
        .outbound
        .queues
        .pop()
        .expect("valid retry was answered");
    assert_eq!(response.packet_type, PacketType::Announce);
    assert_eq!(response.destination, destination);
}

#[tokio::test]
async fn exhausted_generation_cannot_fall_back_to_unstamped_emission_or_leak_slots() {
    let ep = Endpoint::new(crate::PrivateIdentity::from_secret_bytes(&[9; 64]));
    let wire = ep.attach_interface();
    ep.set_network_stamp_policy(NetworkStampPolicy {
        stamp_destinations: true,
        always_stamp_links: true,
        max_attempts: 0,
        ..Default::default()
    });
    let name = DestinationName::new("retinue", ["no-work-budget"]);
    let destination = name.destination_hash(ep.identity());
    ep.register(name, b"pending");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !ep.shared.network_stamps.lock().unwrap().pending.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("zero-work job releases its slot");
    assert!(wire.outbound.queues.pop().is_none());
    assert!(
        ep.shared.path_response(destination).is_none(),
        "path response also requires a stamp"
    );
    assert_eq!(
        link_seed(&ep.shared, destination).await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(ep.shared.network_stamps.lock().unwrap().pending.is_empty());
}

#[tokio::test]
async fn cache_capacity_is_rechecked_when_concurrent_workers_finish_after_a_shrink() {
    for final_capacity in [1, 0] {
        let ep = Endpoint::new(crate::PrivateIdentity::from_secret_bytes(&[10; 64]));
        let wire = ep.attach_interface();
        let mut policy = NetworkStampPolicy {
            stamp_destinations: true,
            destination_value: StampCost::ZERO,
            destination_capacity: 2,
            generation_capacity: 2,
            ..Default::default()
        };
        ep.set_network_stamp_policy(policy);
        ep.register(
            DestinationName::new("retinue", ["concurrent-first"]),
            b"first",
        );
        ep.register(
            DestinationName::new("retinue", ["concurrent-second"]),
            b"second",
        );
        assert_eq!(ep.shared.network_stamps.lock().unwrap().pending.len(), 2);
        policy.destination_capacity = final_capacity;
        ep.set_network_stamp_policy(policy);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !ep.shared.network_stamps.lock().unwrap().pending.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("workers finish");
        assert_eq!(
            ep.shared.network_stamps.lock().unwrap().own.len(),
            final_capacity
        );
        let mut emitted = 0;
        while wire.outbound.queues.pop().is_some() {
            emitted += 1;
        }
        assert_eq!(
            emitted, final_capacity,
            "no over-capacity destination was emitted"
        );
    }
}
