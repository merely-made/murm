//! Comms presentation and public journals for resident and stock radio owners.
//!
//! Journals contain text and observed wire facts, never channel keys or private
//! identity seeds. A resident host supplies its authorized runtime and durable
//! packet-ID reservations. Stock companion hosts supply an exclusive byte link;
//! firmware owns keys and RF execution. Both paths use a host-owned realm name.
//! The companion client cannot attach its journal to a resident PHY runtime.
pub mod carriage;
pub mod companion;
mod contacts;
mod journal;
mod resident;
mod scope;
pub mod sennet_ids;
#[cfg(test)]
mod tests;

use crate::comms_adapter::comms;
use async_trait::async_trait;
use comms::{
    AdapterError, Conversation, ConversationId, DeliveryQueueReason, DeliveryStatus, Direction,
    Draft, Identity, Message, MessageBody, MessageId, ProtocolAdapter, ProtocolKind,
};
pub use journal::{FileJournal, RadioJournal};
pub use resident::ResidentSession;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_RECORDS: usize = 2048;

use scope::Scope;
pub use scope::{CompanionScope, RadioScope};

/// Native protocol facts remain available alongside the portable projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RadioFact {
    /// Public pairwise binding; private cipher material never enters history.
    MeshtasticPkiBinding {
        peer: u32,
        local_public: [u8; 32],
        peer_public: [u8; 32],
    },
    /// A matched routing response. Channel responses are not authenticated.
    MeshtasticRouting {
        peer: u32,
        request_id: u32,
        code: u32,
        authenticated: bool,
    },
    MeshtasticPacket {
        source: u32,
        destination: u32,
        packet_id: u32,
        channel_hash: u8,
    },
    MeshCoreOperation {
        timestamp: u32,
    },
    /// Private text verified by the protocol's 16-bit pairwise MAC, not an
    /// Ed25519 signature. `attempt` is the two-bit wire attempt number.
    MeshCoreText {
        sender_key: [u8; 32],
        timestamp: u32,
        attempt: u8,
    },
    /// Matching a 32-bit, unencrypted ACK does not authenticate its origin.
    MeshCoreAck {
        value: [u8; 4],
    },
    PhysicalCompletion {
        generation: u64,
        sequence: u64,
        transmitted: bool,
    },
    CompanionPacket {
        source: u32,
        destination: u32,
        packet_id: u32,
        channel_index: u8,
        pki_reported: bool,
    },
    CompanionRequest {
        packet_id: u32,
    },
    CompanionAccepted {
        correlation: u32,
    },
    CompanionReceived {
        protocol: ProtocolKind,
        source: String,
        timestamp: u32,
    },
    Refusal {
        detail: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RadioRecord {
    pub conversation: ConversationId,
    pub message: Message,
    pub delivery: DeliveryStatus,
    pub facts: Vec<RadioFact>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Book {
    version: u8,
    scope: Scope,
    next_id: u64,
    records: Vec<RadioRecord>,
    #[serde(default)]
    notices: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    meshcore_contacts: Vec<[u8; 32]>,
}
struct Inner {
    book: Book,
    journal: Box<dyn RadioJournal>,
    attached: bool,
    fault: Option<String>,
}
/// Cloneable handle to one durable public journal, shared by Comms and the
/// resident owner. A failed write leaves the previous published book intact.
#[derive(Clone)]
pub struct RadioComms {
    inner: Arc<Mutex<Inner>>,
}
fn backend(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::Backend(error.to_string())
}
impl RadioComms {
    pub fn open(
        scope: RadioScope,
        mut journal: impl RadioJournal + 'static,
    ) -> Result<Self, AdapterError> {
        let bytes = journal.load().map_err(backend)?;
        Self::open_loaded(Scope::Resident(scope), journal, bytes)
    }
    /// Reopen a pre-existing public journal; this never creates a local identity.
    pub fn reopen(mut journal: impl RadioJournal + 'static) -> Result<Self, AdapterError> {
        let bytes = journal
            .load()
            .map_err(backend)?
            .ok_or(AdapterError::NotFound)?;
        let book: Book = serde_json::from_slice(&bytes).map_err(backend)?;
        Self::open_loaded(book.scope, journal, Some(bytes))
    }
    fn open_loaded(
        scope: Scope,
        mut journal: impl RadioJournal + 'static,
        bytes: Option<Vec<u8>>,
    ) -> Result<Self, AdapterError> {
        if !scope.valid() {
            return Err(AdapterError::Unsupported(
                "invalid public radio realm".into(),
            ));
        }
        let mut book: Book = match bytes {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(backend)?,
            None => Book {
                version: 1,
                scope: scope.clone(),
                next_id: 1,
                records: Vec::new(),
                notices: Vec::new(),
                meshcore_contacts: Vec::new(),
            },
        };
        if !matches!(book.version, 1 | 2)
            || book.scope != scope
            || book.records.len() > MAX_RECORDS
            || book.next_id == 0
        {
            return Err(backend("radio journal scope/version/bounds mismatch"));
        }
        contacts::validate_book(&scope, &book.meshcore_contacts)?;
        if (book.version == 1 && !book.meshcore_contacts.is_empty())
            || (book.version == 2 && !matches!(scope, Scope::Resident(_)))
        {
            return Err(backend(
                "contact cache requires a version-2 resident journal",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for record in &mut book.records {
            scope.peer(&record.conversation)?;
            if let Some(sequence) = record.message.id.0.strip_prefix("out-") {
                let sequence: u64 = sequence.parse().map_err(backend)?;
                if sequence >= book.next_id {
                    return Err(backend("journal counter would reuse a message ID"));
                }
            }
            if !ids.insert(record.message.id.clone()) {
                return Err(backend("duplicate journal message ID"));
            }
            if record.message.direction == Direction::Outgoing
                && matches!(
                    record.delivery,
                    DeliveryStatus::Unknown
                        | DeliveryStatus::Queued(DeliveryQueueReason::ReadyForCarriage)
                )
            {
                record.delivery = DeliveryStatus::Failed {
                    detail: "radio outcome unknown after reopen; explicit resend required".into(),
                };
            }
        }
        journal
            .commit(&serde_json::to_vec(&book).map_err(backend)?)
            .map_err(backend)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                book,
                journal: Box::new(journal),
                attached: false,
                fault: None,
            })),
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, AdapterError> {
        self.inner.lock().map_err(backend)
    }
    fn edit<T>(
        &self,
        change: impl FnOnce(&mut Book) -> Result<T, AdapterError>,
    ) -> Result<T, AdapterError> {
        let mut inner = self.lock()?;
        if let Some(fault) = &inner.fault {
            return Err(backend(fault));
        }
        let mut candidate = inner.book.clone();
        let result = change(&mut candidate)?;
        let bytes = serde_json::to_vec(&candidate).map_err(backend)?;
        if let Err(error) = inner.journal.commit(&bytes) {
            if inner.attached {
                inner.fault = Some(format!(
                    "resident journal write failed; stop carriage: {error}"
                ));
            }
            return Err(backend(error));
        }
        inner.book = candidate;
        Ok(result)
    }
    pub fn scope(&self) -> Result<RadioScope, AdapterError> {
        match &self.lock()?.book.scope {
            Scope::Resident(scope) => Ok(scope.clone()),
            _ => Err(backend("this is a stock companion scope")),
        }
    }
    pub fn open_companion(
        scope: CompanionScope,
        mut journal: impl RadioJournal + 'static,
    ) -> Result<Self, AdapterError> {
        let bytes = journal.load().map_err(backend)?;
        Self::open_loaded(Scope::Companion(scope), journal, bytes)
    }
    pub fn supports(&self, protocol: ProtocolKind) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.book.scope.protocols().contains(&protocol))
            .unwrap_or(false)
    }
    pub fn conversation(
        &self,
        protocol: ProtocolKind,
        peer: &str,
    ) -> Result<ConversationId, AdapterError> {
        self.lock()?.book.scope.conversation(protocol, peer)
    }
    pub fn records(&self) -> Result<Vec<RadioRecord>, AdapterError> {
        Ok(self.lock()?.book.records.clone())
    }
    pub fn diagnostics(&self) -> Result<Vec<String>, AdapterError> {
        let inner = self.lock()?;
        let mut lines = vec!["Meshtastic channel senders are unauthenticated; MeshCore private text uses a 16-bit pairwise MAC; ACKs are unencrypted 32-bit matches.".into()];
        if matches!(inner.book.scope, Scope::Companion(_)) {
            lines.push("Stock companion observations are device-reported. Signalman does not verify PKI proofs or treat protocol ACKs as authenticated/read receipts.".into());
        }
        if let Some(fault) = &inner.fault {
            lines.push(fault.clone());
        }
        lines.extend(inner.book.notices.clone());
        lines.extend(inner.book.records.iter().filter_map(|r| match &r.delivery {
            DeliveryStatus::Failed { detail } => Some(format!("{}: {detail}", r.message.id.0)),
            _ => None,
        }));
        Ok(lines)
    }
    pub fn add_protocol_to(&self, model: &mut comms::Comms, protocol: ProtocolKind) {
        if self.supports(protocol) {
            model.add_adapter(Box::new(RadioAdapter {
                owner: self.clone(),
                protocol,
            }));
        }
    }
    pub fn add_to(&self, model: &mut comms::Comms) {
        for protocol in [ProtocolKind::Meshtastic, ProtocolKind::MeshCore] {
            self.add_protocol_to(model, protocol);
        }
    }
    pub fn model(&self) -> comms::Comms {
        let mut model = comms::Comms::new();
        self.add_to(&mut model);
        model
    }
    fn update(
        &self,
        id: &MessageId,
        status: DeliveryStatus,
        facts: Vec<RadioFact>,
    ) -> Result<(), AdapterError> {
        self.edit(|book| {
            let record = book
                .records
                .iter_mut()
                .find(|r| &r.message.id == id)
                .ok_or(AdapterError::NotFound)?;
            if record.delivery != DeliveryStatus::ProtocolAcknowledged {
                record.delivery = status;
            }
            for fact in facts {
                if !record.facts.contains(&fact) {
                    record.facts.push(fact);
                }
            }
            Ok(())
        })
    }
    fn notice(&self, detail: String) -> Result<(), AdapterError> {
        self.edit(|book| {
            if book.notices.len() == 32 {
                book.notices.remove(0);
            }
            book.notices.push(detail);
            Ok(())
        })
    }
    fn receive(&self, record: RadioRecord) -> Result<(), AdapterError> {
        self.edit(|book| {
            if let Some(previous) = book
                .records
                .iter_mut()
                .find(|r| r.message.id == record.message.id)
            {
                if previous.message.body != record.message.body
                    || previous.conversation != record.conversation
                {
                    return Err(backend("conflicting radio message identity"));
                }
                for fact in record.facts {
                    if !previous.facts.contains(&fact) {
                        previous.facts.push(fact);
                    }
                }
                return Ok(());
            }
            if book.records.len() == MAX_RECORDS {
                return Err(backend("radio journal full"));
            }
            book.records.push(record);
            Ok(())
        })
    }
}
struct RadioAdapter {
    owner: RadioComms,
    protocol: ProtocolKind,
}
#[async_trait]
impl ProtocolAdapter for RadioAdapter {
    fn protocol(&self) -> ProtocolKind {
        self.protocol
    }
    fn identity(&self) -> Identity {
        // Scope is immutable after construction; a poisoned journal is surfaced
        // by every fallible operation rather than changing the local identity.
        let scope = self
            .owner
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .book
            .scope
            .clone();
        Identity::new(self.protocol, scope.local(self.protocol))
    }
    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        let scope = self.owner.lock()?.book.scope.clone();
        let mut rows: BTreeMap<String, Conversation> = BTreeMap::new();
        for record in self
            .owner
            .records()?
            .into_iter()
            .filter(|r| r.conversation.protocol == self.protocol)
        {
            let peer = scope.peer(&record.conversation)?;
            let row = rows
                .entry(record.conversation.key.clone())
                .or_insert(Conversation {
                    id: record.conversation,
                    title: peer.clone(),
                    participants: vec![self.identity(), Identity::new(self.protocol, peer)],
                    last_activity_ms: None,
                    unread: 0,
                });
            row.last_activity_ms = row.last_activity_ms.max(record.message.timestamp_ms);
        }
        Ok(rows.into_values().collect())
    }
    async fn messages(&self, id: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        if id.protocol != self.protocol {
            return Err(AdapterError::NotFound);
        }
        self.owner.lock()?.book.scope.peer(id)?;
        Ok(self
            .owner
            .records()?
            .into_iter()
            .filter(|r| &r.conversation == id)
            .map(|r| r.message)
            .collect())
    }
    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let target = draft.conversation.as_ref().ok_or(AdapterError::NotFound)?;
        if target.protocol != self.protocol {
            return Err(AdapterError::NotFound);
        }
        self.owner.lock()?.book.scope.peer(target)?;
        let author = self.identity();
        let limit = if self.protocol == ProtocolKind::Meshtastic {
            sennet::application::MAX_APPLICATION_PAYLOAD
        } else {
            if matches!(self.owner.lock()?.book.scope, Scope::Companion(_)) {
                133
            } else {
                tucket::node::MAX_TEXT_BYTES
            }
        };
        if draft.subject.is_some()
            || draft.body.trim().is_empty()
            || draft.body.len() > limit
            || draft.body.contains('\0')
        {
            return Err(AdapterError::Unsupported(format!(
                "plain text without subject/NUL required; limit {limit} UTF-8 bytes"
            )));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(backend)?
            .as_millis();
        let now = u64::try_from(now).map_err(backend)?;
        self.owner.edit(|book| {
            if book.records.len() == MAX_RECORDS {
                return Err(backend("radio journal full"));
            }
            let id = MessageId(format!("out-{}", book.next_id));
            book.next_id = book
                .next_id
                .checked_add(1)
                .ok_or_else(|| backend("message ID exhausted"))?;
            book.records.push(RadioRecord {
                conversation: target.clone(),
                message: Message {
                    id: id.clone(),
                    author: author.clone(),
                    body: MessageBody::PlainText(draft.body.clone()),
                    subject: None,
                    timestamp_ms: Some(now),
                    direction: Direction::Outgoing,
                },
                delivery: DeliveryStatus::Queued(DeliveryQueueReason::Offline),
                facts: vec![],
            });
            // The target is validated before any mutation; native addressing
            // stays in the scoped conversation and is checked again at dispatch.
            Ok(id)
        })
    }
    async fn delivery_status(
        &self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<DeliveryStatus, AdapterError> {
        if conversation.protocol != self.protocol {
            return Err(AdapterError::NotFound);
        }
        self.owner.lock()?.book.scope.peer(conversation)?;
        self.owner
            .records()?
            .into_iter()
            .find(|r| &r.conversation == conversation && &r.message.id == message)
            .map(|r| r.delivery)
            .ok_or(AdapterError::NotFound)
    }
}
