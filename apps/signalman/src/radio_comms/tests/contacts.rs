use super::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Snapshot {
    bytes: Option<Vec<u8>>,
    refuse: bool,
    commits: usize,
}
#[derive(Clone, Default)]
struct MemoryJournal(Arc<Mutex<Snapshot>>);
impl RadioJournal for MemoryJournal {
    fn load(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().bytes.clone())
    }
    fn commit(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.commits += 1;
        if state.refuse {
            return Err(std::io::Error::other(
                "injected atomic contact-save failure",
            ));
        }
        state.bytes = Some(bytes.to_vec());
        Ok(())
    }
}

fn peer(seed: u8) -> tucket::node::Node {
    tucket::node::Node::new(
        tucket::identity::LocalIdentity::from_seed([seed; 32]),
        false,
    )
}

async fn exchange(
    owner: &RadioComms,
    session: &mut ResidentSession<'_>,
    remote: &mut tucket::node::Node,
    conversation: ConversationId,
    now: u64,
    text: &str,
) -> MessageId {
    let id = send(&owner.model(), conversation.clone(), text).await;
    assert_eq!(
        session.dispatch_next(now, 100 + now as u32).unwrap(),
        Some(id.clone())
    );
    session.poll(now).unwrap();
    let tx = session.begin_tx(now + 1).unwrap().unwrap();
    let (events, replies) = remote.on_frame(&tx.frame);
    assert!(events.iter().any(|event| matches!(event,
        tucket::node::Event::Message { message, .. } if message.text == text)));
    session.complete_tx(now + 2, tx.id, true).unwrap();
    for reply in replies {
        session.ingest(now + 3, &reply).unwrap();
    }
    assert_eq!(
        owner
            .model()
            .delivery_status(&conversation, &id)
            .await
            .unwrap(),
        DeliveryStatus::ProtocolAcknowledged
    );
    while let Some(tx) = session.begin_tx(now + 4).unwrap() {
        session.complete_tx(now + 4, tx.id, true).unwrap();
    }
    id
}

#[tokio::test]
async fn real_file_reload_sends_without_fresh_advert_and_preserves_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contacts.json");
    let a = scope(7, 3);
    let mut remote = peer(4);
    let peer_key = remote.identity().pub_key;
    remote.on_frame(&peer(3).advert_frame(1, b"local"));
    let (first, history) = {
        let mailbox = RadioComms::open(a.clone(), FileJournal::new(&path)).unwrap();
        let mut first_runtime = runtime(&a, 3, TUCKET, 10);
        let mut session = mailbox.attach(&mut first_runtime).unwrap();
        session.ingest(1, &remote.advert_frame(1, b"peer")).unwrap();
        assert_eq!(mailbox.meshcore_contacts().unwrap(), vec![peer_key]);
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["version"], 2, "old writers must refuse this cache");
        let first = exchange(
            &mailbox,
            &mut session,
            &mut remote,
            a.meshcore(peer_key),
            4,
            "before restart",
        )
        .await;
        let history = serde_json::to_value(mailbox.records().unwrap()).unwrap();
        (first, history)
    };
    let mailbox = RadioComms::open(a.clone(), FileJournal::new(&path)).unwrap();
    assert_eq!(
        serde_json::to_value(mailbox.records().unwrap()).unwrap(),
        history
    );
    let mut fresh = runtime(&a, 3, TUCKET, 100);
    assert_eq!(fresh.tucket().node().contact_keys().count(), 0);
    let mut session = mailbox.attach(&mut fresh).unwrap();
    assert!(session.knows_tucket_peer(&peer_key));
    assert!(
        session.begin_tx(0).unwrap().is_none(),
        "old pending work must not replay"
    );
    // No peer advert is delivered to this runtime before the new text.
    let second = exchange(
        &mailbox,
        &mut session,
        &mut remote,
        a.meshcore(peer_key),
        10,
        "after restart",
    )
    .await;
    assert_ne!(first, second);
    assert_eq!(mailbox.records().unwrap().len(), 2);
    assert_eq!(mailbox.meshcore_contacts().unwrap(), vec![peer_key]);
}

#[test]
fn failed_save_keeps_last_snapshot_and_stops_carriage() {
    let a = scope(7, 3);
    let store = MemoryJournal::default();
    let mailbox = RadioComms::open(a.clone(), store.clone()).unwrap();
    let mut active = runtime(&a, 3, TUCKET, 10);
    let mut session = mailbox.attach(&mut active).unwrap();
    let mut one = peer(4);
    let mut two = peer(9);
    session.ingest(1, &one.advert_frame(1, b"one")).unwrap();
    let durable = store.0.lock().unwrap().bytes.clone().unwrap();
    store.0.lock().unwrap().refuse = true;
    assert!(session.ingest(2, &two.advert_frame(2, b"two")).is_err());
    assert!(session.begin_tx(3).is_err());
    assert_eq!(store.0.lock().unwrap().bytes.as_ref().unwrap(), &durable);
    assert_eq!(
        mailbox.meshcore_contacts().unwrap(),
        vec![one.identity().pub_key]
    );
    drop(session);
    drop(mailbox);
    store.0.lock().unwrap().refuse = false;
    let reopened = RadioComms::open(a.clone(), store).unwrap();
    let mut fresh = runtime(&a, 3, TUCKET, 100);
    let session = reopened.attach(&mut fresh).unwrap();
    assert!(session.knows_tucket_peer(&one.identity().pub_key));
    assert!(!session.knows_tucket_peer(&two.identity().pub_key));
}

#[test]
fn malformed_or_wrong_scope_cache_is_refused_without_a_repair_write() {
    let a = scope(7, 3);
    let store = MemoryJournal::default();
    let mailbox = RadioComms::open(a.clone(), store.clone()).unwrap();
    let mut active = runtime(&a, 3, TUCKET, 10);
    mailbox
        .attach(&mut active)
        .unwrap()
        .ingest(1, &peer(4).advert_frame(1, b"peer"))
        .unwrap();
    drop(mailbox);
    let original = store.0.lock().unwrap().bytes.clone().unwrap();
    let mut duplicate: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let key = duplicate["meshcore_contacts"][0].clone();
    duplicate["meshcore_contacts"] = serde_json::json!([key, key]);
    let mut weak: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut invalid = [0_u8; 32];
    invalid[0] = 1;
    weak["meshcore_contacts"] = serde_json::json!([invalid]);
    let mut wrong: serde_json::Value = serde_json::from_slice(&original).unwrap();
    wrong["scope"]["realm"] = "another-realm".into();
    let mut downgraded: serde_json::Value = serde_json::from_slice(&original).unwrap();
    downgraded["version"] = 1.into();
    for corrupt in [
        serde_json::to_vec(&duplicate).unwrap(),
        serde_json::to_vec(&weak).unwrap(),
        serde_json::to_vec(&wrong).unwrap(),
        serde_json::to_vec(&downgraded).unwrap(),
        original[..original.len() / 2].to_vec(),
    ] {
        let commits = {
            let mut state = store.0.lock().unwrap();
            state.bytes = Some(corrupt.clone());
            state.commits
        };
        assert!(RadioComms::open(a.clone(), store.clone()).is_err());
        let state = store.0.lock().unwrap();
        assert_eq!(state.commits, commits);
        assert_eq!(state.bytes.as_ref().unwrap(), &corrupt);
    }
}

#[test]
fn repeated_advert_does_not_rewrite_cache_and_inactive_restore_does_not_switch() {
    let a = scope(7, 3);
    let store = MemoryJournal::default();
    let mailbox = RadioComms::open(a.clone(), store.clone()).unwrap();
    let mut active = runtime(&a, 3, TUCKET, 10);
    let mut remote = peer(4);
    {
        let mut session = mailbox.attach(&mut active).unwrap();
        session.ingest(1, &remote.advert_frame(1, b"peer")).unwrap();
        let commits = store.0.lock().unwrap().commits;
        session.ingest(2, &remote.advert_frame(2, b"peer")).unwrap();
        assert_eq!(store.0.lock().unwrap().commits, commits);
    }
    let mut inactive = runtime(&a, 3, SENNET, 100);
    assert!(inactive.tucket().is_paused());
    {
        let mut session = mailbox.attach(&mut inactive).unwrap();
        assert!(session.knows_tucket_peer(&remote.identity().pub_key));
        assert!(session.begin_tx(0).unwrap().is_none());
    }
    assert!(inactive.tucket().is_paused());
    assert_eq!(inactive.active().unwrap().instance, SENNET);
}

#[test]
fn legacy_journal_without_contacts_remains_readable() {
    let a = scope(7, 3);
    let store = MemoryJournal::default();
    drop(RadioComms::open(a.clone(), store.clone()).unwrap());
    let value: serde_json::Value =
        serde_json::from_slice(store.0.lock().unwrap().bytes.as_ref().unwrap()).unwrap();
    assert!(value.get("meshcore_contacts").is_none());
    let mailbox = RadioComms::open(a.clone(), store).unwrap();
    assert!(mailbox.meshcore_contacts().unwrap().is_empty());
    let mut active = runtime(&a, 3, TUCKET, 10);
    assert!(
        mailbox
            .attach(&mut active)
            .unwrap()
            .begin_tx(0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn too_small_runtime_refuses_cache_without_partial_import_or_attachment() {
    let a = scope(7, 3);
    let store = MemoryJournal::default();
    let mailbox = RadioComms::open(a.clone(), store).unwrap();
    let mut initial = runtime(&a, 3, TUCKET, 10);
    let mut one = peer(4);
    let mut two = peer(9);
    {
        let mut session = mailbox.attach(&mut initial).unwrap();
        session.ingest(1, &one.advert_frame(1, b"one")).unwrap();
        session.ingest(2, &two.advert_frame(2, b"two")).unwrap();
    }
    let node = tucket::node::Node::with_capacity(
        tucket::identity::LocalIdentity::from_seed([3; 32]),
        false,
        tucket::node::NodeCapacity::new(1, 32).unwrap(),
    )
    .unwrap();
    let config = Config {
        controller: ControllerConfig {
            home: TUCKET,
            pin: Some(TUCKET),
            installed: InstalledPersonalitySet::new(&[TUCKET]).unwrap(),
            coverage: CoveragePolicy::AllowGap,
            max_excursion_ms: 1000,
            return_budget_ms: 10,
            max_defer_ms: 100,
            transition_timeout_ms: 10,
        },
        profiles: [PhyProfile::meshcore(906_875_000, 250_000, 10, 5); 3],
        tx_budget_ms: 2,
        frame_ttl_ms: 1000,
    };
    let mut small = Runtime::tucket_only(
        0,
        config,
        tucket::instance::Instance::new(
            node,
            tucket::instance::InstanceConfig::new(2, 10).unwrap(),
        ),
    )
    .unwrap();
    small
        .restore_tucket_contacts(&[one.identity().pub_key])
        .unwrap();
    assert!(mailbox.attach(&mut small).is_err());
    assert_eq!(
        small.tucket().node().contact_keys().collect::<Vec<_>>(),
        vec![one.identity().pub_key]
    );
    let mut sufficient = runtime(&a, 3, TUCKET, 100);
    let session = mailbox.attach(&mut sufficient).unwrap();
    assert!(session.knows_tucket_peer(&one.identity().pub_key));
    assert!(session.knows_tucket_peer(&two.identity().pub_key));
}
