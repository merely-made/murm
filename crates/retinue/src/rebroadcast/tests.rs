//! The announce table and the announce cap.

use super::*;
use crate::packet::{DestinationType, HeaderType, PacketType, Propagation};

fn rebroadcast(seed: u8, hops: u8, emitted: u64) -> Rebroadcast {
    let destination = AddressHash::from_bytes([seed; 16]);
    Rebroadcast {
        destination,
        packet: Packet {
            ifac: false,
            header_type: HeaderType::Type2,
            context_flag: false,
            propagation: Propagation::Broadcast,
            destination_type: DestinationType::Single,
            packet_type: PacketType::Announce,
            hops,
            transport: Some(AddressHash::from_bytes([0xEE; 16])),
            destination,
            context: 0,
            payload: alloc::vec![seed; 132],
        },
        interface: 0,
        emitted,
        answer: None,
    }
}

#[test]
fn sends_once_then_retries_once_after_the_grace() {
    let mut table = Rebroadcasts::new(4);
    assert!(table.schedule(rebroadcast(1, 1, 0), 100, 300));
    assert!(table.pop_due(399).is_none());
    assert!(table.pop_due(400).is_some());
    let retry = 400 + REBROADCAST_GRACE + REBROADCAST_WINDOW;
    assert_eq!(table.next_due(), Some(retry));
    assert!(table.pop_due(retry - 1).is_none());
    assert!(table.pop_due(retry).is_some());
    assert_eq!(table.len(), 0, "two transmissions, then done");
}

#[test]
fn neighbour_rebroadcasts_end_ours_only_after_our_first_send() {
    let mut table = Rebroadcasts::new(4);
    table.schedule(rebroadcast(1, 2, 0), 0, 100);
    assert!(!table.heard(AddressHash::from_bytes([1; 16]), 2, 50));
    assert!(!table.heard(AddressHash::from_bytes([1; 16]), 2, 60));
    assert!(table.pop_due(100).is_some(), "RNS still sends once");
    assert!(table.heard(AddressHash::from_bytes([1; 16]), 2, 200));
    assert_eq!(table.len(), 0);

    table.schedule(rebroadcast(2, 2, 0), 0, 0);
    table.pop_due(0);
    assert!(!table.heard(AddressHash::from_bytes([2; 16]), 4, 10));
    assert!(
        table.heard(AddressHash::from_bytes([2; 16]), 3, 10),
        "passed on with one more hop"
    );
}

#[test]
fn an_unverified_copy_is_heard_only_if_it_matches_the_held_announce() {
    let mut table = Rebroadcasts::new(4);
    let held = rebroadcast(1, 2, 0);
    table.schedule(held.clone(), 0, 0);
    table.pop_due(0);
    let mut other = held.packet.clone();
    other.payload[0] ^= 1;
    assert!(!table.heard_copy(&other, 10), "different signed fields");
    let mut copy = held.packet.clone();
    copy.transport = Some(AddressHash::from_bytes([0xDD; 16]));
    copy.hops = 3;
    assert!(table.heard_copy(&copy, 10), "passed on with one more hop");
    assert_eq!(table.len(), 0);
}

#[test]
fn a_full_table_makes_room_only_from_sent_entries() {
    let mut table = Rebroadcasts::new(1);
    assert!(table.schedule(rebroadcast(1, 1, 0), 0, 10));
    assert!(!table.schedule(rebroadcast(2, 1, 0), 0, 10));
    table.pop_due(10);
    assert!(table.schedule(rebroadcast(2, 1, 0), 10, 10));
}

#[test]
fn a_replacement_takes_over_the_remaining_transmission() {
    let mut table = Rebroadcasts::new(1);
    assert!(table.schedule(rebroadcast(1, 1, 0), 0, 0));
    table.pop_due(0);
    assert!(
        table.schedule(rebroadcast(1, 2, 1), 10, 10),
        "a newer announce"
    );
    assert!(
        table.schedule(rebroadcast(2, 1, 0), 10, 10),
        "the replaced entry stays evictable"
    );
    assert!(!table.contains(AddressHash::from_bytes([1; 16])));

    let mut table = Rebroadcasts::new(4);
    table.schedule(rebroadcast(3, 1, 0), 0, 0);
    table.pop_due(0);
    table.schedule(rebroadcast(3, 1, 1), 10, 10);
    assert_eq!(table.pop_due(20).unwrap().emitted, 1);
    assert_eq!(table.len(), 0, "sent once more, then done");
}

#[test]
fn a_path_response_goes_once_and_restores_the_relay_it_displaced() {
    let mut table = Rebroadcasts::new(2);
    assert!(table.schedule(rebroadcast(1, 1, 0), 0, 500));
    let answer = Rebroadcast {
        answer: Some(7),
        ..rebroadcast(1, 2, 0)
    };
    assert!(table.answer(answer, 0, 400));
    assert_eq!(table.pop_due(400).unwrap().answer, Some(7));
    let relay = table.pop_due(500).expect("the held relay is back");
    assert_eq!((relay.answer, relay.packet.hops), (None, 1));

    let mut table = Rebroadcasts::new(2);
    table.answer(
        Rebroadcast {
            answer: Some(7),
            ..rebroadcast(2, 2, 0)
        },
        0,
        400,
    );
    assert!(
        table.heard(AddressHash::from_bytes([2; 16]), 3, 10),
        "another node passed it on first"
    );
    assert_eq!(table.len(), 0);
}

#[test]
fn concurrent_path_responses_each_reach_their_interface() {
    let answer = |iface| Rebroadcast {
        answer: Some(iface),
        ..rebroadcast(1, 2, 0)
    };
    let mut table = Rebroadcasts::new(4);
    assert!(table.schedule(rebroadcast(1, 1, 0), 0, 900));
    assert!(table.answer(answer(7), 0, 400));
    assert!(table.answer(answer(8), 100, 400));
    assert!(table.answer(answer(8), 200, 400), "a repeat supersedes");
    assert!(
        table.schedule(rebroadcast(1, 1, 1), 300, 900),
        "held behind"
    );
    let sent: Vec<_> = core::iter::from_fn(|| table.pop_due(5_000))
        .map(|rebroadcast| rebroadcast.answer)
        .collect();
    assert_eq!(sent[..3], [Some(8), Some(7), None]);
    assert_eq!(table.len(), 1, "the relay keeps its retry");
}

#[test]
fn forgetting_an_interface_drops_what_it_carried() {
    let on = |seed, iface| Rebroadcast {
        interface: iface,
        ..rebroadcast(seed, 1, 0)
    };
    let mut table = Rebroadcasts::new(4);
    assert!(table.schedule(on(1, 3), 0, 500));
    assert!(table.schedule(on(2, 4), 0, 500));
    let answer = Rebroadcast {
        answer: Some(3),
        ..on(2, 4)
    };
    assert!(table.answer(answer, 0, 400), "holds the relay heard on 4");
    table.forget_interface(3);
    assert!(!table.contains(AddressHash::from_bytes([1; 16])));
    let relay = table
        .pop_due(500)
        .expect("the relay heard on 4 is restored");
    assert_eq!((relay.interface, relay.answer), (4, None));
}

#[test]
fn a_recursive_path_request_spends_the_announce_budget() {
    let rate = CapRate::from_bitrate(1_200, 2);
    let mut cap = AnnounceCap::new(4);
    assert!(cap.admit_path_request(67, 0, rate));
    assert!(!cap.admit_path_request(67, cap_wait(67, rate) - 1, rate));
    assert!(cap.admit_path_request(67, cap_wait(67, rate), rate));
    let mut busy = AnnounceCap::new(4);
    busy.offer(rebroadcast(1, 1, 0), 0, rate);
    busy.offer(rebroadcast(2, 1, 0), 0, rate);
    assert!(
        !busy.admit_path_request(67, u64::MAX, rate),
        "announces queue"
    );
}

#[test]
fn an_extreme_airtime_saturates() {
    assert_eq!(cap_wait(500, CapRate::new(u64::MAX)), u64::MAX);
    let mut cap = AnnounceCap::new(1);
    assert!(matches!(
        cap.offer(rebroadcast(1, 1, 0), 5, CapRate::new(u64::MAX)),
        Offer::Send(_)
    ));
    assert!(matches!(
        cap.offer(rebroadcast(2, 1, 0), 6, CapRate::new(u64::MAX)),
        Offer::Queued
    ));
    assert!(cap.pop_due(u64::MAX - 1, CapRate::new(u64::MAX)).is_none());
}

#[test]
fn the_share_scales_the_wait() {
    // 1200 bps at 5 %: a 300-byte announce takes 2 s, so 40 s.
    let rate = CapRate::from_bitrate(1_200, 5);
    assert_eq!(cap_wait(300, rate), 40_000);
    assert_eq!(cap_wait(300, CapRate::from_bitrate(1_200, 2)), 100_000);
    assert_eq!(
        cap_wait(300, CapRate::from_bitrate(1_200, 0)),
        cap_wait(300, CapRate::from_bitrate(1_200, 1))
    );
}

#[test]
fn a_fast_bitrate_paces_finer_than_its_mtu_airtime() {
    // 100 Mbps at 2 %: a 167-byte announce takes 13.36 us, so 0.668 ms, one tick; a
    // whole-millisecond MTU airtime would make it 17 ms.
    assert_eq!(cap_wait(167, CapRate::from_bitrate(100_000_000, 2)), 1);
    assert_eq!(cap_wait(167, CapRate::new(1)), 17);
}

#[test]
fn the_cap_queues_the_newest_emission_and_releases_fewest_hops_first() {
    // 64 ms per MTU (62.5 kbps): a 167-byte announce takes 21.4 ms, so 1069 ms at 2 %.
    const RATE: CapRate = CapRate::new(64);
    let mut cap = AnnounceCap::new(4);
    let first = rebroadcast(1, 3, 10);
    let wait = cap_wait(first.packet.encoded_len(), RATE);
    assert_eq!(wait, 1069);
    assert!(matches!(cap.offer(first, 0, RATE), Offer::Send(_)));
    assert!(matches!(
        cap.offer(rebroadcast(2, 3, 10), 1, RATE),
        Offer::Queued
    ));
    assert!(matches!(
        cap.offer(rebroadcast(3, 1, 10), 2, RATE),
        Offer::Queued
    ));
    assert!(matches!(
        cap.offer(rebroadcast(2, 5, 11), 3, RATE),
        Offer::Queued
    ));
    assert!(matches!(
        cap.offer(rebroadcast(2, 4, 9), 4, RATE),
        Offer::Queued
    ));
    assert_eq!(cap.queued(), 2);
    assert_eq!(cap.next_due(), Some(wait));
    assert!(cap.pop_due(wait - 1, RATE).is_none());
    assert_eq!(cap.pop_due(wait, RATE).unwrap().hops, 1);
    assert!(cap.pop_due(wait + 1, RATE).is_none());
    let newest = cap.pop_due(2 * wait, RATE).unwrap();
    assert_eq!(newest.hops, 5, "the newer emission replaced the queued one");
    assert_eq!(cap.next_due(), None);
}
