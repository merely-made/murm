use super::*;
use crate::comms_adapter::comms::{DeliveryStatus, Draft};
use crate::radio_comms::{
    FileJournal, RadioComms, RadioFact,
    tests::{owner, runtime, scope},
};
use radio_hand::instances::TUCKET;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tucket::{identity::LocalIdentity, node::Node};
use tulle::{link::Received, serial::TransmitError};

struct Radio {
    remote: Arc<Mutex<Node>>,
    emitted: Arc<Mutex<Vec<Vec<u8>>>>,
    inbound: mpsc::Receiver<Received>,
    return_path: mpsc::Sender<Received>,
    fail_send: bool,
    seen_acks: Arc<Mutex<Vec<[u8; 4]>>>,
}
impl PacketRadio for Radio {
    fn max_frame_len(&self) -> usize {
        255
    }
    async fn send_frame(&self, frame: Vec<u8>) -> Result<Duration, TransmitError> {
        self.emitted.lock().unwrap().push(frame.clone());
        let (events, responses) = self.remote.lock().unwrap().on_frame(&frame);
        for event in events {
            if let tucket::node::Event::Ack(ack) = event {
                self.seen_acks.lock().unwrap().push(ack);
            }
        }
        for frame in responses {
            self.return_path
                .try_send(Received {
                    frame,
                    rssi_dbm: -70,
                    snr_db: 8.0,
                })
                .unwrap();
        }
        if self.fail_send {
            Err(TransmitError::Transport(
                "completion lost after send".into(),
            ))
        } else {
            Ok(Duration::from_millis(1))
        }
    }
    async fn recv_frame(&mut self) -> Option<Received> {
        self.inbound.recv().await
    }
}
fn radio(fail_send: bool) -> Radio {
    let mut remote = Node::new(LocalIdentity::from_seed([4; 32]), false);
    let (tx, rx) = mpsc::channel(32);
    tx.try_send(Received {
        frame: remote.advert_frame(
            unix_seconds().unwrap(),
            &tucket::advert::AdvertData::chat("peer").encode().unwrap(),
        ),
        rssi_dbm: -70,
        snr_db: 8.0,
    })
    .unwrap();
    Radio {
        remote: Arc::new(Mutex::new(remote)),
        emitted: Default::default(),
        inbound: rx,
        return_path: tx,
        fail_send,
        seen_acks: Default::default(),
    }
}

#[tokio::test]
async fn custody_lost_during_receive_stops_before_ingress_or_transmit() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(7, 3);
    let owner = owner(scope.clone(), dir.path(), "custody.json");
    let mut runtime = runtime(&scope, 3, TUCKET, 10);
    let mut io = radio(false);
    let emitted = io.emitted.clone();
    let calls = std::cell::Cell::new(0);
    let mut carriage = ResidentCarriage::new(
        owner.attach(&mut runtime).unwrap(),
        &mut io,
        Instant::now(),
        0,
        Duration::from_millis(10),
    )
    .unwrap();
    let result = carriage
        .poll_authorized(|| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Ok(())
            } else {
                Err(backend("custody locked during receive"))
            }
        })
        .await;
    assert!(result.is_err());
    assert!(
        carriage.poll().await.is_err(),
        "custody loss permanently stops this carriage"
    );
    assert!(emitted.lock().unwrap().is_empty());
    let remote_key = tucket::identity::LocalIdentity::from_seed([4; 32])
        .identity()
        .pub_key;
    assert!(!carriage.knows_peer(&remote_key));
    assert!(owner.records().unwrap().is_empty());
}
#[tokio::test]
async fn physical_driver_carries_real_text_ack_and_full_key_contact() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let owner = owner(a.clone(), dir.path(), "radio.json");
    let mut rt = runtime(&a, 3, TUCKET, 10);
    let mut io = radio(false);
    let remote = io.remote.clone();
    let emitted = io.emitted.clone();
    let mut carriage = ResidentCarriage::new(
        owner.attach(&mut rt).unwrap(),
        &mut io,
        Instant::now(),
        0,
        Duration::from_millis(1),
    )
    .unwrap();
    carriage.advertise("Signalman").unwrap();
    // Establish the full-key contact before persisting an addressed intent.
    carriage.poll().await.unwrap();
    assert!(carriage.knows_peer(&b.meshcore_key));
    let mut draft = Draft::reply_to(a.meshcore(b.meshcore_key));
    draft.body = "physical driver".into();
    let id = owner.model().send(&draft).await.unwrap();
    for _ in 0..12 {
        carriage.poll().await.unwrap();
        if owner.records().unwrap()[0].delivery == DeliveryStatus::ProtocolAcknowledged {
            break;
        }
    }
    let record = &owner.records().unwrap()[0];
    assert_eq!(record.message.id, id);
    assert_eq!(record.delivery, DeliveryStatus::ProtocolAcknowledged);
    assert!(record.facts.iter().any(|f| matches!(
        f,
        RadioFact::PhysicalCompletion {
            transmitted: true,
            ..
        }
    )));
    assert!(remote.lock().unwrap().contact(a.meshcore_key[0]).is_some());
    assert!(!emitted.lock().unwrap().is_empty());
    let (return_frame, return_ack) = remote
        .lock()
        .unwrap()
        .text_frame(a.meshcore_key[0], unix_seconds().unwrap(), "driver return")
        .unwrap();
    // Inject only actual wire bytes produced by the peer protocol.
    carriage
        .radio
        .return_path
        .try_send(Received {
            frame: return_frame,
            rssi_dbm: -70,
            snr_db: 8.0,
        })
        .unwrap();
    carriage.poll().await.unwrap();
    assert!(
        carriage
            .radio
            .seen_acks
            .lock()
            .unwrap()
            .contains(&return_ack),
        "the direct return requires an actual wire ACK"
    );
    let records = owner.records().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].message.body.text(), "driver return");
    assert_eq!(
        records[1].message.author.address,
        hex::encode(b.meshcore_key)
    );
    assert_eq!(records[1].delivery, DeliveryStatus::ReceivedByRadio);
    drop(carriage);
    drop(owner);
    let reopened = RadioComms::open(a, FileJournal::new(dir.path().join("radio.json"))).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.records().unwrap()).unwrap(),
        serde_json::to_value(records).unwrap()
    );
}

#[tokio::test]
async fn undiscovered_recipient_waits_without_blocking_verified_contacts() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let owner = owner(a.clone(), dir.path(), "radio.json");
    let mut unknown = Draft::reply_to(a.meshcore(scope(9, 5).meshcore_key));
    unknown.body = "await verified discovery".into();
    owner.model().send(&unknown).await.unwrap();
    let mut known = Draft::reply_to(a.meshcore(b.meshcore_key));
    known.body = "reachable peer".into();
    let known_id = owner.model().send(&known).await.unwrap();
    let mut rt = runtime(&a, 3, TUCKET, 10);
    let mut io = radio(false);
    let mut carriage = ResidentCarriage::new(
        owner.attach(&mut rt).unwrap(),
        &mut io,
        Instant::now(),
        0,
        Duration::from_millis(1),
    )
    .unwrap();
    carriage.advertise("Signalman").unwrap();
    for _ in 0..12 {
        carriage.poll().await.unwrap();
        if owner
            .records()
            .unwrap()
            .iter()
            .any(|r| r.message.id == known_id && r.delivery == DeliveryStatus::ProtocolAcknowledged)
        {
            break;
        }
    }
    let records = owner.records().unwrap();
    assert_eq!(
        records[0].delivery,
        DeliveryStatus::Queued(crate::comms_adapter::comms::DeliveryQueueReason::Offline)
    );
    assert!(records[0].facts.is_empty());
    assert_eq!(records[1].message.id, known_id);
    assert_eq!(records[1].delivery, DeliveryStatus::ProtocolAcknowledged);
}
#[tokio::test]
async fn ambiguous_physical_error_stops_without_false_completion_or_replay() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let owner = owner(a.clone(), dir.path(), "radio.json");
    let mut rt = runtime(&a, 3, TUCKET, 10);
    let mut io = radio(false);
    // Preload the verified peer and empty its advert-forwarding queue.
    let peer_advert = io.inbound.recv().await.unwrap();
    let mut session = owner.attach(&mut rt).unwrap();
    session.ingest(0, &peer_advert.frame).unwrap();
    while let Some(tx) = session.begin_tx(0).unwrap() {
        session.complete_tx(0, tx.id, true).unwrap();
    }
    io.fail_send = true;
    let emitted = io.emitted.clone();
    let mut carriage = ResidentCarriage::new(
        session,
        &mut io,
        Instant::now(),
        0,
        Duration::from_millis(1),
    )
    .unwrap();
    let mut draft = Draft::reply_to(a.meshcore(b.meshcore_key));
    draft.body = "uncertain physical send".into();
    owner.model().send(&draft).await.unwrap();
    assert!(
        carriage
            .poll()
            .await
            .unwrap_err()
            .to_string()
            .contains("outcome uncertain")
    );
    assert_eq!(emitted.lock().unwrap().len(), 1);
    assert!(carriage.poll().await.is_err());
    assert_eq!(emitted.lock().unwrap().len(), 1);
    drop(carriage);
    let record = &owner.records().unwrap()[0];
    assert!(
        !record
            .facts
            .iter()
            .any(|f| matches!(f, RadioFact::PhysicalCompletion { .. }))
    );
    assert!(
        matches!(&record.delivery,DeliveryStatus::Failed{detail} if detail.contains("unknown"))
    );
}

#[tokio::test]
async fn protocol_expiry_wakes_history_without_radio_traffic() {
    let dir = tempfile::tempdir().unwrap();
    let a = scope(7, 3);
    let b = scope(8, 4);
    let owner = owner(a.clone(), dir.path(), "radio.json");
    let mut rt = runtime(&a, 3, TUCKET, 10);
    let mut io = radio(false);
    let mut session = owner.attach(&mut rt).unwrap();
    let advert = io.inbound.recv().await.unwrap();
    session.ingest(0, &advert.frame).unwrap();
    while let Some(tx) = session.begin_tx(0).unwrap() {
        session.complete_tx(0, tx.id, true).unwrap();
    }
    let mut draft = Draft::reply_to(a.meshcore(b.meshcore_key));
    draft.body = "expires before physical carriage".into();
    owner.model().send(&draft).await.unwrap();
    session.dispatch_next(0, 1).unwrap();
    session.poll(0).unwrap();
    let mut carriage = ResidentCarriage::new(
        session,
        &mut io,
        Instant::now(),
        31_000,
        Duration::from_millis(1),
    )
    .unwrap();
    assert!(carriage.poll().await.unwrap());
    assert!(matches!(
        owner.records().unwrap()[0].delivery,
        DeliveryStatus::Failed { .. }
    ));
    assert!(carriage.radio.emitted.lock().unwrap().is_empty());
}
