use super::*;
use radio_hand::{
    instances::{Event, Report, Runtime, SENNET, TUCKET},
    work::{Transmission, WorkId},
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Exclusive borrowed access to an already commissioned resident runtime.
/// Physical execution stays with the host: call `begin_tx`, perform bounded
/// carriage through its authorized device owner, then report `complete_tx`.
/// Dropping a session marks unresolved operations uncertain instead of replaying
/// them against another runtime whose operation IDs might start again at one.
pub struct ResidentSession<'a> {
    owner: RadioComms,
    runtime: &'a mut Runtime,
    bindings: HashMap<(u8, u32), MessageId>,
    timestamp: HashMap<u32, u32>,
    transmissions: HashMap<(u8, u64, u64, Option<u64>), MessageId>,
}
impl RadioComms {
    pub fn attach<'a>(
        &self,
        runtime: &'a mut Runtime,
    ) -> Result<ResidentSession<'a>, AdapterError> {
        let mut inner = self.lock()?;
        if inner.attached {
            return Err(backend("resident journal already attached"));
        }
        let Scope::Resident(scope) = &inner.book.scope else {
            return Err(backend(
                "stock companion journals cannot attach to a resident runtime",
            ));
        };
        if runtime
            .installed_sennet()
            .is_some_and(|s| s.address() != (scope.meshtastic_node, scope.meshtastic_channel))
            || runtime
                .installed_tucket()
                .is_some_and(|t| t.node().identity().pub_key != scope.meshcore_key)
        {
            return Err(backend(
                "resident public identity does not match journal realm",
            ));
        }
        let binding = runtime
            .installed_sennet()
            .and_then(|s| s.directed_binding());
        for record in &inner.book.records {
            for fact in &record.facts {
                if let RadioFact::MeshtasticPkiBinding {
                    peer,
                    local_public,
                    peer_public,
                } = fact
                    && binding.is_none_or(|b| {
                        b.peer != *peer
                            || b.local_public != *local_public
                            || b.peer_public != *peer_public
                    })
                {
                    return Err(backend(
                        "Meshtastic history is bound to another pinned PKI identity; select its original configuration or a new realm/journal",
                    ));
                }
            }
        }
        inner.attached = true;
        Ok(ResidentSession {
            owner: self.clone(),
            runtime,
            bindings: HashMap::new(),
            timestamp: HashMap::new(),
            transmissions: HashMap::new(),
        })
    }
}
impl ResidentSession<'_> {
    /// Advertise through the retained runtime and its ordinary physical work queue.
    pub fn advertise_tucket(
        &mut self,
        now: u64,
        unix_seconds: u32,
        name: &str,
    ) -> Result<(), AdapterError> {
        self.healthy()?;
        let data = tucket::advert::AdvertData::try_chat(name)
            .and_then(|d| d.encode())
            .ok_or_else(|| backend("resident advert name exceeds MeshCore bounds"))?;
        let report = self
            .runtime
            .advertise_tucket(now, unix_seconds, &data)
            .map_err(|e| backend(format!("{e:?}")))?;
        self.observe(now, report)
    }
    /// Full-key contact match, after the protocol has verified its advert.
    pub fn knows_tucket_peer(&self, key: &[u8; 32]) -> bool {
        self.runtime
            .installed_tucket()
            .and_then(|t| t.node().contact(key[0]))
            .is_some_and(|c| c.pub_key == *key)
    }
    pub fn transmit_budget_ms(&self) -> u64 {
        self.runtime.tx_budget_ms()
    }
    fn healthy(&self) -> Result<(), AdapterError> {
        let inner = self.owner.lock()?;
        if let Some(fault) = &inner.fault {
            return Err(backend(fault));
        }
        Ok(())
    }
    /// Queue one already durable intent for the currently active personality.
    /// Clock values are host-selected monotonic milliseconds and Unix seconds.
    pub fn dispatch_next(
        &mut self,
        now: u64,
        unix_seconds: u32,
    ) -> Result<Option<MessageId>, AdapterError> {
        self.healthy()?;
        let active = self
            .runtime
            .active()
            .ok_or_else(|| backend("no active resident personality"))?
            .instance;
        let protocol = match active {
            SENNET => ProtocolKind::Meshtastic,
            TUCKET => ProtocolKind::MeshCore,
            _ => return Ok(None),
        };
        if protocol == ProtocolKind::Meshtastic && !self.runtime.sennet().ready_for_text() {
            return Ok(None);
        }
        let scope = self.owner.scope()?;
        let mut next = None;
        for record in self.owner.records()?.into_iter().filter(|r| {
            r.conversation.protocol == protocol
                && r.delivery == DeliveryStatus::Queued(DeliveryQueueReason::Offline)
        }) {
            let peer = scope.peer(&record.conversation)?;
            if protocol == ProtocolKind::MeshCore {
                let key: [u8; 32] = hex::decode(&peer)
                    .map_err(backend)?
                    .try_into()
                    .map_err(|_| backend("invalid MeshCore key"))?;
                // Discovery does not block other recipients whose full keys
                // are already verified by the protocol.
                if !self.knows_tucket_peer(&key) {
                    continue;
                }
            }
            next = Some((record, peer));
            break;
        }
        let Some((record, peer)) = next else {
            return Ok(None);
        };
        let id = record.message.id;
        // Persist the ambiguous boundary before touching protocol state. A crash
        // here cannot silently replay an operation whose carriage was possible.
        let intent = if protocol == ProtocolKind::MeshCore {
            vec![RadioFact::MeshCoreOperation {
                timestamp: unix_seconds,
            }]
        } else if protocol == ProtocolKind::Meshtastic && peer != "ffffffff" {
            self.runtime
                .sennet()
                .directed_binding()
                .filter(|b| u32::from_str_radix(&peer, 16).ok() == Some(b.peer))
                .map(|b| {
                    vec![RadioFact::MeshtasticPkiBinding {
                        peer: b.peer,
                        local_public: b.local_public,
                        peer_public: b.peer_public,
                    }]
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        self.owner.update(&id, DeliveryStatus::Unknown, intent)?;
        let result: Result<(), AdapterError> = (|| {
            match protocol {
                ProtocolKind::Meshtastic => {
                    let destination = u32::from_str_radix(&peer, 16).map_err(backend)?;
                    let header = sennet::transport::Header {
                        destination,
                        source: 0,
                        packet_id: 0,
                        hop_limit: 3,
                        want_ack: destination != u32::MAX,
                        via_mqtt: false,
                        hop_start: 3,
                        channel_hash: 0,
                        next_hop: 0,
                        relay_node: 0,
                    };
                    let report = if destination == u32::MAX {
                        self.runtime
                            .send_sennet(now, header, record.message.body.text())
                    } else {
                        let mut extra = [0; 4];
                        getrandom::fill(&mut extra).map_err(backend)?;
                        self.runtime.send_sennet_directed(
                            now,
                            header,
                            record.message.body.text(),
                            extra,
                        )
                    }
                    .map_err(|e| backend(format!("{e:?}")))?;
                    // A successful runtime call can still report queue loss.
                    let identity = self.runtime.sennet().pending_identity();
                    let operation = self.runtime.sennet().pending_operation_id();
                    if let (Some(identity), Some(operation)) = (identity, operation) {
                        self.bindings.insert((SENNET.0, operation), id.clone());
                        self.owner.update(
                            &id,
                            DeliveryStatus::Queued(DeliveryQueueReason::ReadyForCarriage),
                            vec![RadioFact::MeshtasticPacket {
                                source: identity.source,
                                destination,
                                packet_id: identity.packet_id,
                                channel_hash: if destination == u32::MAX {
                                    scope.meshtastic_channel
                                } else {
                                    0
                                },
                            }],
                        )?;
                    } else {
                        self.owner.update(
                            &id,
                            DeliveryStatus::Failed {
                                detail: "resident rejected prepared Meshtastic physical work"
                                    .into(),
                            },
                            vec![],
                        )?;
                    }
                    self.observe(now, report)?;
                }
                ProtocolKind::MeshCore => {
                    let key: [u8; 32] = hex::decode(peer)
                        .map_err(backend)?
                        .try_into()
                        .map_err(|_| backend("invalid MeshCore key"))?;
                    let contact = self
                        .runtime
                        .tucket()
                        .node()
                        .contact(key[0])
                        .ok_or(AdapterError::NotFound)?;
                    if contact.pub_key != key {
                        return Err(backend("MeshCore one-byte contact collision"));
                    }
                    // Identical timestamp/text tuples are wire-indistinguishable.
                    // Never originate one twice under this scope, even after reopen.
                    if self.owner.records()?.iter().any(|r| r.message.id != id && r.message.direction == Direction::Outgoing &&
                        r.conversation == record.conversation && r.message.body == record.message.body &&
                        r.facts.iter().any(|f| matches!(f, RadioFact::MeshCoreOperation { timestamp } if *timestamp == unix_seconds))) {
                        return Err(backend("duplicate MeshCore timestamp/text tuple; select a later timestamp"));
                    }
                    let expires_at = now
                        .checked_add(30_000)
                        .ok_or_else(|| backend("deadline overflow"))?;
                    let operation = self
                        .runtime
                        .send_tucket(
                            now,
                            key[0],
                            record.message.body.text(),
                            tucket::node::TextRetryPolicy::default(),
                            tucket::instance::SendTiming {
                                timestamp: unix_seconds,
                                expires_at,
                                allowed_until: expires_at,
                            },
                        )
                        .map_err(|e| backend(format!("{e:?}")))?;
                    self.bindings.insert((TUCKET.0, operation.0), id.clone());
                    self.timestamp.insert(operation.0, unix_seconds);
                    self.owner.update(
                        &id,
                        DeliveryStatus::Queued(DeliveryQueueReason::ReadyForCarriage),
                        vec![RadioFact::MeshCoreOperation {
                            timestamp: unix_seconds,
                        }],
                    )?;
                }
                _ => unreachable!(),
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.owner.update(
                &id,
                DeliveryStatus::Failed {
                    detail: error.to_string(),
                },
                vec![RadioFact::Refusal {
                    detail: error.to_string(),
                }],
            )?;
            return Err(error);
        }
        Ok(Some(id))
    }
    pub fn poll(&mut self, now: u64) -> Result<(), AdapterError> {
        self.poll_changed(now).map(|_| ())
    }
    /// Protocol events include timer-only delivery expiry. Hosts use this
    /// result to wake projections even when no radio frame arrived.
    pub fn poll_changed(&mut self, now: u64) -> Result<bool, AdapterError> {
        self.healthy()?;
        let result = self.runtime.poll(now, None);
        let changed = result
            .as_ref()
            .is_ok_and(|r| !r.events.is_empty() || r.overflowed != 0);
        self.finish(now, result)?;
        Ok(changed)
    }
    pub fn ingest(&mut self, now: u64, frame: &[u8]) -> Result<(), AdapterError> {
        self.healthy()?;
        let result = self.runtime.ingest(now, frame);
        self.finish(now, result)
    }
    fn finish(
        &mut self,
        now: u64,
        result: Result<Report, radio_hand::instances::Error>,
    ) -> Result<(), AdapterError> {
        match result {
            Ok(report) => self.observe(now, report),
            Err(error) => {
                let report = self.runtime.take_report();
                self.observe(now, report)?;
                Err(backend(format!("resident refused: {error:?}")))
            }
        }
    }
    pub fn begin_tx(&mut self, now: u64) -> Result<Option<Transmission>, AdapterError> {
        self.healthy()?;
        let Some(tx) = self
            .runtime
            .begin_tx(now)
            .map_err(|e| backend(format!("{e:?}")))?
        else {
            return Ok(None);
        };
        if let Some(operation) = tx.id.operation
            && let Some(id) = self
                .bindings
                .get(&(tx.id.activation.instance.0, operation as u32))
        {
            self.transmissions.insert(work_key(tx.id), id.clone());
        }
        Ok(Some(tx))
    }
    pub fn complete_tx(
        &mut self,
        now: u64,
        work: WorkId,
        transmitted: bool,
    ) -> Result<(), AdapterError> {
        let report = self
            .runtime
            .complete_tx(now, work, transmitted)
            .map_err(|e| backend(format!("{e:?}")))?;
        if let Some(id) = self.transmissions.remove(&work_key(work)) {
            let status = if transmitted {
                DeliveryStatus::HandedToRadio
            } else {
                DeliveryStatus::Failed {
                    detail: "physical owner reported transmission loss".into(),
                }
            };
            self.owner.update(
                &id,
                status,
                vec![RadioFact::PhysicalCompletion {
                    generation: work.activation.generation,
                    sequence: work.sequence,
                    transmitted,
                }],
            )?;
        }
        self.observe(now, report)
    }
    fn observe(&mut self, now: u64, report: Report) -> Result<(), AdapterError> {
        let result = self.observe_report(now, report);
        if let Err(error) = &result {
            self.owner.lock()?.fault = Some(format!(
                "resident observation could not be persisted; stop carriage: {error}"
            ));
        }
        result
    }
    fn observe_report(&mut self, _now: u64, report: Report) -> Result<(), AdapterError> {
        let scope = self.owner.scope()?;
        if report.overflowed != 0 {
            return self.fault(format!("resident report lost {} events", report.overflowed));
        }
        let acks: Vec<_> = report
            .events
            .iter()
            .filter_map(|e| {
                if let Event::Tucket(tucket::node::Event::Ack(ack)) = e {
                    Some(*ack)
                } else {
                    None
                }
            })
            .collect();
        for event in report.events {
            match event {
                Event::SennetReceived(sennet::instance::ReceiveOutcome::PkiText(text)) => {
                    let h = text.header;
                    let b = self
                        .runtime
                        .sennet()
                        .directed_binding()
                        .ok_or_else(|| backend("PKI receive lost its peer binding"))?;
                    self.received_facts(
                        scope.meshtastic(h.source),
                        format!("rx-mt-{:08x}-{:08x}", h.source, h.packet_id),
                        format!("{:08x}", h.source),
                        text.text,
                        None,
                        vec![
                            RadioFact::MeshtasticPacket {
                                source: h.source,
                                destination: h.destination,
                                packet_id: h.packet_id,
                                channel_hash: h.channel_hash,
                            },
                            RadioFact::MeshtasticPkiBinding {
                                peer: b.peer,
                                local_public: b.local_public,
                                peer_public: b.peer_public,
                            },
                        ],
                    )?;
                }
                Event::SennetReceived(sennet::instance::ReceiveOutcome::Routing {
                    operation_id,
                    identity,
                    peer,
                    code,
                    authenticated,
                }) => {
                    if let Some(id) = self.bindings.remove(&(SENNET.0, operation_id)) {
                        self.owner.update(
                            &id,
                            if code == 0 {
                                DeliveryStatus::ProtocolAcknowledged
                            } else {
                                DeliveryStatus::HandedToRadio
                            },
                            vec![RadioFact::MeshtasticRouting {
                                peer,
                                request_id: identity.packet_id,
                                code,
                                authenticated,
                            }],
                        )?;
                        if code != 0 {
                            self.owner.notice(format!(
                                "Meshtastic routing response code {code}; no success ACK observed."
                            ))?;
                        }
                    }
                }
                Event::SennetReceived(sennet::instance::ReceiveOutcome::Text(text)) => {
                    let header = text.header;
                    if header.source == scope.meshtastic_node {
                        continue;
                    }
                    let peer = if header.destination == u32::MAX {
                        u32::MAX
                    } else {
                        header.source
                    };
                    let id = format!("rx-mt-{:08x}-{:08x}", header.source, header.packet_id);
                    self.received(
                        scope.meshtastic(peer),
                        id,
                        format!("{:08x}", header.source),
                        text.text,
                        None,
                        RadioFact::MeshtasticPacket {
                            source: header.source,
                            destination: header.destination,
                            packet_id: header.packet_id,
                            channel_hash: header.channel_hash,
                        },
                    )?;
                }
                Event::Tucket(tucket::node::Event::Message { from, message, .. }) => {
                    if message.txt_type != tucket::message::TXT_TYPE_PLAIN {
                        self.owner.notice(format!(
                            "MeshCore text type {} is unsupported in Comms",
                            message.txt_type
                        ))?;
                        continue;
                    }
                    let key = self
                        .runtime
                        .tucket()
                        .node()
                        .contact(from)
                        .ok_or_else(|| {
                            backend("received MeshCore sender missing from retained contact map")
                        })?
                        .pub_key;
                    let mut digest = Sha256::new();
                    digest.update(key);
                    digest.update(message.timestamp.to_le_bytes());
                    digest.update(message.text.as_bytes());
                    let id = format!("rx-mc-{}", hex::encode(digest.finalize()));
                    self.received(
                        scope.meshcore(key),
                        id,
                        hex::encode(key),
                        message.text,
                        Some(u64::from(message.timestamp) * 1000),
                        RadioFact::MeshCoreText {
                            sender_key: key,
                            timestamp: message.timestamp,
                            attempt: message.attempt,
                        },
                    )?;
                }
                Event::Sennet(event) => {
                    if let sennet::instance::InstanceEvent::AckExpired { operation_id, .. }
                    | sennet::instance::InstanceEvent::AckLost { operation_id, .. } = event
                    {
                        if self.bindings.remove(&(SENNET.0, operation_id)).is_some() {
                            self.owner.notice("Meshtastic acknowledgement window ended without a matching response; physical completion remains recorded.".into())?;
                        }
                        continue;
                    }
                    let (op, status) = match event {
                        sennet::instance::InstanceEvent::TxCompleted { operation_id, .. } => {
                            (operation_id, DeliveryStatus::HandedToRadio)
                        }
                        sennet::instance::InstanceEvent::PendingExpired {
                            operation_id, ..
                        } => (
                            operation_id,
                            DeliveryStatus::Failed {
                                detail: "Meshtastic operation expired".into(),
                            },
                        ),
                        sennet::instance::InstanceEvent::PendingLost { operation_id, .. } => (
                            operation_id,
                            DeliveryStatus::Failed {
                                detail: "Meshtastic operation lost".into(),
                            },
                        ),
                        sennet::instance::InstanceEvent::AckExpired { .. }
                        | sennet::instance::InstanceEvent::AckLost { .. } => {
                            unreachable!("handled above")
                        }
                    };
                    let keep = self.runtime.sennet().awaiting_ack_operation_id() == Some(op);
                    let id = if keep {
                        self.bindings.get(&(SENNET.0, op)).cloned()
                    } else {
                        self.bindings.remove(&(SENNET.0, op))
                    };
                    if let Some(id) = id {
                        self.owner.update(&id, status, vec![])?;
                    }
                }
                Event::TucketAcknowledged(op) => {
                    if let Some(id) = self.bindings.remove(&(TUCKET.0, op.0)) {
                        let timestamp = self
                            .timestamp
                            .get(&op.0)
                            .copied()
                            .ok_or_else(|| backend("ACK operation timestamp missing"))?;
                        let record = self
                            .owner
                            .records()?
                            .into_iter()
                            .find(|r| r.message.id == id)
                            .ok_or(AdapterError::NotFound)?;
                        let expected: Vec<_> = (0..4)
                            .map(|attempt| {
                                let mut text = tucket::message::TextMessage::plain(
                                    timestamp,
                                    record.message.body.text(),
                                );
                                text.attempt = attempt;
                                text.ack_crc(&scope.meshcore_key)
                            })
                            .collect();
                        let facts = acks
                            .iter()
                            .copied()
                            .filter(|value| expected.contains(value))
                            .map(|value| RadioFact::MeshCoreAck { value })
                            .collect();
                        self.owner
                            .update(&id, DeliveryStatus::ProtocolAcknowledged, facts)?;
                        self.timestamp.remove(&op.0);
                    }
                }
                Event::TucketLost(op) => {
                    if let Some(id) = self.bindings.remove(&(TUCKET.0, op.0)) {
                        self.owner.update(
                            &id,
                            DeliveryStatus::Failed {
                                detail: "MeshCore retry operation expired or interrupted".into(),
                            },
                            vec![],
                        )?;
                        self.timestamp.remove(&op.0);
                    }
                }
                Event::ActionsOverflowed(count) => {
                    return self.fault(format!("resident omitted {count} protocol actions"));
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn received(
        &self,
        conversation: ConversationId,
        id: String,
        author: String,
        text: String,
        timestamp_ms: Option<u64>,
        fact: RadioFact,
    ) -> Result<(), AdapterError> {
        self.received_facts(conversation, id, author, text, timestamp_ms, vec![fact])
    }
    fn received_facts(
        &self,
        conversation: ConversationId,
        id: String,
        author: String,
        text: String,
        timestamp_ms: Option<u64>,
        facts: Vec<RadioFact>,
    ) -> Result<(), AdapterError> {
        self.owner.receive(RadioRecord {
            message: Message {
                id: MessageId(id),
                author: Identity::new(conversation.protocol, author),
                body: MessageBody::PlainText(text),
                subject: None,
                timestamp_ms,
                direction: Direction::Incoming,
            },
            conversation,
            delivery: DeliveryStatus::ReceivedByRadio,
            facts,
        })
    }
    fn fault<T>(&self, detail: String) -> Result<T, AdapterError> {
        self.owner.lock()?.fault = Some(detail.clone());
        Err(backend(detail))
    }
}
impl Drop for ResidentSession<'_> {
    fn drop(&mut self) {
        for ((instance, operation), id) in &self.bindings {
            if *instance == SENNET.0
                && self.runtime.sennet().awaiting_ack_operation_id() == Some(*operation)
            {
                let _=self.owner.notice("Meshtastic session ended without a matched protocol ACK; physical completion remains recorded.".into());
                continue;
            }
            if let Err(error) = self.owner.update(
                id,
                DeliveryStatus::Failed {
                    detail:
                        "resident session ended; carriage outcome unknown, explicit resend required"
                            .into(),
                },
                vec![],
            ) && let Ok(mut inner) = self.owner.lock()
            {
                inner.fault = Some(error.to_string());
            }
        }
        if let Ok(mut inner) = self.owner.lock() {
            inner.attached = false;
        }
    }
}

fn work_key(work: WorkId) -> (u8, u64, u64, Option<u64>) {
    (
        work.activation.instance.0,
        work.activation.generation,
        work.sequence,
        work.operation,
    )
}
