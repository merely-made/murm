//! Explicit encoded and decoded Resource receive budgets.
use super::*;
use crate::Error;
use crate::link::{CTX_RESOURCE_ADV, CTX_RESOURCE_RCL};
use crate::resource::{Advertisement, Outgoing, content};

fn receiver(link: &Link, encoded: usize, decoded: usize) -> SegmentedReceiver {
    let make = link.clone();
    SegmentedReceiver::new(link.clone(), move || ResourceReceiver::new(make.clone()))
        .with_receive_limits(ResourceReceiveLimits {
            max_encoded_bytes: encoded,
            max_decoded_bytes: decoded,
        })
}

fn drive(sender: &mut ResourceSender, receiver: &mut SegmentedReceiver) {
    let mut iv = iv_gen();
    let packet = sender.advertisement(&iv());
    drive_packets(sender, receiver, vec![packet]);
}

fn drive_packets(
    sender: &mut ResourceSender,
    receiver: &mut SegmentedReceiver,
    mut incoming: Vec<Packet>,
) {
    let mut iv = iv_gen();
    while !incoming.is_empty() {
        let outgoing = deliver(core::mem::take(&mut incoming), |p| {
            receiver.on_packet(p, 0, &mut iv)
        });
        incoming = deliver(outgoing, |p| sender.on_packet(p, 0, &mut iv));
    }
}

#[test]
fn understated_encoded_size_is_refused_before_any_part_request() {
    let (send, recv) = link_pair();
    let mut iv = iv_gen();
    let sender = ResourceSender::publish(send.clone(), b"small", [3; 4], &iv());
    let mut adv =
        Advertisement::parse(&recv.decrypt(&sender.advertisement(&iv())).unwrap()).unwrap();
    adv.data_size = 1;
    adv.transfer_size = 1_000_000;
    adv.parts = 4096;
    let packet = send.sealed_packet(CTX_RESOURCE_ADV, &adv.pack(), &iv());
    let mut bounded = receiver(&recv, 64, 64);
    let replies = bounded.on_packet(&packet, 0, &mut iv);
    assert_eq!(bounded.failure(), Some(Error::CapacityExceeded));
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].context, CTX_RESOURCE_RCL);
    assert_eq!(bounded.segments_proved(), 0);
    assert!(bounded.data().is_none());
}

#[test]
fn exact_encoded_fit_counts_one_segment_once_across_reoffers() {
    let (send, recv) = link_pair();
    let mut iv = iv_gen();
    let data = sha256_counter_payload(4000);
    let mut sender = ResourceSender::publish(send, &data, [3; 4], &iv());
    let packet = sender.advertisement(&iv());
    let adv = Advertisement::parse(&recv.decrypt(&packet).unwrap()).unwrap();
    let mut bounded = receiver(&recv, adv.transfer_size as usize, data.len());
    let requests = bounded.on_packet(&packet, 0, &mut iv);
    assert!(bounded.on_packet(&packet, 0, &mut iv).is_empty());
    let parts = deliver(requests, |p| sender.on_packet(p, 0, &mut iv));
    drive_packets(&mut sender, &mut bounded, parts);
    assert_eq!(bounded.data(), Some(data.as_slice()));
    assert!(sender.is_done());
}

#[test]
fn uncompressed_understatement_fails_before_recovered_data_is_proved() {
    let (send, recv) = link_pair();
    let random = [3; 4];
    let data = sha256_counter_payload(4000);
    let token = send.seal(&content(&data, &random), &[4; 16]);
    let out = Outgoing::new(&data, &token, random, false).with_segment(1, 1, 100, [0; 32]);
    let mut sender = ResourceSender::from_outgoing(send, out);
    let mut bounded = receiver(&recv, token.len(), 100);
    drive(&mut sender, &mut bounded);
    assert_eq!(bounded.failure(), Some(Error::ResourceCorrupt));
    assert_eq!(bounded.segments_proved(), 0);
    assert!(sender.is_canceled());
}

#[test]
fn decoded_offer_refusal_and_encoded_refusal_are_independent() {
    let (send, recv) = link_pair();
    let mut iv = iv_gen();
    let data = vec![0; 4000];
    let sender = ResourceSender::publish(send, &data, [3; 4], &iv());
    let packet = sender.advertisement(&iv());
    for (encoded, decoded) in [(64_000, 3999), (1, 4000)] {
        let mut bounded = receiver(&recv, encoded, decoded);
        let replies = bounded.on_packet(&packet, 0, &mut iv);
        assert_eq!(bounded.failure(), Some(Error::CapacityExceeded));
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].context, CTX_RESOURCE_RCL);
    }
}

#[test]
fn encoded_budget_is_shared_across_segments() {
    let (send, recv) = link_pair();
    let mut iv = iv_gen();
    let data = sha256_counter_payload(crate::resource::MAX_SEGMENT_SIZE + 32);
    let mut sender = SegmentedSender::new(
        send,
        data.as_slice(),
        None,
        ResourceKind::Data,
        [3; 4],
        &iv(),
    )
    .unwrap();
    let packet = sender.advertisement(&iv());
    let first = Advertisement::parse(&recv.decrypt(&packet).unwrap()).unwrap();
    let mut bounded = receiver(&recv, first.transfer_size as usize, data.len());
    let mut incoming = vec![packet];
    while !incoming.is_empty() {
        let outgoing = deliver(core::mem::take(&mut incoming), |p| {
            bounded.on_packet(p, 0, &mut iv)
        });
        incoming = deliver(outgoing, |p| sender.on_packet(p, 0, &mut iv));
    }
    assert_eq!(bounded.segments_proved(), 1);
    assert_eq!(bounded.failure(), Some(Error::CapacityExceeded));
    assert!(bounded.data().is_none());
    assert!(sender.is_canceled());
}

#[test]
fn multipart_last_segment_obeys_remaining_decoded_budget() {
    let (send, recv) = link_pair();
    let mut iv = iv_gen();
    let random = [3; 4];
    let data = vec![7; crate::resource::MAX_SEGMENT_SIZE];
    let token = send.seal(&content(&data, &random), &iv());
    let first = Outgoing::new(&data, &token, random, false);
    let hash = first.advertisement().resource_hash;
    let mut original = [0; 32];
    original.copy_from_slice(&hash);
    let total = (data.len() + 1) as u64;
    let first = first.with_segment(1, 2, total, original);
    let mut sender = ResourceSender::from_outgoing(send.clone(), first);
    let mut bounded = receiver(&recv, 2 * token.len(), total as usize);
    drive(&mut sender, &mut bounded);
    assert!(sender.is_done());
    assert_eq!(bounded.segments_proved(), 1);
    let extra = [1, 2]; // The advertised remaining budget is only one byte.
    let token = send.seal(&content(&extra, &random), &iv());
    let second = Outgoing::new(&extra, &token, random, false).with_segment(2, 2, total, original);
    let mut sender = ResourceSender::from_outgoing(send, second);
    drive(&mut sender, &mut bounded);
    assert_eq!(
        bounded.segments_proved(),
        1,
        "oversized tail was not proved or appended"
    );
    assert_eq!(bounded.failure(), Some(Error::ResourceCorrupt));
    assert!(bounded.data().is_none());
    assert!(sender.is_canceled());
}
