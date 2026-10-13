// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use identity::InMemoryProvider;
use p2panda_core::prune::PruneFlag;
use p2panda_core::{Body, Header, SigningKey};
use proofs::Digest;

const KEY: [u8; 32] = [0x73; 32];

async fn engine(seed: u8) -> (ConversationEngine, [u8; 32]) {
    let engine = ConversationEngine::new(Arc::new(InMemoryProvider::from_seed([seed; 32])));
    let id = engine.open(KEY).await.unwrap();
    (engine, id)
}

fn text(body: &str) -> PostKind {
    PostKind::Text {
        channel: ChannelName::new("session"),
        text: body.into(),
        timestamp_ms: 1,
    }
}

async fn operation(
    engine: &ConversationEngine,
    id: &[u8; 32],
    post: PostId,
) -> Operation<CabalExt> {
    engine
        .sync_store(id)
        .unwrap()
        .get_operation(&post.0.into())
        .await
        .unwrap()
        .unwrap()
}

fn ids(history: CabalHistory) -> Vec<PostId> {
    history.posts.iter().map(hash_post).collect()
}

#[tokio::test]
async fn ordinary_authoring_signs_complete_cross_author_frontiers() {
    let (alice, id) = engine(1).await;
    let (bob, _) = engine(2).await;
    let (late, _) = engine(3).await;
    let a = alice
        .post_text(&id, "session", "question", 9000)
        .await
        .unwrap();
    let a_op = operation(&alice, &id, a).await;
    bob.ingest_operation(&id, &a_op).await.unwrap();
    let b = bob.post_text(&id, "session", "answer", 1).await.unwrap();
    let b_op = operation(&bob, &id, b).await;
    assert_eq!(b_op.header.extensions.parents, vec![a.0]);

    // A header whose dependencies are missing is retained but not observed as
    // complete history by an unrelated local author.
    late.ingest_operation(&id, &b_op).await.unwrap();
    let unrelated = late
        .post_text(&id, "session", "independent", 0)
        .await
        .unwrap();
    assert!(
        operation(&late, &id, unrelated)
            .await
            .header
            .extensions
            .parents
            .is_empty()
    );
    let pending = late.causal_history(&id, "session").await.unwrap();
    assert_eq!(ids(pending.clone()), vec![unrelated]);
    assert_eq!(pending.pending[0].missing, vec![a.0]);
    late.ingest_operation(&id, &a_op).await.unwrap();
    let complete = late.causal_history(&id, "session").await.unwrap();
    assert!(complete.pending.is_empty());
    let ordered = ids(complete);
    assert!(ordered.iter().position(|p| *p == a) < ordered.iter().position(|p| *p == b));

    alice.ingest_operation(&id, &b_op).await.unwrap();
    let c = alice.post_text(&id, "notes", "follow-up", 0).await.unwrap();
    let c_op = operation(&alice, &id, c).await;
    assert_eq!(c_op.header.backlink.unwrap().as_bytes(), &a.0);
    assert_eq!(c_op.header.extensions.parents, vec![b.0]);
    bob.ingest_operation(&id, &c_op).await.unwrap();
    assert_eq!(
        ids(alice.causal_history(&id, "session").await.unwrap()),
        vec![a, b]
    );
    assert_eq!(
        ids(bob.causal_history(&id, "session").await.unwrap()),
        vec![a, b]
    );
    assert_eq!(
        ids(bob.causal_history(&id, "notes").await.unwrap()),
        vec![c]
    );
}

#[tokio::test]
async fn blocked_local_tail_refuses_authoring_without_mutation() {
    let (owner, id) = engine(4).await;
    let keypair = InMemoryProvider::from_seed([4; 32])
        .derive_keypair(&KEY)
        .unwrap();
    let post = sign_post(
        &keypair,
        id,
        0,
        None,
        vec![PostId([99; 32])],
        text("blocked"),
    );
    owner.ingest_post(&id, post).await.unwrap();
    assert!(matches!(
        owner.post_text(&id, "session", "no", 1).await,
        Err(MurmError::IncompleteAuthorHead)
    ));
    assert_eq!(
        owner
            .session(&id)
            .unwrap()
            .store
            .operations()
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn parent_limit_refuses_an_oversized_observed_frontier_without_truncation() {
    let (owner, id) = engine(5).await;
    for seed in 10..75 {
        let keypair = InMemoryProvider::from_seed([seed; 32])
            .derive_keypair(&KEY)
            .unwrap();
        owner
            .ingest_post(
                &id,
                sign_post(&keypair, id, 0, None, Vec::new(), text("peer")),
            )
            .await
            .unwrap();
    }
    let result = owner.post_text(&id, "session", "too many", 1).await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("65 causal parents")
    );
    assert_eq!(
        owner
            .session(&id)
            .unwrap()
            .store
            .operations()
            .await
            .unwrap()
            .len(),
        65
    );
    assert_eq!(
        *owner.session(&id).unwrap().author_head.lock().await,
        (0, None)
    );
}

#[tokio::test]
async fn only_an_authorized_checkpoint_cuts_its_own_prefix() {
    let (alice, id) = engine(6).await;
    let (bob, _) = engine(7).await;
    let revision = Digest::blake3(b"accepted cabal retention policy");
    let old = alice.post_text(&id, "session", "old", 9).await.unwrap();
    let old_op = operation(&alice, &id, old).await;
    assert!(
        alice
            .post_retention_checkpoint(&id, revision.clone(), 1)
            .await
            .is_err()
    );
    let authority = CabalCheckpointAuthority::new(
        revision.clone(),
        [alice.author_public_key(&id).unwrap().to_bytes()],
    );
    alice
        .set_checkpoint_authority(&id, Some(authority.clone()))
        .await
        .unwrap();
    assert!(
        alice
            .post_retention_checkpoint(&id, Digest::blake3(b"wrong revision"), 1)
            .await
            .is_err()
    );
    assert!(
        alice
            .sync_store(&id)
            .unwrap()
            .get_operation(&old.0.into())
            .await
            .unwrap()
            .is_some()
    );

    // Bob's ordinary reply names the old record; pruning Alice's log cannot
    // claim that Bob's reference is now resolved or erase Bob's record.
    bob.ingest_operation(&id, &old_op).await.unwrap();
    let reply = bob.post_text(&id, "session", "reply", 0).await.unwrap();
    let reply_op = operation(&bob, &id, reply).await;
    alice.ingest_operation(&id, &reply_op).await.unwrap();
    let checkpoint = alice
        .post_retention_checkpoint(&id, revision.clone(), 1)
        .await
        .unwrap();
    let checkpoint_op = operation(&alice, &id, checkpoint).await;
    assert!(checkpoint_op.header.extensions.prune_flag.is_set());
    assert_eq!(checkpoint_op.header.backlink.unwrap().as_bytes(), &old.0);
    assert!(alice.get_post(&id, &old).is_none());
    assert!(
        alice
            .sync_store(&id)
            .unwrap()
            .get_operation(&old.0.into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        alice
            .sync_store(&id)
            .unwrap()
            .get_operation(&reply.0.into())
            .await
            .unwrap()
            .is_some()
    );
    let view = alice.causal_history(&id, "session").await.unwrap();
    assert_eq!(view.retention_checkpoints, vec![checkpoint]);
    assert_eq!(view.pending[0].operation, reply.0);
    assert_eq!(view.pending[0].missing, vec![old.0]);
    let tail = alice
        .post_text(&id, "session", "after retention", 0)
        .await
        .unwrap();
    let tail_op = operation(&alice, &id, tail).await;
    assert!(tail_op.header.extensions.parents.is_empty());
    assert_eq!(
        ids(alice.causal_history(&id, "session").await.unwrap()),
        vec![tail]
    );

    // Late bootstrap has no prefix. Default refusal leaves the store empty;
    // explicit authority admits the root through p2panda's prune validation.
    let (late, _) = engine(8).await;
    assert!(late.ingest_operation(&id, &checkpoint_op).await.is_err());
    assert!(
        late.session(&id)
            .unwrap()
            .store
            .operations()
            .await
            .unwrap()
            .is_empty()
    );
    late.set_checkpoint_authority(&id, Some(authority))
        .await
        .unwrap();
    late.ingest_operation(&id, &checkpoint_op).await.unwrap();
    late.ingest_operation(&id, &tail_op).await.unwrap();
    assert_eq!(
        ids(late.causal_history(&id, "session").await.unwrap()),
        vec![tail]
    );
    let bob_checkpoint = sign_post(
        &InMemoryProvider::from_seed([7; 32])
            .derive_keypair(&KEY)
            .unwrap(),
        id,
        1,
        Some(reply),
        Vec::new(),
        PostKind::RetentionCheckpoint {
            authority_revision: revision,
            timestamp_ms: 0,
        },
    );
    assert!(late.ingest_post(&id, bob_checkpoint).await.is_err());
    late.sync_store(&id)
        .unwrap()
        .delete_operation(&checkpoint.0.into())
        .await
        .unwrap();
    let missing = late.causal_history(&id, "session").await.unwrap();
    assert!(missing.retention_checkpoints.is_empty());
    assert!(missing.posts.is_empty());
    assert_eq!(missing.pending[0].missing, vec![checkpoint.0]);
}

#[tokio::test]
async fn a_prune_marker_on_an_ordinary_post_is_refused_before_storage() {
    let (owner, id) = engine(9).await;
    let signer = SigningKey::from_bytes(
        &InMemoryProvider::from_seed([9; 32])
            .derive_keypair(&KEY)
            .unwrap()
            .to_seed(),
    );
    let ext = CabalExt {
        cabal_id: id,
        channel: "session".into(),
        prune_flag: PruneFlag::new(true),
        authority_revision: Some(Digest::blake3(b"spoof")),
        ..Default::default()
    };
    let header = Header::builder().body(b"spoof").build(&signer, ext);
    let op = Operation {
        hash: header.hash(),
        header,
        body: Some(Body::from_bytes(b"spoof")),
    };
    assert!(
        owner
            .ingest_operation(&id, &op)
            .await
            .unwrap_err()
            .to_string()
            .contains("ordinary post carries checkpoint fields")
    );
    assert!(
        owner
            .session(&id)
            .unwrap()
            .store
            .operations()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn durable_pruned_history_needs_explicit_owner_authority_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let revision = Digest::blake3(b"durable policy");
    let authority;
    let checkpoint;
    let tail;
    let id;
    {
        let owner = ConversationEngine::with_storage(
            Arc::new(InMemoryProvider::from_seed([90; 32])),
            ConversationStorage::redb(directory.path()),
        );
        id = owner.open(KEY).await.unwrap();
        authority = CabalCheckpointAuthority::new(
            revision.clone(),
            [owner.author_public_key(&id).unwrap().to_bytes()],
        );
        owner
            .post_text(&id, "session", "discarded prefix", 99)
            .await
            .unwrap();
        owner
            .set_checkpoint_authority(&id, Some(authority.clone()))
            .await
            .unwrap();
        checkpoint = owner
            .post_retention_checkpoint(&id, revision, 1)
            .await
            .unwrap();
        tail = owner
            .post_text(&id, "session", "retained", 0)
            .await
            .unwrap();
    }
    let reopened = ConversationEngine::with_storage(
        Arc::new(InMemoryProvider::from_seed([90; 32])),
        ConversationStorage::redb(directory.path()),
    );
    reopened.open(KEY).await.unwrap();
    assert!(reopened.causal_history(&id, "session").await.is_err());
    reopened
        .set_checkpoint_authority(&id, Some(authority))
        .await
        .unwrap();
    let view = reopened.causal_history(&id, "session").await.unwrap();
    assert_eq!(view.retention_checkpoints, vec![checkpoint]);
    assert_eq!(ids(view), vec![tail]);
    let next = reopened
        .post_text(&id, "session", "continuation", 0)
        .await
        .unwrap();
    let op = operation(&reopened, &id, next).await;
    assert_eq!(op.header.seq_num, 3);
    assert_eq!(op.header.backlink.unwrap().as_bytes(), &tail.0);
}

#[tokio::test]
async fn a_protected_drop_bootstraps_an_authorized_pruned_root() {
    let (source, id) = engine(91).await;
    let revision = Digest::blake3(b"offline cabal retention");
    let authority = CabalCheckpointAuthority::new(
        revision.clone(),
        [source.author_public_key(&id).unwrap().to_bytes()],
    );
    source
        .set_checkpoint_authority(&id, Some(authority.clone()))
        .await
        .unwrap();
    source
        .post_text(&id, "session", "removed", 99)
        .await
        .unwrap();
    let root = source
        .post_retention_checkpoint(&id, revision, 1)
        .await
        .unwrap();
    let tail = source
        .post_text(&id, "session", "offline tail", 0)
        .await
        .unwrap();
    let records = stickleback::export_topic_operations::<_, _, u64>(
        &source.sync_store(&id).unwrap(),
        &p2panda_core::Topic::from(id),
        stickleback::DropExportProfile::default(),
    )
    .await
    .unwrap();
    assert_eq!(records.len(), 2);
    let mut bytes = Vec::new();
    stickleback::write_protected_drop(
        &mut bytes,
        &records,
        DropLimits::default(),
        &source.keyring(&id).unwrap(),
    )
    .unwrap();
    let (receiver, _) = engine(92).await;
    assert!(
        receiver
            .import_protected_drop(&id, std::io::Cursor::new(&bytes), DropLimits::default())
            .await
            .is_err()
    );
    assert!(
        receiver
            .session(&id)
            .unwrap()
            .store
            .operations()
            .await
            .unwrap()
            .is_empty()
    );
    receiver
        .set_checkpoint_authority(&id, Some(authority))
        .await
        .unwrap();
    let (report, _) = receiver
        .import_protected_drop(&id, std::io::Cursor::new(bytes), DropLimits::default())
        .await
        .unwrap();
    assert_eq!(report.accepted, 2);
    let history = receiver.causal_history(&id, "session").await.unwrap();
    assert_eq!(history.retention_checkpoints, vec![root]);
    assert_eq!(ids(history), vec![tail]);
}

#[tokio::test]
async fn changing_retention_authority_preserves_old_roots_but_gates_new_ones() {
    let (owner, id) = engine(93).await;
    let author = owner.author_public_key(&id).unwrap().to_bytes();
    let first = Digest::blake3(b"first policy");
    owner
        .set_checkpoint_authority(
            &id,
            Some(CabalCheckpointAuthority::new(first.clone(), [author])),
        )
        .await
        .unwrap();
    owner.post_text(&id, "session", "prefix", 9).await.unwrap();
    owner
        .post_retention_checkpoint(&id, first.clone(), 1)
        .await
        .unwrap();
    let tail = owner.post_text(&id, "session", "tail", 0).await.unwrap();
    owner.set_checkpoint_authority(&id, None).await.unwrap();
    assert_eq!(
        ids(owner.causal_history(&id, "session").await.unwrap()),
        vec![tail]
    );
    assert!(
        owner
            .post_retention_checkpoint(&id, first.clone(), 0)
            .await
            .is_err()
    );
    let next = Digest::blake3(b"next policy");
    owner
        .set_checkpoint_authority(
            &id,
            Some(CabalCheckpointAuthority::new(next.clone(), [author])),
        )
        .await
        .unwrap();
    assert!(
        owner
            .post_retention_checkpoint(&id, first, 0)
            .await
            .is_err()
    );
    let root = owner.post_retention_checkpoint(&id, next, 0).await.unwrap();
    let view = owner.causal_history(&id, "session").await.unwrap();
    assert_eq!(view.retention_checkpoints, vec![root]);
    assert!(view.posts.is_empty());
    assert!(view.pending.is_empty());
}
