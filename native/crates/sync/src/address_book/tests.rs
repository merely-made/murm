// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use muniment::{JsonCodec, MemoryBackend};
use p2panda_core::SigningKey;

use super::*;

fn book() -> MunimentAddressBook<MemoryBackend, JsonCodec> {
    MunimentAddressBook::new(MemoryBackend::new())
}

fn node() -> NodeInfo {
    NodeInfo::new(SigningKey::generate().verifying_key())
}

#[tokio::test]
async fn insert_reports_new_then_update() {
    let book = book();
    let info = node();

    assert!(book.insert_node_info(info.clone()).await.unwrap());
    // Same node again is an update, not an insert.
    assert!(!book.insert_node_info(info.clone()).await.unwrap());
    assert_eq!(book.all_nodes_len().await.unwrap(), 1);
    assert_eq!(book.node_info(&info.node_id).await.unwrap(), Some(info));
}

#[tokio::test]
async fn insert_if_absent_writes_only_a_missing_record() {
    let book = book();
    let info = node();

    assert!(book.insert_node_info_if_absent(info.clone()).await.unwrap());
    assert_eq!(book.node_info(&info.node_id).await.unwrap(), Some(info));
}

/// The case it exists for: a record written between a caller's check and
/// its write survives, rather than being replaced by a bare one.
#[tokio::test]
async fn insert_if_absent_keeps_a_record_already_there() {
    let book = book();
    let mut known = node();
    known.bootstrap = true;
    book.insert_node_info(known.clone()).await.unwrap();

    let bare = NodeInfo::new(known.node_id);
    assert!(!book.insert_node_info_if_absent(bare).await.unwrap());
    assert_eq!(book.node_info(&known.node_id).await.unwrap(), Some(known));
}

#[tokio::test]
async fn remove_reports_whether_it_existed() {
    let book = book();
    let info = node();

    assert!(!book.remove_node_info(&info.node_id).await.unwrap());
    book.insert_node_info(info.clone()).await.unwrap();
    assert!(book.remove_node_info(&info.node_id).await.unwrap());
    assert_eq!(book.node_info(&info.node_id).await.unwrap(), None);
}

#[tokio::test]
async fn removing_a_node_takes_its_topics() {
    let book = book();
    let info = node();
    let topic = Topic::random();

    book.insert_node_info(info.clone()).await.unwrap();
    book.set_topics(info.node_id, HashSet::from([topic]))
        .await
        .unwrap();
    book.remove_node_info(&info.node_id).await.unwrap();

    assert!(book.node_topics(&info.node_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn set_topics_overwrites_rather_than_extends() {
    let book = book();
    let info = node();
    let (a, b) = (Topic::random(), Topic::random());

    book.insert_node_info(info.clone()).await.unwrap();
    book.set_topics(info.node_id, HashSet::from([a]))
        .await
        .unwrap();
    book.set_topics(info.node_id, HashSet::from([b]))
        .await
        .unwrap();

    assert_eq!(
        book.node_topics(&info.node_id).await.unwrap(),
        HashSet::from([b])
    );
}

#[tokio::test]
async fn lookup_by_topic_matches_any_of_them() {
    let book = book();
    let (weather, sport) = (Topic::random(), Topic::random());

    let subscriber = node();
    book.insert_node_info(subscriber.clone()).await.unwrap();
    book.set_topics(subscriber.node_id, HashSet::from([weather]))
        .await
        .unwrap();

    let bystander = node();
    book.insert_node_info(bystander.clone()).await.unwrap();

    let found = book.node_infos_by_topics(&[weather, sport]).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].node_id, subscriber.node_id);
}

#[tokio::test]
async fn bootstrap_nodes_are_counted_and_pickable() {
    let book = book();

    let mut boot = node();
    boot.bootstrap = true;
    book.insert_node_info(boot.clone()).await.unwrap();
    book.insert_node_info(node()).await.unwrap();

    assert_eq!(book.all_nodes_len().await.unwrap(), 2);
    assert_eq!(book.all_bootstrap_nodes_len().await.unwrap(), 1);
    assert_eq!(
        book.random_bootstrap_node()
            .await
            .unwrap()
            .map(|i| i.node_id),
        Some(boot.node_id)
    );
}

#[tokio::test]
async fn random_node_skips_stale_ones() {
    let book = book();

    let mut stale = node();
    stale.metrics.report_failed_connection();
    assert!(stale.is_stale(), "fixture must actually be stale");
    book.insert_node_info(stale).await.unwrap();

    assert_eq!(book.random_node().await.unwrap(), None);

    let fresh = node();
    book.insert_node_info(fresh.clone()).await.unwrap();
    assert_eq!(
        book.random_node().await.unwrap().map(|i| i.node_id),
        Some(fresh.node_id)
    );
}

#[tokio::test]
async fn selected_node_infos_skips_unknown_ids() {
    let book = book();
    let known = node();
    let unknown = node();

    book.insert_node_info(known.clone()).await.unwrap();
    let found = book
        .selected_node_infos(&[known.node_id, unknown.node_id])
        .await
        .unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].node_id, known.node_id);
}

/// Writes a record stamped in the past, which is the only way to age an
/// entry without waiting. Mirrors what `write_record` stores.
async fn insert_aged(
    book: &MunimentAddressBook<MemoryBackend, JsonCodec>,
    info: NodeInfo,
    age: Duration,
) {
    let record = Record {
        updated_at: now_secs() - age.as_secs(),
        info,
    };
    book.backend()
        .put(
            &node_key(&record.info.node_id),
            &JsonCodec::encode(&record).unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn remove_older_than_ages_by_local_stamp() {
    let book = book();
    let fresh = node();
    let old = node();

    book.insert_node_info(fresh.clone()).await.unwrap();
    insert_aged(&book, old.clone(), Duration::from_secs(7200)).await;
    assert_eq!(book.all_nodes_len().await.unwrap(), 2);

    assert_eq!(
        book.remove_older_than(Duration::from_secs(3600))
            .await
            .unwrap(),
        1
    );
    assert_eq!(book.node_info(&old.node_id).await.unwrap(), None);
    assert_eq!(
        book.node_info(&fresh.node_id)
            .await
            .unwrap()
            .map(|i| i.node_id),
        Some(fresh.node_id)
    );
}

#[tokio::test]
async fn remove_older_than_keeps_same_second_entries() {
    let book = book();
    book.insert_node_info(node()).await.unwrap();

    // The comparison is strict, matching the SQLite backend's
    // `updated_at < UNIXEPOCH() - ?`, so an entry written this second
    // survives even a zero-length window.
    assert_eq!(book.remove_older_than(Duration::ZERO).await.unwrap(), 0);
    assert_eq!(book.all_nodes_len().await.unwrap(), 1);
}

#[tokio::test]
async fn remove_older_than_takes_topics_with_it() {
    let book = book();
    let old = node();
    let topic = Topic::random();

    insert_aged(&book, old.clone(), Duration::from_secs(7200)).await;
    book.set_topics(old.node_id, HashSet::from([topic]))
        .await
        .unwrap();

    assert_eq!(
        book.remove_older_than(Duration::from_secs(3600))
            .await
            .unwrap(),
        1
    );
    assert!(book.node_topics(&old.node_id).await.unwrap().is_empty());
}
