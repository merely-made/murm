//! Text messages and saved contacts.

use signalman::message::{MessageEvent, MessageId, MessagePeer, QueuedReason};

use crate::messages::MessageStore;

use super::{DesktopState, parse_address};

impl DesktopState {
    pub fn replace_message_store(&mut self, store: MessageStore) {
        self.next_message_nonce = u64::try_from(store.log_len()).unwrap_or(u64::MAX);
        if self.message_local.is_none() {
            self.message_local = store.known_local_peer();
        }
        self.message_store = store;
        self.message_notice = None;
    }

    pub fn set_message_local(&mut self, local: MessagePeer) {
        self.message_local = Some(local);
        self.message_notice = None;
    }

    pub fn queue_message(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        self.queue_message_at(now);
    }

    pub fn queue_message_at(&mut self, observed_unix_ms: u64) {
        if self.message_protocol != signalman::comms_adapter::comms::ProtocolKind::Lxmf {
            self.queue_radio_message();
            return;
        }
        let Some(sender) = self.message_local else {
            self.message_notice =
                Some("Connect a station identity before queueing a message.".into());
            return;
        };
        let Some(destination) = parse_address(self.message_recipient.text()) else {
            self.message_notice = Some("A recipient is exactly 32 hexadecimal characters.".into());
            return;
        };
        let text = self.message_draft.text().trim();
        if text.is_empty() {
            self.message_notice = Some("Write a message before queueing it.".into());
            return;
        }
        let model = crate::comms::model(
            sender,
            self.message_store.clone(),
            self.station_sender.clone(),
            Some(observed_unix_ms),
        );
        let conversation =
            signalman::comms_adapter::LxmfAdapter::<crate::comms::Mailbox>::conversation_id(
                sender,
                MessagePeer::new(destination, None),
            );
        let mut draft = signalman::comms_adapter::comms::Draft::reply_to(conversation);
        draft.body = text.to_owned();
        match pollster::block_on(model.send(&draft)) {
            Ok(id) => {
                let bytes = hex::decode(id.0).expect("Signalman adapter returns a hex message id");
                self.selected_message = Some(MessageId(
                    bytes.try_into().expect("Signalman message id is 32 bytes"),
                ));
                self.next_message_nonce =
                    u64::try_from(self.message_store.log_len()).unwrap_or(u64::MAX);
                self.message_draft = cambium::TextInput::default();
                self.message_notice =
                    Some("Message persisted and queued for LXMF carriage.".into());
            }
            Err(error) => self.message_notice = Some(format!("Message was not queued: {error}")),
        }
    }

    /// A Comms view over the same journal and attended station handoff.
    pub fn comms(
        &self,
    ) -> Result<signalman::comms_adapter::comms::Comms, signalman::comms_adapter::comms::AdapterError>
    {
        let mut model = match self.message_local {
            Some(local) => crate::comms::model(
                local,
                self.message_store.clone(),
                self.station_sender.clone(),
                None,
            ),
            None => signalman::comms_adapter::comms::Comms::new(),
        };
        for (protocol, owner) in &self.companion_comms {
            owner.add_protocol_to(&mut model, *protocol);
        }
        if let Some(owner) = &self.radio_comms {
            for protocol in [
                signalman::comms_adapter::comms::ProtocolKind::Meshtastic,
                signalman::comms_adapter::comms::ProtocolKind::MeshCore,
            ] {
                if !self
                    .companion_comms
                    .iter()
                    .any(|(kind, _)| *kind == protocol)
                {
                    owner.add_protocol_to(&mut model, protocol);
                }
            }
        }
        if model.identities().is_empty() {
            return Err(signalman::comms_adapter::comms::AdapterError::Unsupported(
                "a station identity or commissioned radio journal is required".into(),
            ));
        }
        Ok(model)
    }

    pub fn set_radio_comms(&mut self, owner: signalman::radio_comms::RadioComms) {
        self.radio_comms = Some(owner);
    }
    pub fn radio_owner(
        &self,
        protocol: signalman::comms_adapter::comms::ProtocolKind,
    ) -> Option<&signalman::radio_comms::RadioComms> {
        self.companion_comms
            .iter()
            .find(|(kind, _)| *kind == protocol)
            .map(|(_, owner)| owner)
            .or_else(|| {
                self.radio_comms
                    .as_ref()
                    .filter(|owner| owner.supports(protocol))
            })
    }
    pub fn apply_companion_event(&mut self, event: crate::companion::CompanionEvent) {
        use crate::companion::CompanionEvent;
        match event {
            CompanionEvent::Connected {
                protocol,
                owner,
                stop,
                port,
                source,
            } => {
                self.companion_comms.retain(|(kind, _)| *kind != protocol);
                self.companion_comms.push((protocol, owner));
                self.companion_stops.retain(|(p, _)| *p != protocol);
                self.companion_stops.push((protocol, stop));
                self.companion_notices.retain(|(p, _)| *p != protocol);
                self.companion_notices
                    .push((protocol, format!("Connected on {port}. {source}")));
            }
            CompanionEvent::Stopped { protocol, detail } => {
                if protocol == signalman::comms_adapter::comms::ProtocolKind::MeshCore {
                    self.resident_connecting = false;
                }
                self.companion_settings
                    .retain(|s| s.protocol.kind() != protocol);
                self.companion_stops.retain(|(p, _)| *p != protocol);
                self.companion_notices.retain(|(p, _)| *p != protocol);
                self.companion_notices.push((protocol, detail));
            }
            CompanionEvent::Progress => (),
        }
    }
    pub fn request_companion_connection(&mut self) {
        use signalman::comms_adapter::comms::ProtocolKind;
        use signalman::radio_comms::companion::CompanionProtocol;
        let result = (|| {
            if self.install_running {
                return Err("Finish the active installation before connecting a radio".to_owned());
            }
            if self.observation_collecting {
                return Err(
                    "Stop the serial observation collector before connecting a companion"
                        .to_owned(),
                );
            }
            if self
                .companion_settings
                .iter()
                .any(|s| s.protocol.kind() == self.message_protocol)
            {
                return Err(
                    "This protocol already has a connecting or connected companion".to_owned(),
                );
            }
            let protocol = match self.message_protocol {
                ProtocolKind::Meshtastic => CompanionProtocol::Meshtastic {
                    channel: self
                        .companion_channel
                        .text()
                        .trim()
                        .parse::<u8>()
                        .map_err(|_| "Use a Meshtastic channel index from 0 to 7")?,
                },
                ProtocolKind::MeshCore => CompanionProtocol::MeshCore,
                _ => {
                    return Err(
                        "Select Meshtastic or MeshCore to connect its stock companion".into(),
                    );
                }
            };
            let settings = crate::companion::CompanionSettings {
                protocol,
                port: self.companion_port.text().trim().into(),
                realm: self.companion_realm.text().trim().into(),
                journal: self.companion_journal.text().trim().into(),
            };
            let mut active = self.companion_settings.clone();
            if self.resident_connecting
                && let Some(resident) = &self.resident_settings
            {
                active.push(crate::companion::CompanionSettings {
                    protocol: CompanionProtocol::MeshCore,
                    port: resident.port.clone(),
                    realm: resident.realm.clone(),
                    journal: resident.journal.clone(),
                });
            }
            active.push(settings.clone());
            let station = std::env::var_os("SIGNALMAN_STATION_PORT").map(std::path::PathBuf::from);
            crate::companion::validate_settings(&active, station.as_deref())?;
            Ok(settings)
        })();
        match result {
            Ok(settings) => {
                // Retire the previous inactive handle so the replacement can
                // acquire the same journal's exclusive file lock on reconnect.
                self.companion_comms
                    .retain(|(kind, _)| *kind != self.message_protocol);
                self.companion_settings.push(settings.clone());
                self.pending_companion = Some(settings);
                self.message_notice =
                    Some("Connecting stock companion; waiting for its public identity.".into());
            }
            Err(error) => {
                self.message_notice = Some(format!("Companion connection refused: {error}"))
            }
        }
    }
    pub fn take_companion_request(&mut self) -> Option<crate::companion::CompanionSettings> {
        self.pending_companion.take()
    }
    pub fn request_resident_connection(&mut self) {
        use signalman::comms_adapter::comms::ProtocolKind;
        let result = (|| {
            if self.message_protocol != ProtocolKind::MeshCore {
                return Err("Select MeshCore for the Tucket connection".to_owned());
            }
            if self.install_running || self.observation_collecting || self.resident_connecting {
                return Err("Stop the active radio owner before connecting Tucket".into());
            }
            let mut settings = self.resident_settings.clone().ok_or_else(|| {
                "A scoped Tucket authority and radio profile must be configured first".to_owned()
            })?;
            settings.port = self.companion_port.text().trim().into();
            settings.realm = self.companion_realm.text().trim().into();
            settings.journal = self.companion_journal.text().trim().into();
            crate::resident::validate(&settings)?;
            let mut active = self.companion_settings.clone();
            active.push(crate::companion::CompanionSettings {
                protocol: signalman::radio_comms::companion::CompanionProtocol::MeshCore,
                port: settings.port.clone(),
                realm: settings.realm.clone(),
                journal: settings.journal.clone(),
            });
            crate::companion::validate_settings(
                &active,
                std::env::var_os("SIGNALMAN_STATION_PORT")
                    .as_deref()
                    .map(std::path::Path::new),
            )?;
            Ok(settings)
        })();
        match result {
            Ok(settings) => {
                self.companion_comms
                    .retain(|(kind, _)| *kind != ProtocolKind::MeshCore);
                self.resident_settings = Some(settings);
                self.resident_connecting = true;
                self.pending_resident = true;
                self.message_notice =
                    Some("Connecting Tucket through Tulle; checking scoped identity first.".into());
            }
            Err(error) => self.message_notice = Some(format!("Tucket connection refused: {error}")),
        }
    }
    pub fn take_resident_request(&mut self) -> Option<crate::resident::ResidentSettings> {
        if std::mem::take(&mut self.pending_resident) {
            self.resident_settings.clone()
        } else {
            None
        }
    }
    pub fn disconnect_companion(&mut self) {
        for (protocol, stop) in &self.companion_stops {
            if *protocol == self.message_protocol {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    pub fn select_message_protocol(
        &mut self,
        protocol: signalman::comms_adapter::comms::ProtocolKind,
    ) {
        use signalman::comms_adapter::comms::ProtocolKind;
        if !matches!(
            protocol,
            ProtocolKind::Lxmf | ProtocolKind::Meshtastic | ProtocolKind::MeshCore
        ) {
            self.message_notice = Some("Unsupported message protocol".into());
            return;
        }
        self.message_protocol = protocol;
        self.message_notice = None;
    }
    fn queue_radio_message(&mut self) {
        use signalman::comms_adapter::comms::{Draft, ProtocolKind};
        let result = (|| {
            let owner = self
                .radio_owner(self.message_protocol)
                .ok_or_else(|| "a commissioned radio journal is required".to_owned())?;
            let peer = self.message_recipient.text().trim().trim_start_matches('!');
            let length = if self.message_protocol == ProtocolKind::Meshtastic {
                8
            } else {
                64
            };
            if peer.len() != length {
                return Err(format!("use an exact {length}-digit recipient address"));
            }
            let target = owner
                .conversation(self.message_protocol, &peer.to_ascii_lowercase())
                .map_err(|e| e.to_string())?;
            let mut draft = Draft::reply_to(target);
            draft.body = self.message_draft.text().to_owned();
            pollster::block_on(self.comms().map_err(|e| e.to_string())?.send(&draft))
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(_) => {
                self.message_draft = cambium::TextInput::default();
                self.message_notice = Some("Text persisted. The connected companion or authorized resident owner carries queued intents.".into());
            }
            Err(error) => self.message_notice = Some(format!("Message was not queued: {error}")),
        }
    }

    pub fn apply_message_event(&mut self, event: MessageEvent) -> bool {
        if let Err(error) = self.message_store.append(event) {
            self.message_notice = Some(format!("Message event was not persisted: {error}"));
            return false;
        }
        true
    }

    pub fn comms_diagnostics(
        &self,
    ) -> Result<
        signalman::comms_adapter::LxmfDiagnostics,
        signalman::comms_adapter::comms::AdapterError,
    > {
        let local = self.message_local.ok_or_else(|| {
            signalman::comms_adapter::comms::AdapterError::Unsupported(
                "Connect a station identity to inspect its LXMF queue.".into(),
            )
        })?;
        signalman::comms_adapter::LxmfAdapter::new(
            local,
            crate::comms::Mailbox {
                store: self.message_store.clone(),
                station: self.station_sender.clone(),
                now_ms: None,
            },
        )
        .diagnostics()
    }

    /// Resume durable intents after station progress. Waiting-for-peer work
    /// retries only after a matching peer observation (or a fresh connection).
    pub fn dispatch_queued_messages(&mut self, peer: Option<MessagePeer>) {
        let (Some(sender), Some(local)) = (&self.station_sender, self.message_local) else {
            return;
        };
        for record in self.message_store.book().iter() {
            if record.message.sender() != local {
                continue;
            }
            let eligible = match record.status {
                signalman::message::MessageStatus::Queued(
                    QueuedReason::Offline | QueuedReason::ReadyForCarriage,
                ) => true,
                signalman::message::MessageStatus::Queued(QueuedReason::WaitingForPeer) => peer
                    .is_some_and(|peer| {
                        peer == local || peer.destination == record.message.recipient().destination
                    }),
                _ => false,
            };
            if eligible && let Err(error) = sender.submit(record.message.clone()) {
                self.message_notice = Some(error);
                break;
            }
        }
    }

    pub fn select_message(&mut self, id: MessageId) {
        self.selected_message = Some(id);
        self.message_notice = None;
    }

    pub fn save_selected_sender(&mut self) {
        let Some(id) = self.selected_message else {
            self.message_notice = Some("Select an incoming message first.".into());
            return;
        };
        let Some(peer) = self
            .message_store
            .book()
            .get(id)
            .map(|record| record.message.sender())
        else {
            self.message_notice = Some("The selected message is no longer present.".into());
            return;
        };
        let petname = self.message_contact_name.text().trim();
        if petname.is_empty() {
            self.message_notice = Some("Give this contact your own name first.".into());
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        match self.message_store.save_contact(peer, petname, now) {
            Ok(()) => {
                self.message_contact_name = cambium::TextInput::default();
                self.message_notice = Some("Contact saved.".into());
            }
            Err(error) => self.message_notice = Some(format!("Contact was not saved: {error}")),
        }
    }
}
