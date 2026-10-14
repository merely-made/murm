//! The attended face and Comms share the same Signalman journal owner.
use crate::messages::MessageStore;
use crate::station::StationSender;
use signalman::comms_adapter::{LxmfAdapter, LxmfMailbox, comms};
use signalman::message::{MessageId, MessagePeer};

pub(crate) struct Mailbox {
    pub store: MessageStore,
    pub station: Option<StationSender>,
    pub now_ms: Option<u64>,
}

impl LxmfMailbox for Mailbox {
    fn book(&self) -> Result<std::sync::Arc<signalman::message::MessageBook>, comms::AdapterError> {
        self.store
            .try_book()
            .map_err(|error| comms::AdapterError::Backend(error.to_string()))
    }
    fn petname(&self, peer: MessagePeer) -> Option<String> {
        self.store.contact_name(peer)
    }
    fn queue_text(
        &self,
        local: MessagePeer,
        peer: MessagePeer,
        text: &str,
    ) -> Result<MessageId, comms::AdapterError> {
        let now = self.now_ms.map(Ok).unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| comms::AdapterError::Backend(error.to_string()))
                .and_then(|duration| {
                    u64::try_from(duration.as_millis())
                        .map_err(|error| comms::AdapterError::Backend(error.to_string()))
                })
        })?;
        let id = self
            .store
            .queue_text_at(local, peer, text, now)
            .map_err(|error| comms::AdapterError::Backend(error.to_string()))?;
        if let Some(station) = &self.station {
            let book = self.store.book();
            let record = book.get(id).expect("persisted message is present");
            // A busy/offline actor leaves the already persisted intent queued.
            // The host retries queued intents after station progress; it never
            // reports a network failure as a failure to persist this draft.
            let _ = station.submit(record.message.clone());
        }
        Ok(id)
    }
}

pub(crate) fn model(
    local: MessagePeer,
    store: MessageStore,
    station: Option<StationSender>,
    now_ms: Option<u64>,
) -> comms::Comms {
    comms::Comms::new().with_adapter(Box::new(LxmfAdapter::new(
        local,
        Mailbox {
            store,
            station,
            now_ms,
        },
    )))
}

#[cfg(test)]
mod tests;
