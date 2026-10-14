//! Bounded, acknowledged handoff from the application journal to its station.
use super::*;
use signalman::message::{Message, MessageEvent, MessageId, MessageStatus, QueuedReason};
use std::collections::HashSet;
const CAPACITY: usize = 16;

#[derive(Clone)]
pub struct StationSender {
    commands: tokio::sync::mpsc::Sender<Message>,
    pending: Arc<std::sync::Mutex<HashSet<MessageId>>>,
}
impl StationSender {
    pub fn pending_count(&self) -> Result<usize, String> {
        self.pending
            .lock()
            .map(|pending| pending.len())
            .map_err(|_| "station handoff unavailable".into())
    }
    /// Return whether this persisted intent was newly accepted by the actor.
    /// Capacity or a stopped actor refuses without losing the journal intent.
    pub fn submit(&self, message: Message) -> Result<bool, String> {
        let id = message.id();
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "station handoff unavailable")?;
        if pending.contains(&id) {
            return Ok(false);
        }
        if pending.len() >= CAPACITY {
            return Err("station handoff is full; message remains queued".into());
        }
        self.commands
            .try_send(message)
            .map_err(|error| format!("station handoff unavailable: {error}"))?;
        pending.insert(id);
        Ok(true)
    }
    /// Release capacity only after the application persisted a terminal attempt
    /// observation. A failed journal write retains the reservation.
    pub fn acknowledge(&self, id: MessageId) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
    }
}
pub(super) fn channel() -> (StationSender, tokio::sync::mpsc::Receiver<Message>) {
    let (commands, receiver) = tokio::sync::mpsc::channel(CAPACITY);
    (
        StationSender {
            commands,
            pending: Arc::new(std::sync::Mutex::new(HashSet::new())),
        },
        receiver,
    )
}

pub(super) async fn stopped(stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) async fn send(
    station: &SitedStation,
    message: Message,
    local: signalman::message::MessagePeer,
    events: &Sender<StationEvent>,
    wake: &LayoutWake,
    stop: &AtomicBool,
) {
    let id = message.id();
    let observed = || unix_time_ms().unwrap_or(0);
    let status = if message.sender() != local {
        MessageStatus::Failed("queued message belongs to a different station identity".into())
    } else {
        let _ = events.send(StationEvent::Message {
            event: MessageEvent::StatusChanged {
                id,
                status: MessageStatus::Queued(QueuedReason::ReadyForCarriage),
                observed_unix_ms: observed(),
            },
            complete: false,
        });
        wake();
        match message.encode_payload(observed() as f64 / 1000.0) {
            Err(error) => MessageStatus::Failed(format!("message payload refused: {error}")),
            Ok(payload) => {
                let prefix = hex::encode(message.recipient().destination);
                tokio::select! {
                    result=station.send_payload(&prefix,&payload,signalman::SEND_PATIENCE)=>match result{
                        Ok(sent)=>{
                            let MessageEvent::StatusChanged{status,..}=signalman::message::sent_event(id,&sent,observed())else{unreachable!()};status
                        },
                        Err(error)=>MessageStatus::Failed(format!("station carriage failed: {error}")),
                    },
                    _=stopped(stop)=>MessageStatus::Failed("station stopped before the carriage outcome was known".into()),
                }
            }
        }
    };
    let _ = events.send(StationEvent::Message {
        event: MessageEvent::StatusChanged {
            id,
            status,
            observed_unix_ms: observed(),
        },
        complete: true,
    });
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use signalman::message::{MessagePeer, TextMessage};
    fn message(n: u8) -> Message {
        TextMessage::compose(
            MessagePeer::new([1; 16], Some([1; 32])),
            MessagePeer::new([2; 16], None),
            100,
            [n; 32],
            "hello",
        )
        .into()
    }
    #[test]
    fn handoff_is_bounded_deduplicated_and_waits_for_persisted_acknowledgement() {
        let (sender, mut receiver) = channel();
        for n in 0..16 {
            assert_eq!(sender.submit(message(n)).unwrap(), true);
        }
        assert_eq!(sender.submit(message(0)).unwrap(), false);
        assert!(sender.submit(message(17)).is_err());
        let first = receiver.try_recv().unwrap();
        assert!(
            sender.submit(message(17)).is_err(),
            "dequeue alone is not a durable observation"
        );
        sender.acknowledge(first.id());
        assert_eq!(sender.submit(message(17)).unwrap(), true);
        drop(receiver);
        sender.acknowledge(message(1).id());
        assert!(sender.submit(message(18)).is_err());
    }
}
