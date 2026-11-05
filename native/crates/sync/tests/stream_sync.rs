// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Two in-process peers catch up over a plain duplex stream.

use std::collections::HashSet;
use std::time::Duration;

use std::collections::BTreeMap;

use muniment::MemoryBackend;
use murm_sync::{StreamSession, StreamSync, StreamSyncError, StreamSyncLimits, processor_accept};
use p2panda_core::cbor::encode_cbor;
use p2panda_core::{Body, Header, Operation, SigningKey, Topic, VerifyingKey};
use p2panda_sync::protocols::{LogSyncMessage, TopicLogSyncMessage};
use stickleback::{
    Admission, MunimentStore, OperationPolicy, OperationProcessor, Reject, StoreTarget,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

type Store = MunimentStore<MemoryBackend, ()>;

#[derive(Clone)]
struct Policy {
    topic: Topic,
    authors: HashSet<VerifyingKey>,
}

impl OperationPolicy<()> for Policy {
    type LogId = u64;

    fn admit(&self, operation: &Operation<()>) -> Result<Admission<u64>, Reject> {
        if !self.authors.contains(&operation.header.verifying_key) {
            return Err(Reject::new("author", "author is not a member"));
        }
        Ok(Admission::keep(StoreTarget::new(self.topic.clone(), 0)))
    }
}

fn signed(key: &SigningKey, seq: u32, backlink: Option<p2panda_core::Hash>) -> Operation<()> {
    let payload = format!("release {seq}");
    let header = Header::<()>::builder()
        .body(payload.as_bytes())
        .seq_num(seq)
        .backlink(backlink)
        .build(key, ());
    Operation {
        hash: header.hash(),
        header,
        body: Some(Body::from_bytes(payload.as_bytes())),
    }
}

/// `count` chained operations by `key`, stored the way a local author stores.
async fn author(store: &Store, topic: &Topic, key: &SigningKey, count: u32) {
    let mut backlink = None;
    for seq in 0..count {
        let op = signed(key, seq, backlink);
        backlink = Some(op.hash);
        store
            .insert_indexed_operation(topic, &op, &0u64)
            .await
            .unwrap();
    }
}

struct Peer {
    store: Store,
    processor: OperationProcessor<MemoryBackend, (), Policy>,
}

fn peer(topic: &Topic, authors: &[&SigningKey]) -> Peer {
    let store = Store::new(MemoryBackend::new());
    let policy = Policy {
        topic: topic.clone(),
        authors: authors.iter().map(|key| key.verifying_key()).collect(),
    };
    Peer {
        processor: OperationProcessor::new(store.clone(), policy),
        store,
    }
}

fn session(topic: &Topic, peer: &Peer, limits: StreamSyncLimits) -> impl StreamSession {
    StreamSync::<_, u64, (), _>::new(
        topic.clone(),
        peer.store.clone(),
        limits,
        processor_accept(peer.processor.clone()),
    )
}

#[tokio::test]
async fn peers_with_different_halves_converge() {
    let topic = Topic::random();
    let (ada, bea) = (SigningKey::generate(), SigningKey::generate());
    let left = peer(&topic, &[&ada, &bea]);
    let right = peer(&topic, &[&ada, &bea]);
    author(&left.store, &topic, &ada, 3).await;
    author(&right.store, &topic, &bea, 2).await;

    let (a, b) = duplex(64 * 1024);
    let limits = StreamSyncLimits::default();
    let (l, r) = tokio::join!(
        session(&topic, &left, limits).run(a),
        session(&topic, &right, limits).run(b),
    );
    let (l, r) = (l.unwrap(), r.unwrap());
    assert_eq!((l.accepted, l.declined), (2, 0));
    assert_eq!((r.accepted, r.declined), (3, 0));
    assert_eq!(left.store.operation_count().await.unwrap(), 5);
    assert_eq!(right.store.operation_count().await.unwrap(), 5);
}

/// One wire frame as a remote would write it: length prefix, then CBOR.
fn frame(message: &TopicLogSyncMessage<u64>) -> Vec<u8> {
    let body = encode_cbor(message).unwrap();
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

fn raw(op: &Operation<()>) -> TopicLogSyncMessage<u64> {
    TopicLogSyncMessage::Sync(LogSyncMessage::Operation((
        op.header.encode(),
        op.body.as_ref().map(|body| body.to_bytes()),
    )))
}

#[tokio::test]
async fn a_forged_operation_ends_the_session_after_policy_ran_on_the_rest() {
    let topic = Topic::random();
    let (ada, mallory, outsider) = (
        SigningKey::generate(),
        SigningKey::generate(),
        SigningKey::generate(),
    );
    let honest = peer(&topic, &[&ada]);

    let genuine = signed(&ada, 0, None);
    // Out of policy: validly signed, but by an author the policy does not admit.
    let unlisted = signed(&outsider, 0, None);
    // Forged: Mallory's operation carrying Ada's key, so the signature fails.
    let mut forged = signed(&mallory, 0, None);
    forged.header.verifying_key = ada.verifying_key();
    forged.hash = forged.header.hash();

    // A hostile remote speaks the protocol by hand; a stickleback store would
    // refuse to serve a forgery, so it cannot be the sender.
    let (a, mut b) = duplex(64 * 1024);
    let ops = [genuine, unlisted, forged];
    let hostile = async move {
        let mut wire = frame(&TopicLogSyncMessage::Sync(LogSyncMessage::Have(
            BTreeMap::new(),
        )));
        wire.extend(frame(&TopicLogSyncMessage::Sync(LogSyncMessage::PreSync {
            total_operations: 3,
            total_bytes: 1,
        })));
        for op in &ops {
            wire.extend(frame(&raw(op)));
        }
        wire.extend(frame(&TopicLogSyncMessage::Sync(LogSyncMessage::Done)));
        b.write_all(&wire).await.unwrap();
        let mut sink = Vec::new();
        let _ = b.read_to_end(&mut sink).await;
    };
    let (outcome, ()) = tokio::join!(
        session(&topic, &honest, StreamSyncLimits::default()).run(a),
        hostile
    );
    // The protocol verifies each header as it decodes it, so the forgery ends
    // the session; what arrived before it went through the policy.
    let error = outcome.unwrap_err();
    assert!(
        matches!(&error, StreamSyncError::Protocol(text) if text.contains("invalid signature")),
        "{error:?}"
    );
    assert_eq!(honest.store.operation_count().await.unwrap(), 1);
}

#[tokio::test]
async fn an_out_of_policy_operation_is_declined_without_ending_the_session() {
    let topic = Topic::random();
    let (ada, outsider) = (SigningKey::generate(), SigningKey::generate());
    let honest = peer(&topic, &[&ada]);
    let ops = [signed(&outsider, 0, None), signed(&ada, 0, None)];
    let (a, mut b) = duplex(64 * 1024);
    let hostile = async move {
        let mut wire = frame(&TopicLogSyncMessage::Sync(LogSyncMessage::Have(
            BTreeMap::new(),
        )));
        wire.extend(frame(&TopicLogSyncMessage::Sync(LogSyncMessage::PreSync {
            total_operations: 2,
            total_bytes: 1,
        })));
        for op in &ops {
            wire.extend(frame(&raw(op)));
        }
        wire.extend(frame(&TopicLogSyncMessage::Sync(LogSyncMessage::Done)));
        b.write_all(&wire).await.unwrap();
        let mut sink = Vec::new();
        let _ = b.read_to_end(&mut sink).await;
    };
    let (outcome, ()) = tokio::join!(
        session(&topic, &honest, StreamSyncLimits::default()).run(a),
        hostile
    );
    let outcome = outcome.unwrap();
    assert_eq!(
        (outcome.received, outcome.accepted, outcome.declined),
        (2, 1, 1)
    );
    assert_eq!(honest.store.operation_count().await.unwrap(), 1);
}

#[tokio::test]
async fn an_oversize_frame_is_refused_before_it_is_buffered() {
    let topic = Topic::random();
    let victim = peer(&topic, &[]);
    let (a, mut b) = duplex(1024);
    let limits = StreamSyncLimits::new(4096, 1 << 20, Duration::from_secs(5)).unwrap();
    let attacker = async move {
        b.write_all(&(1_000_000u32).to_be_bytes()).await.unwrap();
        // Hold the stream open: the refusal must not wait for the body.
        let mut sink = [0u8; 64];
        let _ = b.read(&mut sink).await;
        let _ = b.read(&mut sink).await;
    };
    let (result, ()) = tokio::join!(session(&topic, &victim, limits).run(a), attacker);
    assert!(
        matches!(result, Err(StreamSyncError::FrameTooLarge)),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_local_operation_over_the_ceiling_fails_the_session() {
    let topic = Topic::random();
    let ada = SigningKey::generate();
    let big = peer(&topic, &[&ada]);
    let header = Header::<()>::builder()
        .body(&vec![7u8; 8192])
        .seq_num(0)
        .build(&ada, ());
    let op = Operation {
        hash: header.hash(),
        header,
        body: Some(Body::from_bytes(&vec![7u8; 8192])),
    };
    big.store
        .insert_indexed_operation(&topic, &op, &0u64)
        .await
        .unwrap();
    let empty = peer(&topic, &[&ada]);

    let (a, b) = duplex(64 * 1024);
    let tight = StreamSyncLimits::new(2048, 1 << 20, Duration::from_secs(5)).unwrap();
    let (sender, _) = tokio::join!(
        session(&topic, &big, tight).run(a),
        session(&topic, &empty, tight).run(b),
    );
    assert!(
        matches!(sender, Err(StreamSyncError::FrameTooLarge)),
        "{sender:?}"
    );
    assert_eq!(empty.store.operation_count().await.unwrap(), 0);
}

#[tokio::test]
async fn a_silent_peer_hits_the_deadline() {
    let topic = Topic::random();
    let waiting = peer(&topic, &[]);
    let (a, _held_open) = duplex(1024);
    let limits = StreamSyncLimits::new(4096, 1 << 20, Duration::from_millis(200)).unwrap();
    let result = session(&topic, &waiting, limits).run(a).await;
    assert!(
        matches!(result, Err(StreamSyncError::Deadline)),
        "{result:?}"
    );
}

#[test]
fn limits_outside_the_ceilings_are_refused() {
    let s = Duration::from_secs(1);
    assert!(StreamSyncLimits::new(0, 10, s).is_err());
    assert!(StreamSyncLimits::new(u32::MAX, u64::MAX, s).is_err());
    assert!(StreamSyncLimits::new(1024, 512, s).is_err());
    assert!(StreamSyncLimits::new(1024, 4096, Duration::ZERO).is_err());
}
