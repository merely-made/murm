//! Desktop persistence adapters for Signalman's message authority.

use std::path::Path;

use gaz::{Contact, ContactBook, Endpoint, EndpointKind, PersonaScope, TypedKey};
use muniment::{Backend, Journal, JsonSlots, MemoryBackend, RedbBackend, StoreError};
use signalman::message::{ApplyOutcome, MessageBook, MessageEvent, MessagePeer, MessageRecord};

const LOG_SLOT: &str = "signalman/messages/v1/log";
const CONTACTS_SLOT: &str = "signalman/messages/v1/contacts";

type DynBackend = Box<dyn Backend + Send + Sync>;

/// Shared handle to the one durable application journal. Comms and the face
/// serialize mutations through this owner rather than retaining separate books.
#[derive(Clone)]
pub struct MessageStore {
    inner: std::sync::Arc<std::sync::Mutex<JournalStore>>,
}

impl MessageStore {
    pub fn memory(scope: impl Into<String>) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(JournalStore::memory(scope))),
        }
    }
    pub fn open(
        path: impl AsRef<Path>,
        scope: impl Into<String>,
    ) -> Result<Self, MessageStoreError> {
        Ok(Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(JournalStore::open(path, scope)?)),
        })
    }
    pub fn append(&self, event: MessageEvent) -> Result<ApplyOutcome, MessageStoreError> {
        self.inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)?
            .append(event)
    }
    pub fn records(&self) -> std::vec::IntoIter<MessageRecord> {
        self.snapshot()
            .expect("message journal owner poisoned")
            .into_iter()
    }
    pub(crate) fn snapshot(&self) -> Result<Vec<MessageRecord>, MessageStoreError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)?
            .records()
            .cloned()
            .collect())
    }
    /// A stable, cheap read of the current owner materialization. No journal
    /// or mutable authority is exposed, and concurrent writes publish a new book.
    pub fn book(&self) -> std::sync::Arc<MessageBook> {
        self.try_book().expect("message journal owner poisoned")
    }
    pub(crate) fn try_book(&self) -> Result<std::sync::Arc<MessageBook>, MessageStoreError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)?
            .book
            .clone())
    }
    /// Recover one unambiguous public authoring identity from the owner's
    /// history. This enables offline drafts; it grants no station authority.
    pub fn known_local_peer(&self) -> Option<MessagePeer> {
        let mut known = None;
        for record in self.book().iter() {
            let peer = match record.direction {
                signalman::message::MessageDirection::Outgoing => record.message.sender(),
                signalman::message::MessageDirection::Incoming => record.message.recipient(),
            };
            if peer.identity.is_none() {
                continue;
            }
            if known.is_some_and(|existing| existing != peer) {
                return None;
            }
            known = Some(peer);
        }
        known
    }
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("message journal owner poisoned")
            .len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .expect("message journal owner poisoned")
            .is_empty()
    }
    pub fn log_len(&self) -> usize {
        self.inner
            .lock()
            .expect("message journal owner poisoned")
            .log_len()
    }
    pub fn contacts(&self) -> ContactBook {
        self.inner
            .lock()
            .expect("message journal owner poisoned")
            .contacts()
            .clone()
    }
    pub fn contact_name(&self, peer: MessagePeer) -> Option<String> {
        self.inner
            .lock()
            .expect("message journal owner poisoned")
            .contact_name(peer)
            .map(str::to_owned)
    }
    pub fn save_contact(
        &self,
        peer: MessagePeer,
        petname: impl Into<String>,
        now_ms: u64,
    ) -> Result<(), MessageStoreError> {
        self.inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)?
            .save_contact(peer, petname, now_ms)
    }
    pub(crate) fn queue_text_at(
        &self,
        local: MessagePeer,
        peer: MessagePeer,
        text: &str,
        now_ms: u64,
    ) -> Result<signalman::message::MessageId, MessageStoreError> {
        let mut store = self
            .inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)?;
        let mut nonce = [0; 32];
        let sequence =
            u64::try_from(store.log.len()).map_err(|_| MessageStoreError::SequenceExhausted)?;
        nonce[..8].copy_from_slice(&sequence.to_be_bytes());
        nonce[8..16].copy_from_slice(&now_ms.to_be_bytes());
        let message = signalman::message::TextMessage::compose(local, peer, now_ms, nonce, text);
        let id = message.id;
        store.append(MessageEvent::OutgoingQueued {
            message: message.into(),
            reason: signalman::message::QueuedReason::Offline,
            observed_unix_ms: now_ms,
        })?;
        Ok(id)
    }
}

struct JournalStore {
    slots: JsonSlots<DynBackend>,
    log: Journal<MessageEvent>,
    book: std::sync::Arc<MessageBook>,
    contacts: ContactBook,
}

impl JournalStore {
    pub fn memory(scope: impl Into<String>) -> Self {
        Self::from_backend(Box::new(MemoryBackend::new()), scope)
            .expect("a fresh memory message store loads")
    }

    pub fn open(
        path: impl AsRef<Path>,
        scope: impl Into<String>,
    ) -> Result<Self, MessageStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_backend(Box::new(RedbBackend::open(path)?), scope)
    }

    fn from_backend(
        backend: DynBackend,
        scope: impl Into<String>,
    ) -> Result<Self, MessageStoreError> {
        let expected = PersonaScope::new(scope);
        let slots = JsonSlots::new(backend);
        let (log, contacts) = pollster::block_on(async {
            let log = Journal::<MessageEvent>::load(&slots, LOG_SLOT).await?;
            let contacts = slots
                .load::<ContactBook>(CONTACTS_SLOT)
                .await?
                .unwrap_or_else(|| ContactBook::new(expected.clone()));
            Ok::<_, StoreError>((log, contacts))
        })?;
        contacts.verify_scope(&expected)?;
        let book = std::sync::Arc::new(MessageBook::replay(log.entries())?);
        Ok(Self {
            slots,
            log,
            book,
            contacts,
        })
    }

    /// Validate and persist a candidate log before exposing its materialized
    /// state. A failed store write leaves both in-memory objects unchanged.
    pub fn append(&mut self, event: MessageEvent) -> Result<ApplyOutcome, MessageStoreError> {
        let mut candidate_book = self.book.clone();
        let outcome = std::sync::Arc::make_mut(&mut candidate_book).apply(&event)?;
        if outcome == ApplyOutcome::Duplicate {
            return Ok(outcome);
        }
        let mut candidate_log = self.log.clone();
        candidate_log.append(event);
        pollster::block_on(candidate_log.save(&self.slots, LOG_SLOT))?;
        self.log = candidate_log;
        self.book = candidate_book;
        Ok(outcome)
    }

    pub fn records(&self) -> impl DoubleEndedIterator<Item = &MessageRecord> {
        self.book.iter()
    }

    pub fn len(&self) -> usize {
        self.book.len()
    }

    pub fn is_empty(&self) -> bool {
        self.book.is_empty()
    }

    pub fn log_len(&self) -> usize {
        self.log.len()
    }

    pub fn contacts(&self) -> &ContactBook {
        &self.contacts
    }

    pub fn contact_name(&self, peer: MessagePeer) -> Option<&str> {
        // `by_key` ties its borrow to the local key, so match by hand.
        let key = TypedKey::ed25519(peer.identity?);
        self.contacts
            .iter()
            .find(|contact| contact.knows_key(&key))
            .map(|contact| contact.petname.as_str())
    }

    /// Saving a sender is an explicit owner action. Merely receiving a proven
    /// key never calls this method.
    pub fn save_contact(
        &mut self,
        peer: MessagePeer,
        petname: impl Into<String>,
        now_ms: u64,
    ) -> Result<(), MessageStoreError> {
        let identity = peer.identity.ok_or(MessageStoreError::UnprovenContact)?;
        let key = TypedKey::ed25519(identity);
        let address = hex_bytes(&peer.destination);
        let mut contact = self
            .contacts
            .by_key(&key)
            .next()
            .cloned()
            .unwrap_or_else(|| {
                Contact::new(petname, key).with_endpoint(Endpoint::new(
                    EndpointKind::Other("reticulum".into()),
                    address,
                ))
            });
        contact.mark_contacted(now_ms);
        let mut candidate = self.contacts.clone();
        candidate.insert(contact);
        pollster::block_on(self.slots.save(CONTACTS_SLOT, &candidate))?;
        self.contacts = candidate;
        Ok(())
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[derive(Debug, thiserror::Error)]
pub enum MessageStoreError {
    #[error("the message journal owner is unavailable")]
    Unavailable,
    #[error("the message journal sequence is exhausted")]
    SequenceExhausted,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Replay(#[from] signalman::message::MessageError),
    #[error(transparent)]
    Scope(#[from] gaz::ScopeMismatch),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("a contact requires an authenticated sender key")]
    UnprovenContact,
}

#[cfg(test)]
mod tests {
    use super::*;
    use signalman::message::{QueuedReason, TextMessage, VoiceMessage};
    use signalman::voice::{VoiceClip, VoiceEncoding};

    #[test]
    fn unknown_authenticated_sender_remains_outside_gaz_until_owner_saves_it() {
        let local = MessagePeer::new([1; 16], Some([1; 32]));
        let unknown = MessagePeer::new([2; 16], Some([2; 32]));
        let message = TextMessage::compose(unknown, local, 10, [3; 32], "hello");
        let store = MessageStore::memory("local");
        store
            .append(MessageEvent::IncomingReceived {
                message: message.into(),
                transport_id: [4; 32],
                mode: signalman::message::MessageTransport::Data,
                observed_unix_ms: 11,
            })
            .unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.contacts().is_empty());
        assert_eq!(store.contact_name(unknown), None);

        store.save_contact(unknown, "Alice", 12).unwrap();
        assert_eq!(store.contact_name(unknown), Some("Alice".into()));
    }

    #[test]
    fn duplicate_event_does_not_grow_journal() {
        let message = TextMessage::compose(
            MessagePeer::new([1; 16], Some([1; 32])),
            MessagePeer::new([2; 16], None),
            10,
            [3; 32],
            "hello",
        );
        let event = MessageEvent::OutgoingQueued {
            message: message.into(),
            reason: QueuedReason::Offline,
            observed_unix_ms: 11,
        };
        let store = MessageStore::memory("local");
        assert_eq!(store.append(event.clone()).unwrap(), ApplyOutcome::Applied);
        assert_eq!(store.append(event).unwrap(), ApplyOutcome::Duplicate);
        assert_eq!(store.log_len(), 1);
    }

    #[test]
    fn redb_reopen_replays_the_exact_journal_material() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("messages.redb");
        let message = TextMessage::compose(
            MessagePeer::new([1; 16], Some([1; 32])),
            MessagePeer::new([2; 16], None),
            10,
            [3; 32],
            "survives restart",
        );
        let id = message.id;
        {
            let first = MessageStore::open(&path, "local").unwrap();
            first
                .append(MessageEvent::OutgoingQueued {
                    message: message.into(),
                    reason: QueuedReason::Offline,
                    observed_unix_ms: 11,
                })
                .unwrap();
        }
        let reopened = MessageStore::open(&path, "local").unwrap();
        let record = reopened.records().next().unwrap();
        assert_eq!(record.message.id(), id);
        assert_eq!(record.message.text(), Some("survives restart"));
        assert_eq!(record.status.label(), "offline, queued");
        assert_eq!(reopened.log_len(), 1);
    }

    #[test]
    fn voice_reopens_from_the_same_journal_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice.redb");
        let clip = VoiceClip::encode_pcm(&vec![1_000_i16; 1_440], VoiceEncoding::Lpc10).unwrap();
        let message = VoiceMessage::compose(
            MessagePeer::new([1; 16], Some([1; 32])),
            MessagePeer::new([2; 16], None),
            10,
            [3; 32],
            clip.clone(),
        )
        .unwrap();
        let id = message.id;
        {
            let first = MessageStore::open(&path, "local").unwrap();
            first
                .append(MessageEvent::OutgoingQueued {
                    message: message.into(),
                    reason: QueuedReason::Offline,
                    observed_unix_ms: 11,
                })
                .unwrap();
        }

        let reopened = MessageStore::open(&path, "local").unwrap();
        let record = reopened.records().next().unwrap();
        let voice = record.message.voice().unwrap();
        assert_eq!(record.message.id(), id);
        assert_eq!(voice.clip, clip);
        assert_eq!(voice.facts().duration_ms, 180);
        assert_eq!(reopened.log_len(), 1);
    }
}
