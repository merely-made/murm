use super::*;
use radio_hand::instances::{Config, RETINUE, Runtime, SENNET, TUCKET};
use selvage::{
    PhyProfile,
    personality::{ControllerConfig, CoveragePolicy, InstalledPersonalitySet},
};
use sennet::{
    flood::{ManagedFloodConfig, RelayDelayWindow},
    instance::{PacketIdLease, SennetInstance, SennetInstanceConfig},
    node::Channel,
    node_info::NodeDirectoryConfig,
    transport::ChannelKey,
};
pub(super) fn scope(node: u32, seed: u8) -> RadioScope {
    RadioScope {
        realm: "household-a".into(),
        meshtastic_node: node,
        meshtastic_channel: 8,
        meshcore_key: tucket::identity::LocalIdentity::from_seed([seed; 32])
            .identity()
            .pub_key,
    }
}
pub(super) fn runtime(
    scope: &RadioScope,
    seed: u8,
    home: selvage::personality::PersonalityId,
    next: u32,
) -> Runtime {
    // Fast model tests select zero turnaround; the physical-driver test below
    // selects and checks the real response delay explicitly.
    runtime_with_reply_delay(scope, seed, home, next, 0)
}
pub(super) fn runtime_with_reply_delay(
    scope: &RadioScope,
    seed: u8,
    home: selvage::personality::PersonalityId,
    next: u32,
    reply_delay_ms: u64,
) -> Runtime {
    runtime_configured(scope, seed, home, next, reply_delay_ms, None)
}
fn runtime_configured(
    scope: &RadioScope,
    seed: u8,
    home: selvage::personality::PersonalityId,
    next: u32,
    reply_delay_ms: u64,
    peer: Option<(u32, u8)>,
) -> Runtime {
    let node = retinue::node::Node::<8, 4, 1, 4>::new(
        retinue::identity::PrivateIdentity::from_secret_bytes(&[1; 64]),
        retinue::destination::DestinationName::new("retinue", ["resident"]).name_hash(),
    );
    let config = Config {
        controller: ControllerConfig {
            home,
            pin: None,
            installed: InstalledPersonalitySet::new(&[RETINUE, SENNET, TUCKET]).unwrap(),
            coverage: CoveragePolicy::AllowGap,
            max_excursion_ms: 60_000,
            return_budget_ms: 10,
            max_defer_ms: 100,
            transition_timeout_ms: 10,
        },
        profiles: [PhyProfile::meshtastic_long_fast(915_000_000); 3],
        tx_budget_ms: 1,
        frame_ttl_ms: 60_000,
    };
    let mut sennet = SennetInstance::new(
        SennetInstanceConfig {
            channel: Channel {
                hash: 8,
                key: ChannelKey::Aes128([2; 16]),
            },
            flood: ManagedFloodConfig {
                channel_hash: 8,
                relay_node: 1,
                seen_capacity: 16,
                delay: RelayDelayWindow::new(std::time::Duration::ZERO, std::time::Duration::ZERO)
                    .unwrap(),
            },
            directory: NodeDirectoryConfig::default(),
            pending_ttl: 60_000,
        },
        PacketIdLease::new(scope.meshtastic_node, next, next + 64, next).unwrap(),
    )
    .unwrap();
    if let Some((node, key)) = peer {
        sennet
            .install_pki(
                node,
                sennet::pki::LocalIdentity::from_secret([seed; 32]),
                sennet::pki::LocalIdentity::from_secret([key; 32]).public_key(),
                50,
                16,
            )
            .unwrap();
    }
    let tucket = tucket::instance::Instance::new(
        tucket::node::Node::new(
            tucket::identity::LocalIdentity::from_seed([seed; 32]),
            false,
        ),
        tucket::instance::InstanceConfig::new(8, 10)
            .unwrap()
            .with_reply_delay_ms(reply_delay_ms),
    );
    Runtime::new(0, config, node, sennet, tucket).unwrap()
}
pub(super) fn owner(scope: RadioScope, directory: &std::path::Path, name: &str) -> RadioComms {
    RadioComms::open(scope, FileJournal::new(directory.join(name))).unwrap()
}
async fn send(model: &comms::Comms, conversation: ConversationId, body: &str) -> MessageId {
    let mut draft = Draft::reply_to(conversation);
    draft.body = body.into();
    model.send(&draft).await.unwrap()
}
#[tokio::test]
async fn meshtastic_real_owner_round_trip_and_reopen_preserve_packet_identity() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let sender = owner(a.clone(), dir.path(), "a.json");
    let receiver = owner(b.clone(), dir.path(), "b.json");
    let id = send(&sender.model(), a.meshtastic(u32::MAX), "real channel text").await;
    let mut ar = runtime(&a, 3, SENNET, 10);
    let mut br = runtime(&b, 4, SENNET, 100);
    let mut ax = sender.attach(&mut ar).unwrap();
    let mut bx = receiver.attach(&mut br).unwrap();
    assert_eq!(ax.dispatch_next(1, 50).unwrap(), Some(id.clone()));
    let tx = ax.begin_tx(3).unwrap().unwrap();
    assert_eq!(
        sender
            .model()
            .delivery_status(&a.meshtastic(u32::MAX), &id)
            .await
            .unwrap(),
        DeliveryStatus::Queued(DeliveryQueueReason::ReadyForCarriage)
    );
    bx.ingest(2, &tx.frame).unwrap();
    bx.ingest(3, &tx.frame).unwrap();
    ax.complete_tx(3, tx.id, true).unwrap();
    assert_eq!(
        sender
            .model()
            .delivery_status(&a.meshtastic(u32::MAX), &id)
            .await
            .unwrap(),
        DeliveryStatus::HandedToRadio
    );
    let records = receiver.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message.body.text(), "real channel text");
    assert_eq!(records[0].message.author.address, "00000007");
    assert_eq!(records[0].delivery, DeliveryStatus::ReceivedByRadio);
    assert!(matches!(
        records[0].facts[0],
        RadioFact::MeshtasticPacket {
            source: 7,
            destination: u32::MAX,
            packet_id: 10,
            ..
        }
    ));
    drop(bx);
    drop(receiver);
    let reopened = owner(b.clone(), dir.path(), "b.json");
    let mut fresh = runtime(&b, 4, SENNET, 164);
    let mut session = reopened.attach(&mut fresh).unwrap();
    session.ingest(4, &tx.frame).unwrap();
    assert_eq!(reopened.records().unwrap().len(), 1);
    assert!(reopened.diagnostics().unwrap()[0].contains("unauthenticated"));
}
#[tokio::test]
async fn meshcore_verified_contact_text_retries_and_protocol_ack_use_full_keys() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let sender = owner(a.clone(), dir.path(), "a.json");
    let receiver = owner(b.clone(), dir.path(), "b.json");
    let mut ar = runtime(&a, 3, TUCKET, 10);
    let mut br = runtime(&b, 4, TUCKET, 100);
    let mut an =
        tucket::node::Node::new(tucket::identity::LocalIdentity::from_seed([3; 32]), false);
    let mut bn =
        tucket::node::Node::new(tucket::identity::LocalIdentity::from_seed([4; 32]), false);
    let mut ax = sender.attach(&mut ar).unwrap();
    let mut bx = receiver.attach(&mut br).unwrap();
    ax.ingest(1, &bn.advert_frame(50, b"B")).unwrap();
    bx.ingest(1, &an.advert_frame(50, b"A")).unwrap();
    // Drain advert forwarding before the caller's own text operation.
    while let Some(tx) = ax.begin_tx(3).unwrap() {
        ax.complete_tx(3, tx.id, true).unwrap();
    }
    while let Some(tx) = bx.begin_tx(3).unwrap() {
        bx.complete_tx(3, tx.id, true).unwrap();
    }
    let id = send(&sender.model(), a.meshcore(b.meshcore_key), "pairwise text").await;
    ax.dispatch_next(4, 51).unwrap();
    ax.poll(4).unwrap();
    let tx = ax.begin_tx(5).unwrap().unwrap();
    bx.ingest(5, &tx.frame).unwrap();
    ax.complete_tx(6, tx.id, true).unwrap();
    assert_eq!(
        receiver.records().unwrap()[0].message.author.address,
        hex::encode(a.meshcore_key)
    );
    assert!(
        matches!(receiver.records().unwrap()[0].facts[0],RadioFact::MeshCoreText{sender_key,..} if sender_key==a.meshcore_key)
    );
    let mut delayed = Vec::new();
    while let Some(reply) = bx.begin_tx(8).unwrap() {
        delayed.push(reply.frame);
        bx.complete_tx(8, reply.id, true).unwrap();
    }
    // Withhold the first ACK and allow a genuine protocol retry.
    ax.poll(20).unwrap();
    let retry = ax.begin_tx(21).unwrap().unwrap();
    bx.ingest(21, &retry.frame).unwrap();
    ax.complete_tx(22, retry.id, true).unwrap();
    assert_eq!(receiver.records().unwrap().len(), 1);
    assert!(
        receiver.records().unwrap()[0]
            .facts
            .iter()
            .any(|fact| matches!(fact, RadioFact::MeshCoreText { attempt: 1, .. }))
    );
    for reply in delayed {
        ax.ingest(23, &reply).unwrap();
    }
    assert_eq!(
        sender
            .model()
            .delivery_status(&a.meshcore(b.meshcore_key), &id)
            .await
            .unwrap(),
        DeliveryStatus::ProtocolAcknowledged
    );
    assert!(
        sender.records().unwrap()[0]
            .facts
            .iter()
            .any(|f| matches!(f, RadioFact::MeshCoreAck { .. }))
    );
    ax.poll(30).unwrap();
    while let Some(control) = ax.begin_tx(31).unwrap() {
        assert!(
            control.id.operation.is_none(),
            "ACK retired the text operation"
        );
        ax.complete_tx(31, control.id, true).unwrap();
    }
    // A second identical wire tuple is refused, preserving the first receipt.
    send(&sender.model(), a.meshcore(b.meshcore_key), "pairwise text").await;
    assert!(ax.dispatch_next(32, 51).is_err());
}
#[tokio::test]
async fn routing_refusals_precede_mutation_and_uncertain_sessions_never_auto_replay() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let owner = owner(a.clone(), dir.path(), "a.json");
    let model = owner.model();
    let mut bad = Draft::reply_to(a.meshtastic(8));
    bad.body = "ok".into();
    bad.subject = Some("unsupported".into());
    assert!(model.send(&bad).await.is_err());
    bad.subject = None;
    bad.conversation.as_mut().unwrap().key = "other/00000007/08/00000008".into();
    assert!(model.send(&bad).await.is_err());
    bad.conversation = Some(a.meshcore([5; 32]));
    bad.body = "é".repeat(86);
    assert!(model.send(&bad).await.is_err());
    assert!(owner.records().unwrap().is_empty());
    let id = send(&model, a.meshtastic(u32::MAX), "queued").await;
    let mut wrong = runtime(&scope(9, 3), 3, SENNET, 10);
    assert!(owner.attach(&mut wrong).is_err());
    let mut ar = runtime(&a, 3, SENNET, 10);
    let mut session = owner.attach(&mut ar).unwrap();
    session.dispatch_next(1, 1).unwrap();
    let tx = session.begin_tx(2).unwrap().unwrap();
    let mut stale = tx.id;
    stale.sequence += 1;
    assert!(session.complete_tx(3, stale, true).is_err());
    drop(session);
    drop(model);
    drop(owner);
    let reopened =
        RadioComms::open(a.clone(), FileJournal::new(dir.path().join("a.json"))).unwrap();
    assert!(matches!(
        reopened
            .model()
            .delivery_status(&a.meshtastic(u32::MAX), &id)
            .await
            .unwrap(),
        DeliveryStatus::Failed { .. }
    ));
    let mut fresh = runtime(&a, 3, SENNET, 74);
    assert!(
        reopened
            .attach(&mut fresh)
            .unwrap()
            .dispatch_next(4, 2)
            .unwrap()
            .is_none()
    );
    assert!(RadioComms::open(scope(99, 3), FileJournal::new(dir.path().join("a.json"))).is_err());
}
struct FailingJournal;

#[tokio::test]
async fn directed_meshtastic_matches_ack_and_refuses_history_key_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let sender = owner(a.clone(), dir.path(), "private-a.json");
    let receiver = owner(b.clone(), dir.path(), "private-b.json");
    let mut ar = runtime_configured(&a, 3, SENNET, 10, 0, Some((8, 4)));
    let mut br = runtime_configured(&b, 4, SENNET, 100, 0, Some((7, 3)));
    let id = send(&sender.model(), a.meshtastic(8), "private text").await;
    let mut ax = sender.attach(&mut ar).unwrap();
    let mut bx = receiver.attach(&mut br).unwrap();
    ax.dispatch_next(1, 1).unwrap();
    let tx = ax.begin_tx(2).unwrap().unwrap();
    bx.ingest(2, &tx.frame).unwrap();
    ax.complete_tx(3, tx.id, true).unwrap();
    assert_eq!(
        sender.records().unwrap()[0].delivery,
        DeliveryStatus::HandedToRadio
    );
    let ack = bx.begin_tx(4).unwrap().unwrap();
    bx.complete_tx(5, ack.id, true).unwrap();
    ax.ingest(6, &ack.frame).unwrap();
    assert_eq!(
        sender
            .model()
            .delivery_status(&a.meshtastic(8), &id)
            .await
            .unwrap(),
        DeliveryStatus::ProtocolAcknowledged
    );
    assert!(sender.records().unwrap()[0].facts.iter().any(|f| matches!(
        f,
        RadioFact::MeshtasticRouting {
            request_id: 10,
            code: 0,
            authenticated: false,
            ..
        }
    )));
    assert!(
        receiver.records().unwrap()[0]
            .facts
            .iter()
            .any(|f| matches!(f, RadioFact::MeshtasticPkiBinding { peer: 7, .. }))
    );
    drop(ax);
    drop(bx);
    let mut changed = runtime_configured(&a, 3, SENNET, 74, 0, Some((8, 55)));
    assert!(sender.attach(&mut changed).is_err());
    let before = serde_json::to_vec(&sender.records().unwrap()).unwrap();
    let mut fresh = runtime_configured(&a, 3, SENNET, 74, 0, Some((8, 4)));
    let mut reopened = sender.attach(&mut fresh).unwrap();
    assert!(reopened.dispatch_next(7, 2).unwrap().is_none());
    assert_eq!(
        before,
        serde_json::to_vec(&sender.records().unwrap()).unwrap()
    );
}

#[tokio::test]
async fn directed_ack_timeout_and_disconnect_preserve_physical_facts_without_replay() {
    for expire in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let a = scope(7, 3);
        let owner = owner(a.clone(), dir.path(), "timeout.json");
        let id = send(&owner.model(), a.meshtastic(8), "unconfirmed").await;
        let mut runtime = runtime_configured(&a, 3, SENNET, 10, 0, Some((8, 4)));
        let mut session = owner.attach(&mut runtime).unwrap();
        session.dispatch_next(1, 1).unwrap();
        let tx = session.begin_tx(2).unwrap().unwrap();
        session.complete_tx(3, tx.id, true).unwrap();
        if expire {
            session.poll(53).unwrap();
        }
        drop(session);
        assert_eq!(
            owner
                .model()
                .delivery_status(&a.meshtastic(8), &id)
                .await
                .unwrap(),
            DeliveryStatus::HandedToRadio
        );
        assert!(owner.records().unwrap()[0].facts.iter().any(|f| matches!(
            f,
            RadioFact::PhysicalCompletion {
                transmitted: true,
                ..
            }
        )));
        let mut fresh = runtime_configured(&a, 3, SENNET, 74, 0, Some((8, 4)));
        assert!(
            owner
                .attach(&mut fresh)
                .unwrap()
                .dispatch_next(54, 2)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn directed_without_pinned_key_is_refused_before_a_protocol_packet_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let owner = owner(a.clone(), dir.path(), "missing-peer.json");
    send(&owner.model(), a.meshtastic(8), "private intent").await;
    let mut runtime = runtime(&a, 3, SENNET, 10);
    let mut session = owner.attach(&mut runtime).unwrap();
    assert!(session.dispatch_next(1, 1).is_err());
    assert!(session.begin_tx(2).unwrap().is_none());
    drop(session);
    assert_eq!(runtime.sennet().packet_id_state().next_packet_id(), 10);
    assert!(matches!(
        owner.records().unwrap()[0].delivery,
        DeliveryStatus::Failed { .. }
    ));
}
impl RadioJournal for FailingJournal {
    fn load(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn commit(&mut self, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("disk refused"))
    }
}
#[test]
fn failing_durable_owner_cannot_publish_a_mailbox() {
    assert!(RadioComms::open(scope(7, 3), FailingJournal).is_err());
}

struct FailOnceJournal {
    commits: usize,
}
impl RadioJournal for FailOnceJournal {
    fn load(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn commit(&mut self, _: &[u8]) -> std::io::Result<()> {
        self.commits += 1;
        if self.commits == 2 {
            Err(std::io::Error::other("disk refused queued intent"))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn failed_intent_commit_never_publishes_or_dispatches_and_file_owner_is_exclusive() {
    let a = scope(7, 3);
    let mailbox = RadioComms::open(a.clone(), FailOnceJournal { commits: 0 }).unwrap();
    let mut draft = Draft::reply_to(a.meshtastic(8));
    draft.body = "must persist first".into();
    assert!(mailbox.model().send(&draft).await.is_err());
    assert!(mailbox.records().unwrap().is_empty());
    let mut ar = runtime(&a, 3, SENNET, 10);
    assert!(
        mailbox
            .attach(&mut ar)
            .unwrap()
            .dispatch_next(1, 1)
            .unwrap()
            .is_none()
    );
    assert_eq!(mailbox.model().send(&draft).await.unwrap().0, "out-1");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exclusive.json");
    let first = RadioComms::open(a.clone(), FileJournal::new(&path)).unwrap();
    assert!(RadioComms::open(a, FileJournal::new(&path)).is_err());
    drop(first);
    assert!(RadioComms::reopen(FileJournal::new(&path)).is_ok());
}
