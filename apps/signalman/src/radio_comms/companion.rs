//! Attended stock companion connection, over host-supplied exclusive byte I/O.
//!
//! Device firmware owns its keys and radio execution. The public journal owns
//! application intents and observed facts. This client neither commissions nor
//! flashes a device. Dropping the session retires all unresolved commands; it
//! never retries a command whose serial write may have reached the firmware.
mod frame;
mod meshtastic;
#[cfg(test)]
mod tests;

use super::*;
use frame::{Frames, invalid};
use meshtastic::{nested, uint};
use std::{collections::BTreeMap, io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::{Instant, timeout},
};

/// Open a stock companion's serial port at the companions' 115200 baud.
///
/// Windows releases a port's handle only once its pending overlapped read has been canceled,
/// so a reconnect straight after a disconnect can be refused for a moment. That refusal is
/// retried briefly; every other error, and a port another program holds, still fails.
pub async fn open_port(path: impl AsRef<std::path::Path>) -> io::Result<serial2_tokio::SerialPort> {
    const ATTEMPTS: u32 = 20;
    let mut attempt = 1;
    loop {
        match serial2_tokio::SerialPort::open(path.as_ref(), 115_200) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < ATTEMPTS => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            result => return result,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompanionProtocol {
    Meshtastic { channel: u8 },
    MeshCore,
}
impl CompanionProtocol {
    pub fn kind(self) -> ProtocolKind {
        match self {
            Self::Meshtastic { .. } => ProtocolKind::Meshtastic,
            Self::MeshCore => ProtocolKind::MeshCore,
        }
    }
}
struct Pending {
    id: MessageId,
    peer: String,
    deadline: Instant,
}
/// One exclusive serial owner. Poll continuously, including while no drafts are queued.
/// Each poll has a bounded receive wait; the host must stop on any returned error.
pub struct CompanionSession<T: AsyncRead + AsyncWrite + Unpin> {
    frames: Frames<T>,
    owner: RadioComms,
    protocol: CompanionProtocol,
    contacts: Vec<[u8; 32]>,
    pending: BTreeMap<u32, Pending>,
    meshcore_sync: bool,
    fault: bool,
}
impl<T: AsyncRead + AsyncWrite + Unpin> CompanionSession<T> {
    /// Handshake before exposing an adapter. An existing journal must match the
    /// actual public identity, realm and channel. Channel secrets are discarded.
    pub async fn connect(
        io: T,
        protocol: CompanionProtocol,
        realm: String,
        journal: impl RadioJournal + 'static,
    ) -> io::Result<Self> {
        let mut frames = Frames::new(io, protocol == CompanionProtocol::MeshCore);
        let mut startup_packets = Vec::new();
        let mut device_summary = None;
        let scope = timeout(Duration::from_secs(10), async {
            match protocol {
                CompanionProtocol::Meshtastic { channel } => {
                    if channel > 7 {
                        return Err(invalid("Meshtastic channel index must be 0..7"));
                    }
                    let nonce = random_id()?;
                    // Request a fresh configuration transaction on the stock stream.
                    frames.send(&meshtastic::configure(nonce)).await?;
                    let mut node = None;
                    let mut enabled_channel = false;
                    for _ in 0..4096 {
                        let frame = frames.next().await?;
                        if nested(&frame, 2)?.is_some() {
                            if startup_packets.len() == 16 {
                                return Err(invalid("Meshtastic startup packet limit"));
                            }
                            startup_packets.push(frame.clone());
                        }
                        if let Some(metadata) = nested(&frame, 13)?
                            && let Some(version) = nested(metadata, 1)?
                        {
                            device_summary = Some(format!(
                                "Meshtastic firmware {}",
                                std::str::from_utf8(version).map_err(as_io)?
                            ));
                        }
                        if let Some(info) = nested(&frame, 3)? {
                            let value = uint(info, 1)?;
                            if value == 0 || value == u32::MAX || node.is_some_and(|n| n != value) {
                                return Err(invalid("invalid or conflicting Meshtastic identity"));
                            }
                            node = Some(value);
                        }
                        if let Some(info) = nested(&frame, 10)?
                            && uint(info, 1)? == u32::from(channel)
                            && uint(info, 3)? != 0
                        {
                            enabled_channel = true;
                        }
                        if uint(&frame, 8)? != 0 {
                            return Err(invalid("Meshtastic rebooted during handshake"));
                        }
                        if uint(&frame, 7)? == nonce {
                            if !enabled_channel {
                                return Err(invalid(
                                    "selected Meshtastic channel is absent or disabled",
                                ));
                            }
                            return Ok(CompanionScope::Meshtastic {
                                realm,
                                node: node.ok_or_else(|| {
                                    invalid("Meshtastic config omitted local identity")
                                })?,
                                channel,
                            });
                        }
                    }
                    Err(invalid("Meshtastic configuration response limit"))
                }
                CompanionProtocol::MeshCore => {
                    frames
                        .send(&[
                            1, 0, 0, 0, 0, 0, 0, 0, b'S', b'i', b'g', b'n', b'a', b'l', b'm', b'a',
                            b'n',
                        ])
                        .await?;
                    let frame = reply(&mut frames, 5).await?;
                    let key: [u8; 32] = frame
                        .get(4..36)
                        .ok_or_else(|| invalid("truncated MeshCore self-info"))?
                        .try_into()
                        .unwrap();
                    if key == [0; 32] {
                        return Err(invalid("empty MeshCore identity"));
                    }
                    frames.send(&[22, 3]).await?;
                    let info = reply(&mut frames, 13).await?;
                    let device = tucket::companion::DeviceInfo::decode(&info)
                        .ok_or_else(|| invalid("invalid MeshCore device-info"))?;
                    device_summary = Some(format!(
                        "MeshCore firmware {}, model {}, build {}, companion API {}",
                        device.version, device.model, device.build, device.protocol_version
                    ));
                    if device.protocol_version < 3 {
                        return Err(invalid("MeshCore companion API requires version >=3"));
                    }
                    Ok(CompanionScope::MeshCore { realm, key })
                }
            }
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "companion identity handshake timed out",
            )
        })??;
        let owner = RadioComms::open_companion(scope, journal).map_err(as_io)?;
        {
            let mut inner = owner.lock().map_err(as_io)?;
            inner.attached = true;
        }
        let mut session = Self {
            frames,
            owner,
            protocol,
            contacts: vec![],
            pending: BTreeMap::new(),
            meshcore_sync: protocol == CompanionProtocol::MeshCore,
            fault: false,
        };
        if protocol == CompanionProtocol::MeshCore {
            session.refresh_contacts().await?;
        }
        for packet in startup_packets {
            session.observe(&packet)?;
        }
        if let Some(summary) = device_summary {
            session.owner.notice(summary).map_err(as_io)?;
        }
        session.owner.notice(format!("Connected stock {:?} companion; public identity read from device. Device acceptance and protocol ACKs are not authenticated/read receipts.", protocol.kind())).map_err(as_io)?;
        Ok(session)
    }
    pub fn owner(&self) -> RadioComms {
        self.owner.clone()
    }
    async fn refresh_contacts(&mut self) -> io::Result<()> {
        self.frames.send(&[4]).await?;
        timeout(Duration::from_secs(5), async {
            let mut contacts = vec![];
            let mut started = false;
            for _ in 0..2048 {
                let frame = self.frames.next().await?;
                match frame[0] {
                    2 if !started => {
                        started = true;
                        if frame.len() != 5 || le(&frame[1..5]) > 1024 {
                            return Err(invalid("MeshCore contact list bounds"));
                        }
                    }
                    3 if started => {
                        if frame.len() < 148 {
                            return Err(invalid("truncated MeshCore contact record"));
                        }
                        if contacts.len() == 1024 {
                            return Err(invalid("MeshCore contact list full"));
                        }
                        let key = frame
                            .get(1..33)
                            .ok_or_else(|| invalid("truncated MeshCore contact"))?
                            .try_into()
                            .unwrap();
                        if !contacts.contains(&key) {
                            contacts.push(key);
                        }
                    }
                    4 if started && frame.len() == 5 => {
                        self.contacts = contacts;
                        return Ok(());
                    }
                    code if code >= 0x80 => {
                        self.meshcore_push(&frame)?;
                    }
                    _ => return Err(invalid("unexpected MeshCore contact-list reply")),
                }
            }
            Err(invalid("MeshCore contact-list frame limit"))
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MeshCore contacts timed out"))?
    }
    /// Receive a frame, expire pending observations, and dispatch at most one
    /// persisted intent. Receive timeouts preserve partial framing state.
    pub async fn poll(&mut self) -> io::Result<bool> {
        if self.fault || self.owner.lock().map_err(as_io)?.fault.is_some() {
            return Err(invalid("companion journal/session failed; stop connection"));
        }
        let result = self.poll_inner().await;
        if result.is_err() {
            self.fault = true;
        }
        result
    }
    async fn poll_inner(&mut self) -> io::Result<bool> {
        let mut changed = false;
        match timeout(Duration::from_millis(100), self.frames.next()).await {
            Ok(Ok(frame)) => {
                self.observe(&frame)?;
                changed = true;
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => (),
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= Instant::now())
            .map(|(key, _)| *key)
            .collect();
        for key in expired {
            let p = self.pending.remove(&key).unwrap();
            // A missing recipient ACK does not undo observed device acceptance.
            self.owner
                .notice(format!(
                    "{}: acknowledgement window ended; no recipient acknowledgement observed",
                    p.id.0
                ))
                .map_err(as_io)?;
            if self
                .owner
                .records()
                .map_err(as_io)?
                .iter()
                .any(|r| r.message.id == p.id && r.delivery == DeliveryStatus::Unknown)
            {
                self.owner
                    .update(
                        &p.id,
                        DeliveryStatus::Failed {
                            detail: "device queue outcome unknown; explicit resend required".into(),
                        },
                        vec![],
                    )
                    .map_err(as_io)?;
            }
            changed = true;
        }
        if self.pending.len() < 16 {
            changed |= self.dispatch().await?;
        }
        if self.protocol == CompanionProtocol::MeshCore && self.meshcore_sync {
            self.meshcore_sync = false;
            self.frames.send(&[10]).await?;
            // Only one command in flight on this request/response protocol.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let frame = tokio::time::timeout_at(deadline, self.frames.next())
                    .await
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::TimedOut, "MeshCore sync timed out")
                    })??;
                if frame[0] >= 0x80 {
                    self.meshcore_push(&frame)?;
                    continue;
                }
                self.observe(&frame)?;
                if frame[0] != 10 {
                    self.meshcore_sync = true;
                }
                break;
            }
            changed = true;
        }
        Ok(changed)
    }
    fn peer_key(&self, prefix: &[u8]) -> io::Result<[u8; 32]> {
        let matches: Vec<_> = self
            .contacts
            .iter()
            .filter(|key| key.starts_with(prefix))
            .collect();
        if matches.len() != 1 {
            return Err(invalid(
                "MeshCore contact prefix is unknown or ambiguous; full key required",
            ));
        }
        Ok(*matches[0])
    }
    async fn dispatch(&mut self) -> io::Result<bool> {
        let Some(record) = self.owner.records().map_err(as_io)?.into_iter().find(|r| {
            r.message.direction == Direction::Outgoing
                && r.delivery == DeliveryStatus::Queued(DeliveryQueueReason::Offline)
        }) else {
            return Ok(false);
        };
        let peer = self
            .owner
            .lock()
            .map_err(as_io)?
            .book
            .scope
            .peer(&record.conversation)
            .map_err(as_io)?;
        let text = record.message.body.text();
        let (payload, correlation, fact) = match self.protocol {
            CompanionProtocol::Meshtastic { channel } => {
                let destination = u32::from_str_radix(&peer, 16).map_err(as_io)?;
                let packet_id = random_id()?;
                if self
                    .owner
                    .records()
                    .map_err(as_io)?
                    .iter()
                    .any(|r| r.facts.contains(&RadioFact::CompanionRequest { packet_id }))
                {
                    return Err(invalid("companion packet ID collision"));
                }
                (
                    meshtastic::text(destination, channel, packet_id, text),
                    packet_id,
                    RadioFact::CompanionRequest { packet_id },
                )
            }
            CompanionProtocol::MeshCore => {
                self.refresh_contacts().await?;
                let key: [u8; 32] = hex::decode(&peer)
                    .map_err(as_io)?
                    .try_into()
                    .map_err(|_| invalid("invalid MeshCore peer key"))?;
                let resolved = self.peer_key(&key[..6]);
                if !matches!(resolved, Ok(resolved) if resolved == key) {
                    self.owner.update(&record.message.id, DeliveryStatus::Failed { detail: "MeshCore peer is absent or has an ambiguous six-byte contact prefix".into() }, vec![]).map_err(as_io)?;
                    return Ok(true);
                }
                let now = unix_seconds()?;
                let mut timestamp = now;
                for r in self.owner.records().map_err(as_io)? {
                    for fact in r.facts {
                        if let RadioFact::MeshCoreOperation {
                            timestamp: previous,
                        } = fact
                        {
                            timestamp = timestamp.max(
                                previous
                                    .checked_add(1)
                                    .ok_or_else(|| invalid("MeshCore timestamp exhausted"))?,
                            );
                        }
                    }
                }
                let mut payload = vec![2, 0, 0];
                payload.extend_from_slice(&timestamp.to_le_bytes());
                payload.extend_from_slice(&key[..6]);
                payload.extend_from_slice(text.as_bytes());
                (payload, 0, RadioFact::MeshCoreOperation { timestamp })
            }
        };
        // Publish the uncertain/dispatched marker BEFORE touching the byte link.
        self.owner
            .update(&record.message.id, DeliveryStatus::Unknown, vec![fact])
            .map_err(as_io)?;
        if self.protocol == CompanionProtocol::MeshCore {
            self.frames.send(&payload).await?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut early_confirmations = Vec::new();
            loop {
                let frame = tokio::time::timeout_at(deadline, self.frames.next())
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "MeshCore send reply timed out; outcome uncertain",
                        )
                    })??;
                if frame[0] >= 0x80 {
                    if frame[0] == 0x82 {
                        if early_confirmations.len() == 16 {
                            return Err(invalid("early MeshCore confirmation limit"));
                        }
                        early_confirmations.push(frame.clone());
                    }
                    self.meshcore_push(&frame)?;
                    continue;
                }
                if frame[0] == 1 {
                    self.owner
                        .update(
                            &record.message.id,
                            DeliveryStatus::Failed {
                                detail: format!(
                                    "MeshCore rejected command, error {}",
                                    frame.get(1).copied().unwrap_or(0)
                                ),
                            },
                            vec![],
                        )
                        .map_err(as_io)?;
                    return Ok(true);
                }
                if frame[0] != 6 || frame.len() != 10 {
                    return Err(invalid("unexpected MeshCore send reply"));
                }
                let ack = le(&frame[2..6]);
                if ack != 0 && self.pending.contains_key(&ack) {
                    return Err(invalid("ambiguous MeshCore acknowledgement correlation"));
                }
                self.owner
                    .update(
                        &record.message.id,
                        DeliveryStatus::HandedToRadio,
                        vec![RadioFact::CompanionAccepted { correlation: ack }],
                    )
                    .map_err(as_io)?;
                if ack != 0 {
                    self.pending.insert(
                        ack,
                        Pending {
                            id: record.message.id,
                            peer,
                            deadline: Instant::now()
                                + Duration::from_millis(
                                    u64::from(le(&frame[6..10])).clamp(1000, 120_000),
                                ),
                        },
                    );
                }
                for confirmation in early_confirmations {
                    self.meshcore_push(&confirmation)?;
                }
                return Ok(true);
            }
        }
        self.pending.insert(
            correlation,
            Pending {
                id: record.message.id,
                peer,
                deadline: Instant::now() + Duration::from_secs(120),
            },
        );
        self.frames.send(&payload).await?;
        Ok(true)
    }
    fn observe(&mut self, frame: &[u8]) -> io::Result<()> {
        match self.protocol {
            CompanionProtocol::Meshtastic { channel } => self.meshtastic_observe(frame, channel),
            CompanionProtocol::MeshCore => {
                match frame[0] {
                    code if code >= 0x80 => self.meshcore_push(frame)?,
                    10 => (),
                    7 | 16 => {
                        let offset = if frame[0] == 16 { 4 } else { 1 };
                        let prefix = frame.get(offset..offset+6).ok_or_else(|| invalid("truncated MeshCore text"))?;
                        let Some(&kind) = frame.get(offset+7) else { return Err(invalid("truncated MeshCore text type")); };
                        if kind != 0 { self.owner.notice(format!("MeshCore text type {kind} is outside the plain-text profile")).map_err(as_io)?; return Ok(()); }
                        let key = match self.peer_key(prefix) {
                            Ok(key) => key,
                            Err(error) => { self.owner.notice(error.to_string()).map_err(as_io)?; return Ok(()); }
                        };
                        let timestamp = le(frame.get(offset+8..offset+12).ok_or_else(|| invalid("truncated MeshCore timestamp"))?);
                        let text = std::str::from_utf8(&frame[offset+12..]).map_err(as_io)?;
                        self.receive(hex::encode(key), timestamp, text, format!("mc-{}-{timestamp}-{}", hex::encode(key), digest(text.as_bytes())))?;
                    }
                    8 | 17 | 27 => self.owner.notice("MeshCore channel/data message is outside the full-contact text profile".into()).map_err(as_io)?,
                    1 => return Err(invalid("MeshCore command failed")),
                    _ => return Err(invalid("unsolicited MeshCore command response")),
                }
                Ok(())
            }
        }
    }
    fn meshcore_push(&mut self, frame: &[u8]) -> io::Result<()> {
        match frame[0] {
            0x82 => {
                if frame.len() != 9 {
                    return Err(invalid("truncated MeshCore confirmation"));
                }
                let key = le(&frame[1..5]);
                if let Some(p) = self.pending.remove(&key) {
                    self.owner
                        .update(
                            &p.id,
                            DeliveryStatus::ProtocolAcknowledged,
                            vec![RadioFact::MeshCoreAck {
                                value: frame[1..5].try_into().unwrap(),
                            }],
                        )
                        .map_err(as_io)?;
                }
            }
            0x83 => self.meshcore_sync = true,
            _ => (), // RF logs, advert/path changes carry no delivery assertion.
        }
        Ok(())
    }
    fn meshtastic_observe(&mut self, frame: &[u8], channel: u8) -> io::Result<()> {
        if uint(frame, 8)? != 0 {
            return Err(invalid("Meshtastic rebooted; reconnect required"));
        }
        if nested(frame, 10)?.is_some() || nested(frame, 5)?.is_some() {
            return Err(invalid(
                "Meshtastic configuration changed; reconnect with the appropriate realm",
            ));
        }
        if let Some(info) = nested(frame, 3)? {
            let local = self
                .owner
                .lock()
                .map_err(as_io)?
                .book
                .scope
                .local(ProtocolKind::Meshtastic);
            if format!("{:08x}", uint(info, 1)?) != local {
                return Err(invalid("Meshtastic identity changed"));
            }
        }
        if let Some(queue) = nested(frame, 11)? {
            let correlation = uint(queue, 4)?;
            if let Some(p) = self.pending.get(&correlation) {
                let error = meshtastic::signed(queue, 1)?;
                self.owner
                    .update(
                        &p.id,
                        if error == 0 {
                            DeliveryStatus::HandedToRadio
                        } else {
                            DeliveryStatus::Failed {
                                detail: format!("Meshtastic queue rejected packet ({error})"),
                            }
                        },
                        vec![RadioFact::CompanionAccepted { correlation }],
                    )
                    .map_err(as_io)?;
                if error != 0 || p.peer == "ffffffff" {
                    self.pending.remove(&correlation);
                }
            }
        }
        let Some(packet) = nested(frame, 2)? else {
            return Ok(());
        };
        let Some(data) = nested(packet, 4)? else {
            return Ok(());
        };
        let source = uint(packet, 1)?;
        let destination = uint(packet, 2)?;
        let packet_id = uint(packet, 6)?;
        let local = self
            .owner
            .lock()
            .map_err(as_io)?
            .book
            .scope
            .local(ProtocolKind::Meshtastic);
        if uint(data, 1)? == 5 {
            let request = uint(data, 6)?;
            let routing = nested(data, 2)?.unwrap_or_default();
            if !meshtastic::fields(routing)?
                .iter()
                .any(|field| field.number == 3)
            {
                return Ok(());
            }
            let error = uint(routing, 3)?;
            if let Some(p) = self.pending.get(&request) {
                // A local routing success can be an implicit relay ACK. Only a
                // matched remote sender earns ProtocolAcknowledged; no proof is verified.
                if destination != u32::from_str_radix(&local, 16).map_err(as_io)? {
                    return Ok(());
                }
                let remote = format!("{source:08x}") == p.peer;
                if remote || (format!("{source:08x}") == local && error != 0) {
                    self.owner
                        .update(
                            &p.id,
                            if error == 0 {
                                DeliveryStatus::ProtocolAcknowledged
                            } else {
                                DeliveryStatus::Failed {
                                    detail: format!("Meshtastic routing error {error}"),
                                }
                            },
                            vec![RadioFact::CompanionPacket {
                                source,
                                destination,
                                packet_id,
                                channel_index: u8::try_from(uint(packet, 3)?).map_err(as_io)?,
                                pki_reported: uint(packet, 17)? != 0,
                            }],
                        )
                        .map_err(as_io)?;
                    self.pending.remove(&request);
                }
            }
        } else if uint(data, 1)? == 1
            && uint(packet, 3)? == u32::from(channel)
            && format!("{source:08x}") != local
        {
            if source == 0 || (destination != u32::MAX && format!("{destination:08x}") != local) {
                return Ok(());
            }
            let text = std::str::from_utf8(nested(data, 2)?.unwrap_or_default()).map_err(as_io)?;
            let peer = if destination == u32::MAX {
                "ffffffff".to_owned()
            } else {
                format!("{source:08x}")
            };
            self.receive_from(
                peer,
                format!("{source:08x}"),
                uint(packet, 7)?,
                text,
                format!("mt-{source:08x}-{packet_id:08x}"),
                vec![RadioFact::CompanionPacket {
                    source,
                    destination,
                    packet_id,
                    channel_index: u8::try_from(uint(packet, 3)?).map_err(as_io)?,
                    pki_reported: uint(packet, 17)? != 0,
                }],
            )?;
        }
        Ok(())
    }
    fn receive(&self, peer: String, timestamp: u32, text: &str, id: String) -> io::Result<()> {
        self.receive_from(peer.clone(), peer, timestamp, text, id, vec![])
    }
    fn receive_from(
        &self,
        peer: String,
        source: String,
        timestamp: u32,
        text: &str,
        id: String,
        mut facts: Vec<RadioFact>,
    ) -> io::Result<()> {
        if text.is_empty() || text.len() > 232 || text.contains('\0') {
            return Err(invalid("invalid companion text"));
        }
        let protocol = self.protocol.kind();
        facts.push(RadioFact::CompanionReceived {
            protocol,
            source: source.clone(),
            timestamp,
        });
        self.owner
            .receive(RadioRecord {
                conversation: self.owner.conversation(protocol, &peer).map_err(as_io)?,
                message: Message {
                    id: MessageId(id),
                    author: Identity::new(protocol, source.clone()),
                    body: MessageBody::PlainText(text.into()),
                    subject: None,
                    timestamp_ms: (timestamp != 0).then_some(u64::from(timestamp) * 1000),
                    direction: Direction::Incoming,
                },
                delivery: DeliveryStatus::ReceivedByRadio,
                facts,
            })
            .map_err(as_io)
    }
}
impl<T: AsyncRead + AsyncWrite + Unpin> Drop for CompanionSession<T> {
    fn drop(&mut self) {
        // Include a command interrupted before its correlation was returned.
        if let Ok(records) = self.owner.records() {
            for r in records {
                if r.message.direction == Direction::Outgoing
                    && r.delivery == DeliveryStatus::Unknown
                {
                    let _ = self.owner.update(&r.message.id, DeliveryStatus::Failed { detail: "companion disconnected with an uncertain command; explicit resend required".into() }, vec![]);
                }
            }
        }
        if let Ok(mut inner) = self.owner.lock() {
            inner.attached = false;
        }
    }
}
async fn reply<T: AsyncRead + AsyncWrite + Unpin>(
    frames: &mut Frames<T>,
    expected: u8,
) -> io::Result<Vec<u8>> {
    for _ in 0..128 {
        let frame = frames.next().await?;
        if frame[0] == expected {
            return Ok(frame);
        }
        if frame[0] < 0x80 {
            return Err(invalid("unexpected companion handshake reply"));
        }
    }
    Err(invalid("companion handshake push limit"))
}
fn le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("validated four-byte range"))
}
fn as_io(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
fn random_id() -> io::Result<u32> {
    loop {
        let value = getrandom::u32().map_err(as_io)?;
        if value != 0 {
            return Ok(value);
        }
    }
}
fn unix_seconds() -> io::Result<u32> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(as_io)?
        .as_secs()
        .try_into()
        .map_err(as_io)
}
fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}
