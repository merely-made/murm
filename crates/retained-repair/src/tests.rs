// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::*;
use ed25519_dalek::SigningKey;

fn owners() -> (Producer, Collector, Profile, SigningKey) {
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let profile = Profile::new([7; 32], key.verifying_key(), 4, 7, 1).unwrap();
    (
        Producer::new(profile.clone(), key.clone()).unwrap(),
        Collector::new(profile.clone()).unwrap(),
        profile,
        key,
    )
}
fn object(n: usize, salt: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(salt))
        .collect()
}

#[test]
fn settlement_preserves_deferred_frame_and_advances_only_accepted_cursor() {
    let (mut p, _, _, _) = owners();
    assert_eq!(
        p.publish(0, 1, b"some exact bytes", 30),
        Err(Error::AdvanceRequired)
    );
    p.advance(0).unwrap();
    p.publish(0, 1, b"some exact bytes", 30).unwrap();
    let m = p.prepare(0, 0, Some(1)).unwrap().unwrap();
    assert_eq!(p.stats().queued_frames, 0);
    assert_eq!(
        p.settle(0, m.token, Decision::Defer).unwrap(),
        Settlement::Deferred
    );
    assert_eq!(p.prepare(0, 0, Some(1)).unwrap().unwrap(), m);
    assert_eq!(p.prepare(0, 1, None), Err(Error::PendingFrame));
    assert_eq!(
        p.settle(0, FrameToken(999), Decision::Accept),
        Err(Error::StaleToken)
    );
    p.settle(0, m.token, Decision::Accept).unwrap();
    assert_eq!(
        p.settle(0, m.token, Decision::Accept),
        Err(Error::StaleToken)
    );
    p.advance(1).unwrap();
    let s = p.prepare(1, 2, Some(1)).unwrap().unwrap();
    assert_eq!(s.lane, Lane::Fresh);
    assert_eq!(s.esi, Some(0));
    p.settle(1, s.token, Decision::Defer).unwrap();
    p.advance(2).unwrap();
    assert_eq!(p.prepare(2, 2, Some(1)).unwrap().unwrap(), s);
    assert_eq!(p.stats().queued_fresh, 0);
    p.settle(2, s.token, Decision::Accept).unwrap();
    assert_eq!(p.stats().queued_fresh, 1);
    assert!(p.prepare(2, 2, Some(1)).unwrap().is_none());
    p.advance(3).unwrap();
    assert_eq!(
        p.prepare(3, 2, Some(1)).unwrap().unwrap().lane,
        Lane::Repair
    );
}

#[test]
fn capacity_duplicate_conflict_and_expiry_are_visible_before_encoder_construction() {
    let (mut p, _, _, _) = owners();
    p.advance(0).unwrap();
    let original = object(MAX_OBJECT_OCTETS, 1);
    for id in 1..=4 {
        p.publish(0, id, &original, if id == 1 { 7 } else { 20 })
            .unwrap();
    }
    assert_eq!(p.retained_original_octets(), MAX_RETAINED_ORIGINAL_OCTETS);
    assert_eq!(
        p.publish(0, 5, &original, 20),
        Err(Error::Full {
            next_expiry: Some(7)
        })
    );
    assert_eq!(p.stats().encoder_allocations, 4);
    assert!(matches!(
        p.publish(0, 1, &original, 7).unwrap(),
        PublishOutcome::AlreadyRetained(_)
    ));
    assert_eq!(p.publish(0, 1, &original, 8), Err(Error::ConflictingObject));
    let mut different = original.clone();
    different[0] ^= 1;
    assert_eq!(
        p.publish(0, 1, &different, 7),
        Err(Error::ConflictingObject)
    );
    assert_eq!(
        p.publish(0, 5, &vec![0; MAX_OBJECT_OCTETS + 1], 20),
        Err(Error::ObjectSize)
    );
    let pending = p.prepare(0, 0, Some(1)).unwrap().unwrap();
    let expiry = p.advance(7).unwrap();
    assert_eq!(expiry.objects.len(), 1);
    assert_eq!(expiry.invalidated_token, Some(pending.token));
    assert_eq!(
        p.settle(7, pending.token, Decision::Accept),
        Err(Error::StaleToken)
    );
    assert_eq!(p.retained_original_octets(), 3 * MAX_OBJECT_OCTETS);
    assert_eq!(p.publish(7, 1, &original, 20), Err(Error::GenerationReused));
    p.publish(7, 5, &original, 20).unwrap();
    assert_eq!(p.stats().encoder_allocations, 5);
    assert_eq!(
        p.lookup(5).unwrap().digest,
        *blake3::hash(&original).as_bytes()
    );
    assert_eq!(p.advance(6), Err(Error::ClockRegression));
    assert_eq!(p.advance(MAX_EPOCH_TICK + 1), Err(Error::EpochExhausted));
}

#[test]
fn authentication_profile_capacity_and_manifest_conflicts_precede_decoder_work() {
    let (mut p, mut c, profile, key) = owners();
    p.advance(0).unwrap();
    c.advance(0).unwrap();
    let original = object(4097, 3);
    p.publish(0, 1, &original, 50).unwrap();
    let m = p.prepare(0, 0, Some(1)).unwrap().unwrap();
    let mut bad = m.bytes.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(c.ingest(0, &bad), Err(Error::InvalidSignature));
    assert!(c.inventory().is_empty());
    assert_eq!(c.stats().decoder_allocations, 0);
    let wrong = Profile::new(
        profile.namespace,
        SigningKey::from_bytes(&[3; 32]).verifying_key(),
        4,
        7,
        1,
    )
    .unwrap();
    let mut wrong = Collector::new(wrong).unwrap();
    wrong.advance(0).unwrap();
    assert_eq!(wrong.ingest(0, &m.bytes), Err(Error::InvalidSignature));
    let wire::AuthenticatedFrame::Manifest(mut manifest) =
        wire::verify(&m.bytes, &profile.namespace, &profile.authority).unwrap()
    else {
        panic!()
    };
    manifest.metadata.digest[0] ^= 1;
    let conflict = wire::encode_manifest(&manifest, &profile.namespace, &key).unwrap();
    c.ingest(0, &m.bytes).unwrap();
    assert_eq!(c.ingest(0, &conflict), Err(Error::ConflictingObject));
    manifest.metadata.generation = 2;
    manifest.oti.transfer_length = 32_000;
    manifest.metadata.object_octets = 32_000;
    let too_large = wire::encode_manifest(&manifest, &profile.namespace, &key).unwrap();
    assert_eq!(c.ingest(0, &too_large), Err(Error::ProfileMismatch));
    manifest.oti.transfer_length = 4097;
    manifest.metadata.object_octets = 4097;
    for id in 2..=4 {
        manifest.metadata.generation = id;
        c.ingest(
            0,
            &wire::encode_manifest(&manifest, &profile.namespace, &key).unwrap(),
        )
        .unwrap();
    }
    manifest.metadata.generation = 5;
    assert_eq!(
        c.ingest(
            0,
            &wire::encode_manifest(&manifest, &profile.namespace, &key).unwrap()
        ),
        Err(Error::Full {
            next_expiry: Some(50)
        })
    );
    assert_eq!(c.stats().decode_work, 0);
    assert_eq!(c.active_decoders(), 0);
    c.advance(50).unwrap();
    assert!(c.inventory().is_empty());
    assert_eq!(c.ingest(50, &m.bytes), Err(Error::ExpiredFrame));
}

#[test]
fn new_b_pipelines_while_late_a_recovers_from_actual_repair_without_foreknowledge() {
    let (mut p, mut fast, profile, _) = owners();
    let mut late = Collector::new(profile).unwrap();
    let a = object(4097, 4);
    let b = object(1281, 9);
    let mut late_source_symbols = 0;
    let mut b_published_while_a_retained = false;
    for now in 0..320 {
        p.advance(now).unwrap();
        fast.advance(now).unwrap();
        late.advance(now).unwrap();
        if now == 0 {
            p.publish(now, 1, &a, 400).unwrap();
        }
        if now == 8 {
            p.publish(now, 2, &b, 400).unwrap();
            b_published_while_a_retained = p.lookup(1).is_some();
        }
        for channel in 0..4 {
            if let Some(frame) = p.prepare(now, channel, None).unwrap() {
                p.settle(now, frame.token, Decision::Accept).unwrap();
                fast.ingest(now, &frame.bytes).unwrap();
                let listen = if now.is_multiple_of(7) {
                    0
                } else {
                    (now % 4) as u8
                };
                if now >= 20 && channel == listen {
                    if frame.generation == 1 && frame.lane == Lane::Fresh {
                        late_source_symbols += 1;
                    }
                    let outcome = late.ingest(now, &frame.bytes);
                    assert!(outcome.is_ok() || outcome == Err(Error::UnknownGeneration));
                }
            }
        }
        if late.recovered(1).is_some() {
            break;
        }
    }
    assert!(b_published_while_a_retained);
    assert_eq!(
        late_source_symbols, 0,
        "late consumer must not see A's fresh symbols"
    );
    assert_eq!(fast.recovered(1).unwrap().bytes, a);
    assert_eq!(fast.recovered(2).unwrap().bytes, b);
    assert_eq!(late.recovered(1).unwrap().bytes, a);
    assert_eq!(
        late.recovered(1).unwrap().metadata.digest,
        *blake3::hash(&a).as_bytes()
    );
    assert_eq!(
        late.active_decoders(),
        late.inventory().len() - late.stats().completed_objects as usize
    );
    assert!(p.stats().queued_repairs > 0);
    let expired = late.advance(400).unwrap();
    assert!(!expired.objects.is_empty());
    assert_eq!(late.recovered_octets(), 0);
    assert_eq!(late.active_decoders(), 0);
}

#[test]
fn requested_inventory_selection_repeats_manifest_for_late_repair_only_collection() {
    let (mut p, mut c, _, _) = owners();
    let original = object(1015, 11);
    let mut done = false;
    for now in 0..80 {
        p.advance(now).unwrap();
        c.advance(now).unwrap();
        if now == 0 {
            p.publish(now, 1, &original, 100).unwrap();
        }
        let frame = p.prepare(now, 0, Some(1)).unwrap().unwrap();
        p.settle(now, frame.token, Decision::Accept).unwrap();
        if now >= 14 && frame.lane != Lane::Fresh {
            c.ingest(now, &frame.bytes).unwrap();
            if let Some(recovered) = c.recovered(1) {
                assert_eq!(recovered.bytes, original);
                done = true;
                break;
            }
        }
    }
    assert!(done);
    assert_eq!(c.active_decoders(), 0);
}
