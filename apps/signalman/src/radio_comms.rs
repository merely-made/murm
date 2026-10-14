//! Comms presentation and public journals for resident and stock radio owners.
//!
//! Journals contain text and observed wire facts, never channel keys or private
//! identity seeds. A resident host supplies its authorized runtime and durable
//! packet-ID reservations. Stock companion hosts supply an exclusive byte link;
//! firmware owns keys and RF execution. Both paths use a host-owned realm name.
//! The companion client cannot attach its journal to a resident PHY runtime.
pub mod companion;
mod journal;
mod resident;
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

/// A caller-owned realm binds channel configuration and public device identities.
/// The 8-bit Meshtastic channel hash alone is not a realm identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioScope {
    pub realm: String,
    pub meshtastic_node: u32,
    pub meshtastic_channel: u8,
    pub meshcore_key: [u8; 32],
}
impl RadioScope {
    fn local(&self, protocol: ProtocolKind) -> String {
        match protocol {
            ProtocolKind::Meshtastic => format!("{:08x}", self.meshtastic_node),
            ProtocolKind::MeshCore => hex::encode(self.meshcore_key),
            _ => unreachable!("radio adapter protocol"),
        }
    }
    pub fn meshtastic(&self, peer: u32) -> ConversationId {
        ConversationId::new(
            ProtocolKind::Meshtastic,
            format!(
                "{}/{:08x}/{:02x}/{peer:08x}",
                self.realm, self.meshtastic_node, self.meshtastic_channel
            ),
        )
    }
    pub fn meshcore(&self, peer: [u8; 32]) -> ConversationId {
        ConversationId::new(
            ProtocolKind::MeshCore,
            format!(
                "{}/{}/{}",
                self.realm,
                hex::encode(self.meshcore_key),
                hex::encode(peer)
            ),
        )
    }
    fn peer(&self, id: &ConversationId) -> Result<String, AdapterError> {
        if !matches!(
            id.protocol,
            ProtocolKind::Meshtastic | ProtocolKind::MeshCore
        ) {
            return Err(AdapterError::NotFound);
        }
        let local = self.local(id.protocol);
        let prefix = match id.protocol {
            ProtocolKind::Meshtastic => {
                format!("{}/{local}/{:02x}/", self.realm, self.meshtastic_channel)
            }
            ProtocolKind::MeshCore => format!("{}/{local}/", self.realm),
            _ => return Err(AdapterError::NotFound),
        };
        let peer = id.key.strip_prefix(&prefix).ok_or(AdapterError::NotFound)?;
        let length = if id.protocol == ProtocolKind::Meshtastic {
            8
        } else {
            64
        };
        if peer.len() != length
            || !peer
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AdapterError::NotFound);
        }
        Ok(peer.to_owned())
    }
}

/// Public identity returned by one stock companion. No synthetic identity is
/// created for the other protocol. Realm names remain host-owned configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "companion", deny_unknown_fields)]
pub enum CompanionScope {
    Meshtastic {
        realm: String,
        node: u32,
        channel: u8,
    },
    MeshCore {
        realm: String,
        key: [u8; 32],
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Scope {
    Resident(RadioScope),
    Companion(CompanionScope),
}
impl Scope {
    fn protocols(&self) -> Vec<ProtocolKind> {
        match self {
            Self::Resident(_) => vec![ProtocolKind::Meshtastic, ProtocolKind::MeshCore],
            Self::Companion(CompanionScope::Meshtastic { .. }) => vec![ProtocolKind::Meshtastic],
            Self::Companion(CompanionScope::MeshCore { .. }) => vec![ProtocolKind::MeshCore],
        }
    }
    fn realm(&self) -> &str {
        match self {
            Self::Resident(s) => &s.realm,
            Self::Companion(
                CompanionScope::Meshtastic { realm, .. } | CompanionScope::MeshCore { realm, .. },
            ) => realm,
        }
    }
    fn valid(&self) -> bool {
        let realm = self.realm();
        !realm.is_empty()
            && realm.len() <= 128
            && realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
    fn local(&self, protocol: ProtocolKind) -> String {
        match self {
            Self::Resident(s) => s.local(protocol),
            Self::Companion(CompanionScope::Meshtastic { node, .. }) => format!("{node:08x}"),
            Self::Companion(CompanionScope::MeshCore { key, .. }) => hex::encode(key),
        }
    }
    fn conversation(
        &self,
        protocol: ProtocolKind,
        peer: &str,
    ) -> Result<ConversationId, AdapterError> {
        if !self.protocols().contains(&protocol) {
            return Err(AdapterError::NotFound);
        }
        let channel = match self {
            Self::Resident(s) => s.meshtastic_channel,
            Self::Companion(CompanionScope::Meshtastic { channel, .. }) => *channel,
            _ => 0,
        };
        let local = self.local(protocol);
        let key = if protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{channel:02x}/{peer}", self.realm())
        } else {
            format!("{}/{local}/{peer}", self.realm())
        };
        let id = ConversationId::new(protocol, key);
        self.peer(&id)?;
        Ok(id)
    }
    fn peer(&self, id: &ConversationId) -> Result<String, AdapterError> {
        if !self.protocols().contains(&id.protocol) {
            return Err(AdapterError::NotFound);
        }
        let local = self.local(id.protocol);
        let channel = match self {
            Self::Resident(s) => s.meshtastic_channel,
            Self::Companion(CompanionScope::Meshtastic { channel, .. }) => *channel,
            _ => 0,
        };
        let prefix = if id.protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{channel:02x}/", self.realm())
        } else {
            format!("{}/{local}/", self.realm())
        };
        let peer = id.key.strip_prefix(&prefix).ok_or(AdapterError::NotFound)?;
        let length = if id.protocol == ProtocolKind::Meshtastic {
            8
        } else {
            64
        };
        if peer.len() != length
            || !peer
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AdapterError::NotFound);
        }
        Ok(peer.to_owned())
    }
}

/// Native protocol facts remain available alongside the portable projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RadioFact {
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
            },
        };
        if book.version != 1
            || book.scope != scope
            || book.records.len() > MAX_RECORDS
            || book.next_id == 0
        {
            return Err(backend("radio journal scope/version/bounds mismatch"));
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
