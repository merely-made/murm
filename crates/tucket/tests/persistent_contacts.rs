use std::collections::BTreeMap;
use tucket::{
    identity::LocalIdentity,
    node::{ContactRestoreError, DirectRoute, Event, Node, NodeCapacity, validate_contact_keys},
};

fn identity(n: u32) -> [u8; 32] {
    let mut seed = [0x51; 32];
    seed[..4].copy_from_slice(&n.to_le_bytes());
    LocalIdentity::from_seed(seed).identity().pub_key
}

fn distinct(count: usize) -> Vec<[u8; 32]> {
    let mut result = BTreeMap::new();
    for n in 0..4096 {
        let key = identity(n);
        result.entry(key[0]).or_insert(key);
        if result.len() == count {
            return result.into_values().collect();
        }
    }
    panic!("fixture could not find distinct wire addresses")
}

fn node(capacity: usize) -> Node {
    Node::with_capacity(
        LocalIdentity::from_seed([3; 32]),
        false,
        NodeCapacity::new(capacity, 32).unwrap(),
    )
    .unwrap()
}

#[test]
fn cold_restore_can_send_and_match_an_ack_without_another_peer_advert() {
    let mut before = node(2);
    let mut peer = Node::new(LocalIdentity::from_seed([4; 32]), false);
    before.on_frame(&peer.advert_frame(1, b"peer"));
    peer.on_frame(&before.advert_frame(1, b"local"));
    let keys: Vec<_> = before.contact_keys().collect();
    drop(before);
    let mut restored = node(2);
    restored.restore_contacts(&keys).unwrap();
    assert!(restored.route_to(peer.my_hash()).is_none());
    let (text, expected_ack) = restored
        .text_frame(peer.my_hash(), 2, "after restart")
        .unwrap();
    let (received, replies) = peer.on_frame(&text);
    assert!(received.iter().any(|event| matches!(event,
        Event::Message { message, .. } if message.text == "after restart")));
    let mut acknowledged = false;
    for reply in replies {
        let (events, _) = restored.on_frame(&reply);
        acknowledged |= events
            .iter()
            .any(|event| matches!(event, Event::Ack(ack) if *ack == expected_ack));
    }
    assert!(acknowledged);
}

#[test]
fn refused_capacity_and_bad_key_leave_contacts_and_routes_unchanged() {
    let keys = distinct(3);
    let mut node = node(2);
    node.restore_contacts(&keys[..1]).unwrap();
    let route = DirectRoute::new(0, &[]).unwrap();
    node.set_route(keys[0][0], route.clone());
    assert_eq!(
        node.restore_contacts(&keys[1..]),
        Err(ContactRestoreError::ContactsFull)
    );
    let mut weak = [0; 32];
    weak[0] = 1;
    assert_eq!(
        node.restore_contacts(&[keys[1], weak]),
        Err(ContactRestoreError::InvalidPublicKey)
    );
    assert_eq!(node.contact_keys().collect::<Vec<_>>(), keys[..1]);
    assert_eq!(node.route_to(keys[0][0]), Some(&route));
}

#[test]
fn collision_in_cache_or_against_live_state_never_partially_imports() {
    let mut found = BTreeMap::new();
    let (a, b) = (0..4096)
        .find_map(|n| {
            let key = identity(n);
            found.insert(key[0], key).map(|old| (old, key))
        })
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(
        validate_contact_keys(&[a, b]),
        Err(ContactRestoreError::HashCollision)
    );
    let other = distinct(3).into_iter().find(|k| k[0] != a[0]).unwrap();
    let mut node = node(3);
    node.restore_contacts(&[a]).unwrap();
    assert_eq!(
        node.restore_contacts(&[other, b]),
        Err(ContactRestoreError::HashCollision)
    );
    assert_eq!(node.contact_keys().collect::<Vec<_>>(), vec![a]);
}

#[test]
fn cache_shape_is_bounded_and_repeat_import_preserves_known_routes() {
    let key = identity(1);
    assert_eq!(
        validate_contact_keys(&[key, key]),
        Err(ContactRestoreError::DuplicateKey)
    );
    assert_eq!(
        validate_contact_keys(&[key; 257]),
        Err(ContactRestoreError::ContactsFull)
    );
    let mut node = node(1);
    node.restore_contacts(&[key]).unwrap();
    let route = DirectRoute::new(0, &[]).unwrap();
    node.set_route(key[0], route.clone());
    node.restore_contacts(&[key]).unwrap();
    assert_eq!(node.contact_keys().count(), 1);
    assert_eq!(node.route_to(key[0]), Some(&route));
}
