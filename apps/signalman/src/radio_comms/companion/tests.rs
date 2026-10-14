use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
#[derive(Clone, Default)]
struct Journal {
    bytes: Arc<Mutex<Option<Vec<u8>>>>,
    fail: Arc<AtomicBool>,
}
impl RadioJournal for Journal {
    fn load(&mut self) -> io::Result<Option<Vec<u8>>> {
        Ok(self.bytes.lock().unwrap().clone())
    }
    fn commit(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(io::Error::other("injected journal failure"));
        }
        *self.bytes.lock().unwrap() = Some(bytes.to_vec());
        Ok(())
    }
}
// Independent peer framing, not the production encoder/deframer.
async fn read_command(peer: &mut DuplexStream, mc: bool) -> Vec<u8> {
    let length = if mc {
        assert_eq!(peer.read_u8().await.unwrap(), b'<');
        peer.read_u16_le().await.unwrap()
    } else {
        assert_eq!(peer.read_u16().await.unwrap(), 0x94c3);
        peer.read_u16().await.unwrap()
    } as usize;
    let mut body = vec![0; length];
    peer.read_exact(&mut body).await.unwrap();
    body
}
fn wire(payload: &[u8], mc: bool) -> Vec<u8> {
    let mut bytes = if mc {
        let mut bytes = vec![b'>'];
        bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        bytes
    } else {
        let mut bytes = vec![0x94, 0xc3];
        bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        bytes
    };
    bytes.extend_from_slice(payload);
    bytes
}
async fn emit(peer: &mut DuplexStream, payload: &[u8], mc: bool) {
    peer.write_all(&wire(payload, mc)).await.unwrap();
}
fn var_field(n: u32, value: u32) -> Vec<u8> {
    let mut out = vec![];
    meshtastic::var(&mut out, n, value);
    out
}
fn message_field(n: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    meshtastic::bytes(&mut out, n, data);
    out
}
async fn mt_handshake(peer: &mut DuplexStream, node: u8, channel: bool) {
    let command = read_command(peer, false).await;
    assert_eq!(command[0], 0x18);
    let nonce = uint(&command, 3).unwrap();
    let mut batch = wire(&[0x1a, 2, 8, node], false);
    if channel {
        batch.extend_from_slice(&wire(&[0x52, 2, 0x18, 1], false));
    }
    // Wrong nonce cannot finish the configuration transaction.
    batch.extend_from_slice(&wire(&var_field(7, nonce ^ 1), false));
    batch.extend_from_slice(&wire(&var_field(7, nonce), false));
    peer.write_all(&batch).await.unwrap();
}
async fn mt(journal: Journal) -> (CompanionSession<DuplexStream>, DuplexStream) {
    let (host, mut peer) = tokio::io::duplex(4096);
    let (session, ()) = tokio::join!(
        CompanionSession::connect(
            host,
            CompanionProtocol::Meshtastic { channel: 0 },
            "stock".into(),
            journal
        ),
        mt_handshake(&mut peer, 1, true)
    );
    (session.unwrap(), peer)
}
async fn contacts(peer: &mut DuplexStream, keys: &[[u8; 32]]) {
    assert_eq!(read_command(peer, true).await, [4]);
    let mut start = vec![2];
    start.extend_from_slice(&(keys.len() as u32).to_le_bytes());
    emit(peer, &start, true).await;
    for key in keys {
        let mut record = vec![0; 148];
        record[0] = 3;
        record[1..33].copy_from_slice(key);
        record[33] = 1;
        record[35] = 0xff;
        emit(peer, &record, true).await;
    }
    emit(peer, &[4, 1, 0, 0, 0], true).await;
}
async fn mc_handshake(peer: &mut DuplexStream, keys: &[[u8; 32]]) {
    let command = read_command(peer, true).await;
    assert_eq!(&command[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
    let mut info = vec![0; 58];
    info[0] = 5;
    info[4..36].fill(7);
    emit(peer, &info, true).await;
    assert_eq!(read_command(peer, true).await, [22, 3]);
    let mut info = vec![0; 80];
    info[0] = 13;
    info[1] = 3;
    info[60..67].copy_from_slice(b"v1.17.1");
    emit(peer, &info, true).await;
    contacts(peer, keys).await;
}
async fn mc(keys: &[[u8; 32]], journal: Journal) -> (CompanionSession<DuplexStream>, DuplexStream) {
    let (host, mut peer) = tokio::io::duplex(4096);
    let (session, ()) = tokio::join!(
        CompanionSession::connect(host, CompanionProtocol::MeshCore, "stock".into(), journal),
        mc_handshake(&mut peer, keys)
    );
    (session.unwrap(), peer)
}
async fn queue(session: &CompanionSession<DuplexStream>, peer: &str, text: &str) -> MessageId {
    let mut draft = Draft::reply_to(
        session
            .owner
            .conversation(session.protocol.kind(), peer)
            .unwrap(),
    );
    draft.body = text.into();
    session.owner.model().send(&draft).await.unwrap()
}
fn delivery(owner: &RadioComms, id: &MessageId) -> DeliveryStatus {
    owner
        .records()
        .unwrap()
        .into_iter()
        .find(|r| r.message.id == *id)
        .unwrap()
        .delivery
}
fn packet(source: u32, to: u32, id: u32, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0x0d];
    p.extend_from_slice(&source.to_le_bytes());
    p.push(0x15);
    p.extend_from_slice(&to.to_le_bytes());
    p.extend_from_slice(&message_field(4, data));
    p.push(0x35);
    p.extend_from_slice(&id.to_le_bytes());
    message_field(2, &p)
}
fn routing(source: u32, request: u32, error: u32) -> Vec<u8> {
    let mut data = vec![8, 5];
    data.extend_from_slice(&message_field(2, &var_field(3, error)));
    data.push(0x35);
    data.extend_from_slice(&request.to_le_bytes());
    packet(source, 1, 91, &data)
}
#[tokio::test]
async fn meshtastic_queues_real_packet_and_only_matched_remote_ack_advances_delivery() {
    let (mut session, mut peer) = mt(Journal::default()).await;
    let owner = session.owner();
    assert_eq!(owner.model().identities().len(), 1);
    assert!(!owner.supports(ProtocolKind::MeshCore));
    let id = queue(&session, "00000002", "hello stock").await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::Unknown);
    let sent = read_command(&mut peer, false).await;
    let p = nested(&sent, 1).unwrap().unwrap();
    assert_eq!(uint(p, 2).unwrap(), 2);
    assert_eq!(uint(p, 10).unwrap(), 1);
    assert_eq!(uint(p, 9).unwrap(), 3);
    assert_eq!(
        nested(nested(p, 4).unwrap().unwrap(), 2).unwrap().unwrap(),
        b"hello stock"
    );
    let correlation = uint(p, 6).unwrap();
    assert_ne!(correlation, 0);
    let mut q = var_field(4, correlation);
    q.extend_from_slice(&var_field(1, 0));
    emit(&mut peer, &message_field(11, &q), false).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::HandedToRadio);
    emit(&mut peer, &routing(1, correlation, 0), false).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::HandedToRadio);
    emit(&mut peer, &routing(3, correlation, 0), false).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::HandedToRadio);
    emit(&mut peer, &routing(2, correlation, 0), false).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::ProtocolAcknowledged);
}
#[tokio::test]
async fn meshtastic_fragmented_receive_survives_poll_timeout_and_deduplicates() {
    let (mut session, mut peer) = mt(Journal::default()).await;
    let frame = packet(2, 1, 99, &[8, 1, 0x12, 2, b'h', b'i']);
    let bytes = wire(&frame, false);
    peer.write_all(&bytes[..6]).await.unwrap();
    assert!(!session.poll().await.unwrap());
    peer.write_all(&bytes[6..]).await.unwrap();
    session.poll().await.unwrap();
    emit(&mut peer, &frame, false).await;
    session.poll().await.unwrap();
    let records = session.owner.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message.body.text(), "hi");
    assert_eq!(records[0].delivery, DeliveryStatus::ReceivedByRadio);
}
#[tokio::test]
async fn meshtastic_disabled_channel_or_changed_identity_refuses_connection() {
    let (host, mut peer) = tokio::io::duplex(4096);
    let (result, ()) = tokio::join!(
        CompanionSession::connect(
            host,
            CompanionProtocol::Meshtastic { channel: 0 },
            "stock".into(),
            Journal::default()
        ),
        mt_handshake(&mut peer, 1, false)
    );
    assert!(result.is_err());
    let journal = Journal::default();
    let (session, peer) = mt(journal.clone()).await;
    drop(session);
    drop(peer);
    let (host, mut peer) = tokio::io::duplex(4096);
    let (result, ()) = tokio::join!(
        CompanionSession::connect(
            host,
            CompanionProtocol::Meshtastic { channel: 0 },
            "stock".into(),
            journal
        ),
        mt_handshake(&mut peer, 2, true)
    );
    assert!(result.is_err());
}
#[tokio::test]
async fn meshtastic_disconnect_marks_uncertain_command_and_does_not_replay_on_reopen() {
    let journal = Journal::default();
    let (mut session, peer) = mt(journal.clone()).await;
    let owner = session.owner();
    let id = queue(&session, "00000002", "once").await;
    session.poll().await.unwrap();
    drop(peer);
    assert!(session.poll().await.is_err());
    drop(session);
    assert!(matches!(
        delivery(&owner, &id),
        DeliveryStatus::Failed { .. }
    ));
    let (mut session, mut peer) = mt(journal).await;
    assert!(!session.poll().await.unwrap());
    assert!(
        timeout(Duration::from_millis(20), read_command(&mut peer, false))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn journal_failure_prevents_serial_dispatch_and_retires_owner() {
    let journal = Journal::default();
    let (mut session, mut peer) = mt(journal.clone()).await;
    queue(&session, "00000002", "stay local").await;
    journal.fail.store(true, Ordering::Relaxed);
    assert!(session.poll().await.is_err());
    assert!(
        timeout(Duration::from_millis(20), read_command(&mut peer, false))
            .await
            .is_err()
    );
    assert!(
        session
            .owner
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.contains("write failed"))
    );
}
#[tokio::test]
async fn meshcore_send_and_sync_use_stock_commands_and_matching_ack() {
    let key = [8; 32];
    let (mut session, mut peer) = mc(&[key], Journal::default()).await;
    session.meshcore_sync = false;
    let owner = session.owner();
    let id = queue(&session, &hex::encode(key), "hello meshcore").await;
    let (poll, ()) = tokio::join!(session.poll(), async {
        contacts(&mut peer, &[key]).await;
        let command = read_command(&mut peer, true).await;
        assert_eq!(&command[..3], &[2, 0, 0]);
        assert_eq!(&command[7..13], &key[..6]);
        assert_eq!(&command[13..], b"hello meshcore");
        assert_eq!(delivery(&owner, &id), DeliveryStatus::Unknown);
        emit(
            &mut peer,
            &[6, 1, 0x11, 0x22, 0x33, 0x44, 0x10, 0x27, 0, 0],
            true,
        )
        .await;
    });
    poll.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::HandedToRadio);
    emit(&mut peer, &[0x82, 1, 0, 0, 0, 0, 0, 0, 0], true).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::HandedToRadio);
    emit(&mut peer, &[0x82, 0x11, 0x22, 0x33, 0x44, 1, 0, 0, 0], true).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&owner, &id), DeliveryStatus::ProtocolAcknowledged);
    emit(&mut peer, &[0x83], true).await;
    let (poll, ()) = tokio::join!(session.poll(), async {
        assert_eq!(read_command(&mut peer, true).await, [10]);
        let mut message = vec![16, 0, 0, 0];
        message.extend_from_slice(&key[..6]);
        message.extend_from_slice(&[0xff, 0, 99, 0, 0, 0]);
        message.extend_from_slice(b"inbound");
        emit(&mut peer, &message, true).await;
    });
    poll.unwrap();
    let (poll, ()) = tokio::join!(session.poll(), async {
        assert_eq!(read_command(&mut peer, true).await, [10]);
        emit(&mut peer, &[10], true).await;
    });
    poll.unwrap();
    let records = owner.records().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].message.author.address, hex::encode(key));
    assert_eq!(records[1].message.body.text(), "inbound");
}
#[tokio::test]
async fn meshcore_early_ack_before_sent_response_is_retained() {
    let key = [8; 32];
    let (mut session, mut peer) = mc(&[key], Journal::default()).await;
    session.meshcore_sync = false;
    let id = queue(&session, &hex::encode(key), "fast ack").await;
    let (poll, ()) = tokio::join!(session.poll(), async {
        contacts(&mut peer, &[key]).await;
        read_command(&mut peer, true).await;
        emit(&mut peer, &[0x82, 1, 2, 3, 4, 1, 0, 0, 0], true).await;
        emit(&mut peer, &[6, 0, 1, 2, 3, 4, 1, 0, 0, 0], true).await;
    });
    poll.unwrap();
    assert_eq!(
        delivery(&session.owner, &id),
        DeliveryStatus::ProtocolAcknowledged
    );
}
#[tokio::test]
async fn meshcore_prefix_collision_refuses_send_before_touching_radio() {
    let key = [8; 32];
    let mut collision = key;
    collision[31] = 9;
    let keys = [key, collision];
    let (mut session, mut peer) = mc(&keys, Journal::default()).await;
    session.meshcore_sync = false;
    let id = queue(&session, &hex::encode(key), "ambiguous").await;
    let (poll, ()) = tokio::join!(session.poll(), contacts(&mut peer, &keys));
    poll.unwrap();
    assert!(matches!(
        delivery(&session.owner, &id),
        DeliveryStatus::Failed { .. }
    ));
    assert!(
        timeout(Duration::from_millis(20), read_command(&mut peer, true))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_lengths_and_truncated_protobuf_stop_the_connection() {
    for mc in [false, true] {
        let (host, mut peer) = tokio::io::duplex(32);
        let mut frames = Frames::new(host, mc);
        let bytes = if mc {
            vec![b'>', 0xff, 0xff]
        } else {
            vec![0x94, 0xc3, 0xff, 0xff]
        };
        peer.write_all(&bytes).await.unwrap();
        assert!(frames.next().await.is_err());
    }
    let (mut session, mut peer) = mt(Journal::default()).await;
    emit(&mut peer, &[0x12, 255], false).await;
    assert!(session.poll().await.is_err());
}
#[tokio::test]
async fn bound_pending_commands_leave_excess_intents_durable_and_offline() {
    let (mut session, mut peer) = mt(Journal::default()).await;
    for _ in 0..17 {
        queue(&session, "00000002", "bounded").await;
    }
    for _ in 0..16 {
        session.poll().await.unwrap();
        read_command(&mut peer, false).await;
    }
    assert!(!session.poll().await.unwrap());
    assert_eq!(session.pending.len(), 16);
    assert_eq!(
        session
            .owner
            .records()
            .unwrap()
            .iter()
            .filter(|r| r.delivery == DeliveryStatus::Queued(DeliveryQueueReason::Offline))
            .count(),
        1
    );
}

#[tokio::test]
async fn meshtastic_queue_rejection_and_routing_requests_are_not_acknowledgements() {
    let (mut session, mut peer) = mt(Journal::default()).await;
    let id = queue(&session, "00000002", "rejection").await;
    session.poll().await.unwrap();
    let command = read_command(&mut peer, false).await;
    let correlation = uint(nested(&command, 1).unwrap().unwrap(), 6).unwrap();
    let mut data = vec![8, 5];
    data.extend_from_slice(&message_field(2, &[0x0a, 0]));
    data.push(0x35);
    data.extend_from_slice(&correlation.to_le_bytes());
    emit(&mut peer, &packet(2, 1, 95, &data), false).await;
    session.poll().await.unwrap();
    assert_eq!(delivery(&session.owner, &id), DeliveryStatus::Unknown);
    let mut queue = var_field(4, correlation);
    queue.push(8);
    sennet::protobuf::write_varint(u64::MAX, &mut queue);
    emit(&mut peer, &message_field(11, &queue), false).await;
    session.poll().await.unwrap();
    assert!(matches!(
        delivery(&session.owner, &id),
        DeliveryStatus::Failed { .. }
    ));
    assert!(session.pending.is_empty());
}
#[tokio::test]
async fn meshcore_interrupted_sent_response_never_replays_after_reconnect() {
    let journal = Journal::default();
    let key = [8; 32];
    let (mut session, mut peer) = mc(&[key], journal.clone()).await;
    session.meshcore_sync = false;
    let id = queue(&session, &hex::encode(key), "one command").await;
    let (poll, ()) = tokio::join!(session.poll(), async {
        contacts(&mut peer, &[key]).await;
        read_command(&mut peer, true).await;
        drop(peer);
    });
    assert!(poll.is_err());
    drop(session);
    let (mut session, mut peer) = mc(&[key], journal).await;
    session.meshcore_sync = false;
    assert!(matches!(
        delivery(&session.owner, &id),
        DeliveryStatus::Failed { .. }
    ));
    assert!(!session.poll().await.unwrap());
    assert!(
        timeout(Duration::from_millis(20), read_command(&mut peer, true))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn meshcore_unknown_or_ambiguous_receive_is_not_given_a_synthetic_full_identity() {
    let keys = [[8; 32], [9; 32]];
    let (mut session, mut peer) = mc(&keys, Journal::default()).await;
    session.meshcore_sync = false;
    let mut message = vec![16, 0, 0, 0];
    message.extend_from_slice(&[11; 6]);
    message.extend_from_slice(&[0xff, 0, 1, 0, 0, 0]);
    message.extend_from_slice(b"unknown peer");
    emit(&mut peer, &message, true).await;
    session.poll().await.unwrap();
    assert!(session.owner.records().unwrap().is_empty());
    assert!(
        session
            .owner
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.contains("unknown or ambiguous"))
    );
}
#[tokio::test]
async fn acknowledgement_expiry_preserves_observed_device_acceptance() {
    let (mut session, mut peer) = mt(Journal::default()).await;
    let id = queue(&session, "00000002", "accepted").await;
    session.poll().await.unwrap();
    let command = read_command(&mut peer, false).await;
    let correlation = uint(nested(&command, 1).unwrap().unwrap(), 6).unwrap();
    emit(
        &mut peer,
        &message_field(11, &var_field(4, correlation)),
        false,
    )
    .await;
    session.poll().await.unwrap();
    session.pending.get_mut(&correlation).unwrap().deadline = Instant::now();
    session.poll().await.unwrap();
    assert_eq!(delivery(&session.owner, &id), DeliveryStatus::HandedToRadio);
    assert!(
        session
            .owner
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.contains("no recipient acknowledgement"))
    );
}
#[tokio::test]
async fn meshtastic_packets_during_configuration_are_persisted_after_identity_validation() {
    let (host, mut peer) = tokio::io::duplex(4096);
    let (session, ()) = tokio::join!(
        CompanionSession::connect(
            host,
            CompanionProtocol::Meshtastic { channel: 0 },
            "stock".into(),
            Journal::default()
        ),
        async {
            let command = read_command(&mut peer, false).await;
            let nonce = uint(&command, 3).unwrap();
            emit(
                &mut peer,
                &packet(2, 1, 77, &[8, 1, 0x12, 2, b'h', b'i']),
                false,
            )
            .await;
            emit(&mut peer, &[0x1a, 2, 8, 1], false).await;
            emit(&mut peer, &[0x52, 2, 0x18, 1], false).await;
            emit(&mut peer, &var_field(7, nonce), false).await;
        }
    );
    assert_eq!(session.unwrap().owner.records().unwrap().len(), 1);
}
