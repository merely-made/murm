use super::*;
use crate::network_stamp::{self, NetworkStampPolicy};
use crate::stamp::StampCost;

fn policy() -> NetworkStampPolicy {
    NetworkStampPolicy {
        enforce: true,
        stamp_destinations: true,
        destination_cost: StampCost::from_fp(640),
        destination_value: StampCost::from_fp(640),
        path_cost: StampCost::from_fp(640),
        path_value: StampCost::from_fp(640),
        link_cost: StampCost::from_fp(640),
        max_attempts: 10_000,
        ..NetworkStampPolicy::default()
    }
}

fn random() -> impl FnMut(&mut [u8]) {
    let mut counter = 0_u64;
    move |out: &mut [u8]| {
        counter += 1;
        use sha2::{Digest, Sha256};
        let block = Sha256::digest(counter.to_be_bytes());
        out.copy_from_slice(&block[..out.len()]);
    }
}

fn provision<const P: usize, const A: usize, const L: usize, const R: usize>(
    node: &mut Node<P, A, L, R>,
) {
    node.set_network_stamp_policy(policy());
    let stamp = network_stamp::generate_detached(
        &node.destination_stamp_material(),
        policy().destination_value,
        10_000,
        random(),
        || false,
    )
    .unwrap();
    assert!(node.set_destination_stamp(stamp));
}

#[test]
fn unstamped_announce_does_not_poison_the_stamped_retry() {
    let (mut source, mut receiver) = pair();
    receiver.set_network_stamp_policy(policy());
    let emission = blob([0x31; RAND_HASH_LEN]);
    let plain = source.announce(&emission, None);
    assert!(receiver.ingest(IFACE, &plain, 0).is_empty());
    assert_eq!(receiver.route_count(), 0);
    assert!(!receiver.peers().knows(source.destination()));
    assert_eq!(receiver.transport_counters().refused_stamps, 1);

    provision(&mut source);
    source.try_set_app_data(b"authenticated app data").unwrap();
    let packet = source
        .try_announce(&emission, Some(&[0x42; RATCHET_LEN]))
        .unwrap();
    assert!(!receiver.ingest(IFACE, &packet, 1).is_empty());
    let peer = receiver.peers().resolve(source.destination()).unwrap();
    assert_eq!(peer.app_data, b"authenticated app data");
    assert_eq!(peer.ratchet, Some([0x42; RATCHET_LEN]));
    assert_eq!(receiver.route_count(), 1);
}

#[test]
fn destination_generation_is_explicit_and_includes_the_wire_budget() {
    let mut n = node();
    n.set_network_stamp_policy(policy());
    let emission = blob([0x31; RAND_HASH_LEN]);
    assert_eq!(
        n.try_announce(&emission, None),
        Err(NodeAnnounceError::StampUnavailable)
    );
    assert!(sent(&n.poll(0, IFACE, Some(&emission))).is_none());
    assert!(n.announce_due(1));
    provision(&mut n);
    n.set_logical_mtu(MIN_LOGICAL_MTU + network_stamp::ANNOUNCE_EXT_LEN as u32)
        .unwrap();
    let packet = n.try_announce(&emission, None).unwrap();
    assert_eq!(packet.encoded_len(), n.logical_mtu() as usize);
    n.try_set_app_data(b"x").unwrap();
    assert_eq!(
        n.try_announce(&emission, None),
        Err(NodeAnnounceError::AppDataTooLarge)
    );
}

#[test]
fn a_bad_path_stamp_cannot_consume_a_legitimate_request_tag() {
    let mut n = node();
    provision(&mut n);
    n.poll(0, IFACE, Some(&blob([0x31; RAND_HASH_LEN])));
    let tag = [0x21; crate::path::TAG_LEN];
    let plain = crate::path::path_request(n.destination(), &tag);
    assert!(n.ingest(IFACE, &plain, 1).is_empty());
    let stamp = network_stamp::generate_detached(
        &network_stamp::path_material(n.destination(), &tag),
        policy().path_value,
        10_000,
        random(),
        || false,
    )
    .unwrap();
    let stamped = crate::path::stamped_path_request(
        n.destination(),
        AddressHash::from_bytes([0x33; 16]),
        &tag,
        &stamp,
    );
    assert!(sent(&n.ingest(IFACE, &stamped, 2)).is_some());
    assert!(n.ingest(IFACE, &stamped, 3).is_empty());
}

#[test]
fn stamped_link_is_generated_and_validated_across_an_enforcing_transport() {
    let (mut source, mut destination) = pair();
    provision(&mut source);
    provision(&mut destination);
    let mut relay = node().with_transport_config(TransportConfig::transit());
    provision(&mut relay);
    let announce = destination
        .try_announce(&blob([0x77; RAND_HASH_LEN]), None)
        .unwrap();
    let announce = relayed(&mut relay, IFACE, &announce, 0).unwrap();
    source.ingest(IFACE, &announce, 1);
    let request = sent(
        &source
            .open_link(destination.destination(), IFACE, &[0x99; 64], 1)
            .unwrap(),
    )
    .unwrap();
    assert!(crate::link::request_stamp_valid(
        &request,
        policy().link_cost
    ));

    let mut invalid = request.clone();
    let mut byte = 0_u8;
    while crate::link::request_stamp_valid(&invalid, policy().link_cost) {
        byte = byte.checked_add(1).unwrap();
        invalid.payload[0] = byte;
    }
    assert!(relay.ingest(IFACE, &invalid, 2).is_empty());
    assert!(relay.bridges.is_empty());
    let forwarded = sent(&relay.ingest(IFACE, &request, 3)).unwrap();
    let proof = sent(&destination.ingest(IFACE, &forwarded, 4)).unwrap();
    let proof = sent(&relay.ingest(IFACE, &proof, 5)).unwrap();
    assert!(link_up(&source.ingest(IFACE, &proof, 6)).is_some());
}

#[test]
fn exhausted_link_generation_returns_without_a_pending_handshake() {
    let (mut source, mut destination) = pair();
    provision(&mut destination);
    let announce = destination
        .try_announce(&blob([0x77; RAND_HASH_LEN]), None)
        .unwrap();
    source.set_network_stamp_policy(policy());
    source.ingest(IFACE, &announce, 0);
    source.set_network_stamp_policy(NetworkStampPolicy {
        max_attempts: 0,
        ..policy()
    });
    assert!(
        source
            .open_link(destination.destination(), IFACE, &[0x99; 64], 1)
            .is_none()
    );
    assert!(source.pending.is_empty());
}

#[test]
fn the_shell_can_cancel_link_work_before_a_pending_handshake_is_created() {
    let (mut source, mut destination) = pair();
    assert_eq!(
        source.network_stamp_policy().max_attempts,
        DEFAULT_LINK_STAMP_ATTEMPTS
    );
    provision(&mut source);
    provision(&mut destination);
    let packet = destination
        .try_announce(&blob([0x77; RAND_HASH_LEN]), None)
        .unwrap();
    source.ingest(IFACE, &packet, 0);
    assert!(
        source
            .open_link_cancellable(destination.destination(), IFACE, &[0x99; 64], 1, || true)
            .is_none()
    );
    assert!(source.pending.is_empty());
}

#[test]
fn permissive_admission_preserves_insufficient_trailer_as_application_data() {
    let (mut source, mut receiver) = pair();
    provision(&mut source);
    source.try_set_app_data(b"app").unwrap();
    // Deliberately higher than the supplied stamp, yet accept legacy observations.
    receiver.set_network_stamp_policy(NetworkStampPolicy {
        enforce: false,
        destination_cost: StampCost::MAX,
        ..policy()
    });
    let packet = source
        .try_announce(&blob([0x77; RAND_HASH_LEN]), None)
        .unwrap();
    receiver.ingest(IFACE, &packet, 0);
    let decoded = Announce::decode(&packet).unwrap();
    let mut expected = b"app".to_vec();
    expected.extend_from_slice(&decoded.network_stamp.unwrap().encode());
    assert_eq!(
        receiver
            .peers()
            .resolve(source.destination())
            .unwrap()
            .app_data,
        expected
    );
    assert_eq!(receiver.routes[0].link_cost, None);
}
