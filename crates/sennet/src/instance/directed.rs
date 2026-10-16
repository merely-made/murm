//! One bounded pinned peer, one response obligation and the shared physical slot.
//! A busy physical slot refuses an ACK-requesting text before dedup/delivery;
//! a later sender retry can be admitted. Control responses never request ACKs.
use super::state::PendingTx;
use super::{DirectedBinding, InstanceError, InstanceEvent, ReceiveOutcome, SennetInstance};
use crate::{
    application::{ApplicationEnvelope, TEXT_PORT},
    node::{NodeError, ReceivedText},
    packet_id::PacketIdentity,
    pki::{LocalIdentity, PeerSession},
    routing,
    transport::{BROADCAST_DESTINATION, Header, Packet},
};
use alloc::{borrow::ToOwned, collections::VecDeque, vec::Vec};

struct Waiting {
    operation_id: u32,
    identity: PacketIdentity,
    expires_at: Option<u64>,
}
pub(super) struct Directed {
    binding: DirectedBinding,
    cipher: PeerSession,
    timeout: u64,
    waiting: Option<Waiting>,
    seen: VecDeque<u32>,
    capacity: usize,
}
impl Directed {
    pub(super) fn deadline(&self) -> Option<u64> {
        self.waiting.as_ref().and_then(|w| w.expires_at)
    }
    pub(super) fn has_pending(&self) -> bool {
        self.waiting.is_some()
    }
    pub(super) fn cancel(&mut self, operation: u32) {
        if self
            .waiting
            .as_ref()
            .is_some_and(|w| w.operation_id == operation)
        {
            self.waiting = None;
        }
    }
    pub(super) fn completed(&mut self, operation: u32, now: u64, success: bool) {
        if let Some(w) = &mut self.waiting
            && w.operation_id == operation
        {
            if success {
                w.expires_at = Some(now.saturating_add(self.timeout));
            } else {
                self.waiting = None;
            }
        }
    }
    pub(super) fn expire(&mut self, now: u64) -> Option<InstanceEvent> {
        if self.deadline().is_some_and(|end| now >= end) {
            self.waiting.take().map(|w| InstanceEvent::AckExpired {
                operation_id: w.operation_id,
                identity: w.identity,
            })
        } else {
            None
        }
    }
    pub(super) fn lose(&mut self) -> Option<InstanceEvent> {
        self.waiting.take().map(|w| InstanceEvent::AckLost {
            operation_id: w.operation_id,
            identity: w.identity,
        })
    }
}
impl SennetInstance {
    /// Install one explicit public peer binding. No identity discovery,
    /// replacement, channel-zero ambiguity or hidden default response window.
    pub fn install_pki(
        &mut self,
        peer: u32,
        identity: LocalIdentity,
        public: [u8; 32],
        ack_timeout: u64,
        seen_capacity: usize,
    ) -> Result<(), InstanceError> {
        self.require_active()?;
        if self.directed.is_some()
            || self.pending.is_some()
            || ack_timeout == 0
            || seen_capacity == 0
            || self.channel.hash == 0
            || self.packet_ids.next_packet_id() == 0
        {
            return Err(InstanceError::InvalidDirectedConfig);
        }
        let local_public = identity.public_key();
        let cipher = identity
            .peer(self.packet_ids.source(), peer, public)
            .map_err(InstanceError::Pki)?;
        self.directed = Some(Directed {
            binding: DirectedBinding {
                peer,
                local_public,
                peer_public: public,
            },
            cipher,
            timeout: ack_timeout,
            waiting: None,
            seen: VecDeque::new(),
            capacity: seen_capacity,
        });
        Ok(())
    }
    pub fn directed_binding(&self) -> Option<DirectedBinding> {
        self.directed.as_ref().map(|d| d.binding)
    }
    pub fn awaiting_ack_operation_id(&self) -> Option<u32> {
        self.directed
            .as_ref()
            .and_then(|d| d.waiting.as_ref())
            .filter(|w| w.expires_at.is_some())
            .map(|w| w.operation_id)
    }
    pub fn ready_for_text(&self) -> bool {
        !self.is_paused()
            && self.pending.is_none()
            && !self.directed.as_ref().is_some_and(|d| d.has_pending())
    }
    pub fn has_obligation(&self) -> bool {
        self.pending.is_some() || self.directed.as_ref().is_some_and(|d| d.has_pending())
    }

    /// Compose a directed PKI text with a caller-generated random extra nonce.
    /// IDs always come from the same caller-durable interval as channel traffic.
    pub fn queue_directed_text(
        &mut self,
        now: u64,
        mut header: Header,
        text: &str,
        extra: [u8; 4],
    ) -> Result<PacketIdentity, InstanceError> {
        self.require_active()?;
        self.check_now(now)?;
        if !self.ready_for_text() {
            return Err(InstanceError::PendingFull);
        }
        let id = self.next_protocol_identity()?;
        header.source = id.source;
        header.packet_id = id.packet_id;
        header.channel_hash = 0;
        let directed = self.directed.as_ref().ok_or(InstanceError::PkiRequired)?;
        let body = crate::application::encode_text(text)
            .map_err(|e| InstanceError::Node(NodeError::Application(e)))?;
        let frame = directed
            .cipher
            .seal_packet(header, extra, &body)
            .map_err(InstanceError::Pki)?;
        let operation = self.store_protocol_frame(now, id, frame)?;
        if header.want_ack {
            self.directed.as_mut().unwrap().waiting = Some(Waiting {
                operation_id: operation,
                identity: id,
                expires_at: None,
            });
        }
        Ok(id)
    }
    fn next_protocol_identity(&self) -> Result<PacketIdentity, InstanceError> {
        let packet_id = self.packet_ids.next_packet_id();
        if packet_id == 0 || packet_id >= self.lease.end() {
            return Err(InstanceError::LeaseExhausted {
                end: self.lease.end(),
            });
        }
        Ok(PacketIdentity {
            source: self.packet_ids.source(),
            packet_id,
        })
    }
    fn store_protocol_frame(
        &mut self,
        now: u64,
        identity: PacketIdentity,
        frame: Vec<u8>,
    ) -> Result<u32, InstanceError> {
        if self.pending.is_some() {
            return Err(InstanceError::PendingFull);
        }
        let expires_at =
            now.checked_add(self.pending_ttl)
                .ok_or(InstanceError::DeadlineOverflow {
                    now,
                    ttl: self.pending_ttl,
                })?;
        let operation_id = self.allocate_operation_id()?;
        let reserved = self.packet_ids.reserve().map_err(InstanceError::PacketId)?;
        debug_assert_eq!(reserved, identity);
        self.record_now(now)?;
        self.pending = Some(PendingTx::Queued {
            operation_id,
            identity,
            frame,
            expires_at,
        });
        Ok(operation_id)
    }
    fn queue_ack(&mut self, now: u64, request: Header) -> Result<(), InstanceError> {
        let identity = self.next_protocol_identity()?;
        let body = routing::encode_ack(request.packet_id).map_err(InstanceError::Routing)?;
        let mut packet = Packet {
            header: Header {
                source: identity.source,
                destination: request.source,
                packet_id: identity.packet_id,
                hop_limit: 3,
                hop_start: 3,
                want_ack: false,
                via_mqtt: false,
                channel_hash: self.channel.hash,
                next_hop: 0,
                relay_node: identity.source as u8,
            },
            payload: body,
        };
        packet.apply_channel_cipher(&self.channel.key);
        let frame = packet.encode().map_err(InstanceError::Transport)?;
        self.store_protocol_frame(now, identity, frame)?;
        Ok(())
    }
    pub(super) fn receive_directed(
        &mut self,
        now: u64,
        mut packet: Packet,
        frame: &[u8],
    ) -> Result<ReceiveOutcome, InstanceError> {
        let header = packet.header;
        let local = self.packet_ids.source();
        if header.destination != local && header.destination != BROADCAST_DESTINATION {
            return Ok(ReceiveOutcome::IgnoredDestination);
        }
        let authenticated = header.channel_hash == 0;
        let plain = if authenticated {
            let d = self.directed.as_ref().unwrap();
            if header.source != d.binding.peer {
                return Ok(ReceiveOutcome::IgnoredPeer);
            }
            d.cipher.open_packet(frame).map_err(InstanceError::Pki)?.1
        } else if header.channel_hash == self.channel.hash {
            packet.apply_channel_cipher(&self.channel.key);
            packet.payload
        } else {
            return Ok(ReceiveOutcome::IgnoredChannel);
        };
        let envelope = ApplicationEnvelope::decode(&plain)
            .map_err(|e| InstanceError::Node(NodeError::Application(e)))?;
        if envelope.port == routing::ROUTING_PORT {
            let Ok(response) = routing::Response::decode(&plain) else {
                return Ok(ReceiveOutcome::NotText);
            };
            let d = self.directed.as_mut().unwrap();
            if header.source != d.binding.peer || header.destination != local {
                return Ok(ReceiveOutcome::NotText);
            }
            if d.waiting.as_ref().is_some_and(|w| {
                w.identity.packet_id == response.request_id
                    && w.expires_at.is_some_and(|end| now < end)
            }) {
                let w = d.waiting.take().unwrap();
                return Ok(ReceiveOutcome::Routing {
                    operation_id: w.operation_id,
                    identity: w.identity,
                    peer: header.source,
                    code: response.code,
                    authenticated,
                });
            }
            return Ok(ReceiveOutcome::NotText);
        }
        if envelope.port != TEXT_PORT {
            return Ok(ReceiveOutcome::NotText);
        }
        if !authenticated {
            if header.destination == local {
                return Err(InstanceError::PrivateChannelDowngrade);
            }
            let text = self.channel.open_text(frame).map_err(InstanceError::Node)?;
            if self.flood.observe(header) == Err(crate::flood::FloodIgnore::Duplicate) {
                return Ok(ReceiveOutcome::Duplicate {
                    identity: PacketIdentity {
                        source: header.source,
                        packet_id: header.packet_id,
                    },
                });
            }
            return Ok(text
                .map(ReceiveOutcome::Text)
                .unwrap_or(ReceiveOutcome::NotText));
        }
        let text = envelope
            .text()
            .map_err(|e| InstanceError::Node(NodeError::Application(e)))?
            .to_owned();
        let duplicate = self
            .directed
            .as_ref()
            .unwrap()
            .seen
            .contains(&header.packet_id);
        if header.want_ack {
            self.queue_ack(now, header)?;
        }
        if duplicate {
            return Ok(ReceiveOutcome::Duplicate {
                identity: PacketIdentity {
                    source: header.source,
                    packet_id: header.packet_id,
                },
            });
        }
        let d = self.directed.as_mut().unwrap();
        if d.seen.len() == d.capacity {
            d.seen.pop_front();
        }
        d.seen.push_back(header.packet_id);
        Ok(ReceiveOutcome::PkiText(ReceivedText { header, text }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{LossPermission, PacketIdLease, PauseOutcome, SennetInstanceConfig};
    use crate::{
        flood::{ManagedFloodConfig, RelayDelayWindow},
        node::Channel,
        transport::ChannelKey,
    };
    use core::time::Duration;
    fn header(source: u32, destination: u32, packet_id: u32) -> Header {
        Header {
            source,
            destination,
            packet_id,
            hop_limit: 3,
            want_ack: true,
            via_mqtt: false,
            hop_start: 3,
            channel_hash: 0,
            next_hop: 0,
            relay_node: 0,
        }
    }
    fn channel() -> Channel {
        Channel {
            hash: 8,
            key: ChannelKey::Aes128([5; 16]),
        }
    }
    fn fixture() -> (SennetInstance, PeerSession) {
        let a = LocalIdentity::from_secret([7; 32]);
        let b = LocalIdentity::from_secret([9; 32]);
        let remote = b.peer(22, 11, a.public_key()).unwrap();
        let mut station = SennetInstance::new(
            SennetInstanceConfig {
                channel: channel(),
                flood: ManagedFloodConfig {
                    channel_hash: 8,
                    relay_node: 11,
                    seen_capacity: 8,
                    delay: RelayDelayWindow::new(Duration::ZERO, Duration::ZERO).unwrap(),
                },
                directory: Default::default(),
                pending_ttl: 100,
            },
            PacketIdLease::new(11, 1, 500, 1).unwrap(),
        )
        .unwrap();
        station.install_pki(22, a, b.public_key(), 30, 2).unwrap();
        (station, remote)
    }
    fn response(source: u32, request: u32) -> Vec<u8> {
        let mut packet = Packet {
            header: Header {
                channel_hash: 8,
                ..header(source, 11, 90)
            },
            payload: routing::encode_ack(request).unwrap(),
        };
        packet.apply_channel_cipher(&channel().key);
        packet.encode().unwrap()
    }
    #[test]
    fn response_requires_physical_completion_exact_peer_id_and_open_window() {
        let (mut s, peer) = fixture();
        let id = s
            .queue_directed_text(0, header(0, 22, 0), "out", [1; 4])
            .unwrap();
        let out = s.take_outbound(1).unwrap().unwrap();
        assert_eq!(
            crate::application::decode_text(&peer.open_packet(&out.frame).unwrap().1),
            Ok("out")
        );
        assert_eq!(
            s.receive(2, &response(22, id.packet_id)).unwrap(),
            ReceiveOutcome::NotText
        );
        s.complete_tx(3, out.operation_id, id).unwrap();
        assert_eq!(s.awaiting_ack_operation_id(), Some(out.operation_id));
        assert_eq!(s.next_deadline(), Some(33));
        assert_eq!(
            s.receive(4, &response(22, id.packet_id + 1)).unwrap(),
            ReceiveOutcome::NotText
        );
        assert_eq!(
            s.receive(5, &response(23, id.packet_id)).unwrap(),
            ReceiveOutcome::NotText
        );
        assert!(
            matches!(s.receive(6,&response(22,id.packet_id)).unwrap(),ReceiveOutcome::Routing{operation_id,code:0,authenticated:false,..} if operation_id==out.operation_id)
        );
        assert!(!s.has_obligation());
        assert!(s.ready_for_text());
    }
    #[test]
    fn ack_timeout_and_explicit_loss_are_retained_absence_obligations() {
        let (mut s, _) = fixture();
        let id = s
            .queue_directed_text(0, header(0, 22, 0), "out", [1; 4])
            .unwrap();
        let out = s.take_outbound(1).unwrap().unwrap();
        s.complete_tx(2, out.operation_id, id).unwrap();
        assert_eq!(
            s.assess_pause(3, 33).unwrap(),
            PauseOutcome::RequiresLoss { pending: 1 }
        );
        assert!(s.pause(3, 10).is_err());
        assert!(
            matches!(s.advance(32).unwrap(),Some(InstanceEvent::AckExpired{identity,..}) if identity==id)
        );
        assert_eq!(
            s.receive(33, &response(22, id.packet_id)).unwrap(),
            ReceiveOutcome::NotText
        );
        let id = s
            .queue_directed_text(34, header(0, 22, 0), "next", [2; 4])
            .unwrap();
        let out = s.take_outbound(35).unwrap().unwrap();
        s.complete_tx(36, out.operation_id, id).unwrap();
        assert!(
            matches!(s.discard_pending(37,LossPermission::operator()).unwrap(),Some(InstanceEvent::AckLost{identity,..}) if identity==id)
        );
        assert!(s.ready_for_text());
    }
    #[test]
    fn authenticated_receive_reacks_duplicates_and_busy_refusal_does_not_poison_seen() {
        let (mut s, peer) = fixture();
        let incoming = peer
            .seal_packet(
                header(22, 11, 900),
                [7; 4],
                &crate::application::encode_text("in").unwrap(),
            )
            .unwrap();
        let own = s
            .queue_directed_text(0, header(0, 22, 0), "out", [1; 4])
            .unwrap();
        assert_eq!(s.receive(1, &incoming), Err(InstanceError::PendingFull));
        let out = s.take_outbound(2).unwrap().unwrap();
        s.complete_tx(3, out.operation_id, own).unwrap();
        assert!(matches!(
            s.receive(4, &incoming).unwrap(),
            ReceiveOutcome::PkiText(_)
        ));
        assert_eq!(
            s.assess_pause(5, 150).unwrap(),
            PauseOutcome::RequiresLoss { pending: 2 }
        );
        let ack = s.take_outbound(5).unwrap().unwrap();
        let mut wire = Packet::decode(&ack.frame).unwrap();
        assert!(!wire.header.want_ack);
        wire.apply_channel_cipher(&channel().key);
        assert_eq!(
            routing::Response::decode(&wire.payload).unwrap().request_id,
            900
        );
        s.complete_tx(6, ack.operation_id, ack.identity).unwrap();
        assert!(matches!(
            s.receive(7, &incoming).unwrap(),
            ReceiveOutcome::Duplicate { .. }
        ));
        let ack2 = s.take_outbound(8).unwrap().unwrap();
        assert_ne!(ack2.identity, ack.identity);
        s.complete_tx(9, ack2.operation_id, ack2.identity).unwrap();
        let mut corrupt = incoming;
        corrupt[16] ^= 1;
        assert!(matches!(
            s.receive(10, &corrupt),
            Err(InstanceError::Pki(_))
        ));
        assert_eq!(s.awaiting_ack_operation_id(), Some(out.operation_id));
    }
    #[test]
    fn channel_text_cannot_downgrade_or_poison_private_deduplication() {
        let (mut s, peer) = fixture();
        let downgrade = channel()
            .seal_text(
                Header {
                    channel_hash: 8,
                    ..header(22, 11, 900)
                },
                "spoof",
            )
            .unwrap();
        assert_eq!(
            s.receive(0, &downgrade),
            Err(InstanceError::PrivateChannelDowngrade)
        );
        let broadcast = channel()
            .seal_text(
                Header {
                    channel_hash: 8,
                    ..header(22, u32::MAX, 900)
                },
                "public",
            )
            .unwrap();
        assert!(matches!(
            s.receive(1, &broadcast).unwrap(),
            ReceiveOutcome::Text(_)
        ));
        let private = peer
            .seal_packet(
                Header {
                    want_ack: false,
                    ..header(22, 11, 900)
                },
                [0; 4],
                &crate::application::encode_text("private").unwrap(),
            )
            .unwrap();
        assert!(matches!(
            s.receive(2, &private).unwrap(),
            ReceiveOutcome::PkiText(_)
        ));
    }
}
