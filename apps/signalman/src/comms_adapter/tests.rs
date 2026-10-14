use super::*;
use crate::message::{MessageBook, MessageEvent, QueuedReason, TextMessage};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Mailbox(Arc<Mutex<MessageBook>>);
impl LxmfMailbox for Mailbox {
    fn book(&self) -> Result<Arc<MessageBook>, AdapterError> {
        Ok(Arc::new(self.0.lock().unwrap().clone()))
    }
    fn queue_text(
        &self,
        local: MessagePeer,
        peer: MessagePeer,
        text: &str,
    ) -> Result<crate::message::MessageId, AdapterError> {
        let mut book = self.0.lock().unwrap();
        let message = TextMessage::compose(local, peer, 100, [book.len() as u8; 32], text);
        let id = message.id;
        book.apply(&MessageEvent::OutgoingQueued {
            message: message.into(),
            reason: QueuedReason::Offline,
            observed_unix_ms: 101,
        })
        .unwrap();
        Ok(id)
    }
}
fn peer(n: u8) -> MessagePeer {
    MessagePeer::new([n; 16], Some([n; 32]))
}
fn draft(local: MessagePeer, remote: MessagePeer) -> Draft {
    let mut draft = Draft::reply_to(LxmfAdapter::<Mailbox>::conversation_id(local, remote));
    draft.body = "hello".into();
    draft
}

#[tokio::test]
async fn router_queues_then_reads_the_same_owner_and_exact_delivery_fact() {
    let mailbox = Mailbox::default();
    let adapter = LxmfAdapter::new(peer(1), mailbox.clone());
    let model = comms::Comms::new().with_adapter(Box::new(adapter));
    let draft = draft(peer(1), peer(2));
    let id = model.send(&draft).await.unwrap();
    let conversation = draft.conversation.as_ref().unwrap();
    let messages = model.messages(conversation).await.unwrap();
    assert_eq!(messages[0].id, id);
    assert_eq!(messages[0].body.text(), "hello");
    assert_eq!(
        model.delivery_status(conversation, &id).await.unwrap(),
        DeliveryStatus::Queued(comms::DeliveryQueueReason::Offline)
    );
    let internal = mailbox
        .0
        .lock()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .message
        .id();
    mailbox
        .0
        .lock()
        .unwrap()
        .apply(&MessageEvent::StatusChanged {
            id: internal,
            status: crate::message::MessageStatus::HandedToRadio {
                transport_id: [8; 32],
                mode: crate::message::MessageTransport::Data,
            },
            observed_unix_ms: 102,
        })
        .unwrap();
    assert_eq!(
        model.delivery_status(conversation, &id).await.unwrap(),
        DeliveryStatus::HandedToRadio
    );
    assert_eq!(model.inbox().await.conversations.len(), 1);
}

#[tokio::test]
async fn unsupported_or_wrong_station_drafts_refuse_before_queue_mutation() {
    let mailbox = Mailbox::default();
    let adapter = LxmfAdapter::new(peer(1), mailbox.clone());
    let mut unsupported = draft(peer(1), peer(2));
    unsupported.subject = Some("subject".into());
    assert!(matches!(
        adapter.send(&unsupported).await,
        Err(AdapterError::Unsupported(_))
    ));
    assert_eq!(
        adapter.send(&draft(peer(3), peer(2))).await,
        Err(AdapterError::NotFound)
    );
    let mut malformed = draft(peer(1), peer(2));
    malformed.conversation.as_mut().unwrap().key.push('0');
    assert_eq!(adapter.send(&malformed).await, Err(AdapterError::NotFound));
    let mut empty = draft(peer(1), peer(2));
    empty.body = " \n".into();
    assert!(matches!(
        adapter.send(&empty).await,
        Err(AdapterError::Unsupported(_))
    ));
    let absent_identity = MessagePeer::new([1; 16], None);
    assert!(matches!(
        LxmfAdapter::new(absent_identity, mailbox.clone())
            .send(&draft(absent_identity, peer(2)))
            .await,
        Err(AdapterError::Unsupported(_))
    ));
    assert!(mailbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn another_station_history_cannot_be_read_or_misrouted() {
    let mailbox = Mailbox::default();
    mailbox.queue_text(peer(3), peer(2), "other owner").unwrap();
    let adapter = LxmfAdapter::new(peer(1), mailbox);
    assert!(adapter.conversations().await.unwrap().is_empty());
    assert_eq!(
        adapter
            .messages(&LxmfAdapter::<Mailbox>::conversation_id(peer(1), peer(2)))
            .await,
        Err(AdapterError::NotFound)
    );
}

#[test]
fn diagnostics_preserve_failure_detail_and_scope_without_inventing_delivery() {
    let mailbox = Mailbox::default();
    let id = mailbox.queue_text(peer(1), peer(2), "mine").unwrap();
    mailbox
        .queue_text(peer(3), peer(2), "another station")
        .unwrap();
    let adapter = LxmfAdapter::new(peer(1), mailbox.clone());
    let before = adapter.diagnostics().unwrap();
    assert_eq!(before.queued_offline, 1);
    assert_eq!(before.handed_to_radio, 0);
    mailbox
        .0
        .lock()
        .unwrap()
        .apply(&MessageEvent::StatusChanged {
            id,
            status: crate::message::MessageStatus::Failed(
                "lease expired before a carriage outcome".into(),
            ),
            observed_unix_ms: 105,
        })
        .unwrap();
    let after = adapter.diagnostics().unwrap();
    assert_eq!(after.queued_offline, 0);
    assert_eq!(after.failures.len(), 1);
    assert_eq!(after.failures[0].message_id, id.0);
    assert_eq!(
        after.failures[0].detail,
        "lease expired before a carriage outcome"
    );
    assert_eq!(after.last_observed_unix_ms, Some(105));
    assert_eq!(after.received_direct, 0);
    assert_eq!(
        before.queued_offline, 1,
        "the prior report stays a snapshot"
    );
}
