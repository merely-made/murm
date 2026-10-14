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
        if let Some(owner) = &self.radio_comms {
            owner.add_to(&mut model);
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
    pub fn select_message_protocol(
        &mut self,
        protocol: signalman::comms_adapter::comms::ProtocolKind,
    ) {
        use signalman::comms_adapter::comms::ProtocolKind;
        if !matches!(
            protocol,
            ProtocolKind::Lxmf | ProtocolKind::Meshtastic | ProtocolKind::MeshCore
        ) || (protocol != ProtocolKind::Lxmf && self.radio_comms.is_none())
        {
            self.message_notice =
                Some("Load a commissioned radio journal before selecting its protocol.".into());
            return;
        }
        self.message_protocol = protocol;
        self.message_notice = None;
    }
    fn queue_radio_message(&mut self) {
        use signalman::comms_adapter::comms::{Draft, ProtocolKind};
        let result = (|| {
            let owner = self
                .radio_comms
                .as_ref()
                .ok_or_else(|| "a commissioned radio journal is required".to_owned())?;
            let scope = owner.scope().map_err(|e| e.to_string())?;
            let peer = self.message_recipient.text().trim().trim_start_matches('!');
            let target = match self.message_protocol {
                ProtocolKind::Meshtastic if peer.len() == 8 => scope.meshtastic(
                    u32::from_str_radix(peer, 16)
                        .map_err(|_| "use an exact 8-digit Meshtastic node number".to_owned())?,
                ),
                ProtocolKind::MeshCore if peer.len() == 64 => scope.meshcore(
                    hex::decode(peer)
                        .map_err(|_| "use an exact 64-digit MeshCore public key".to_owned())?
                        .try_into()
                        .map_err(|_| "invalid MeshCore public key".to_owned())?,
                ),
                _ => return Err(
                    "use an exact 8-digit Meshtastic node number or 64-digit MeshCore public key"
                        .to_owned(),
                ),
            };
            let mut draft = Draft::reply_to(target);
            draft.body = self.message_draft.text().to_owned();
            pollster::block_on(self.comms().map_err(|e| e.to_string())?.send(&draft))
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(_) => {
                self.message_draft = cambium::TextInput::default();
                self.message_notice = Some("Text persisted. The authorized resident owner carries queued intents when this protocol is active.".into());
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
