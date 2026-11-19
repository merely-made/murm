//! Relayed announces: jittered scheduling, neighbour suppression, interface modes and the
//! announce cap.

use super::{Action, Actions, InterfaceId, Node, QUEUED_ANNOUNCES, REBROADCAST_WINDOW};
use crate::hash::AddressHash;
use crate::iface_mode::ModeFlags;
use crate::packet::Packet;
use crate::rebroadcast::{AnnounceCap, CapRate, Offer, Owed, Rebroadcast};
use crate::transport::{relay_targets, release_from};

impl<const PEERS: usize, const ACTIONS: usize, const LINKS: usize, const ROUTES: usize>
    Node<PEERS, ACTIONS, LINKS, ROUTES>
{
    /// When [`Node::poll`] next has a relayed announce to send, if any. A shell that polls
    /// slower than [`REBROADCAST_WINDOW`] can wake for it.
    pub fn next_rebroadcast(&self) -> Option<u64> {
        self.announce_caps
            .iter()
            .filter_map(|(_, cap)| cap.next_due())
            .chain(self.rebroadcasts.next_due())
            .min()
    }

    /// Hold a relayed announce for a first transmission within [`REBROADCAST_WINDOW`]
    /// (`Transport.py` 2338). The delay is drawn from the announce and this node's identity,
    /// so neighbours spread out without an RNG and a replay is reproducible.
    pub(super) fn schedule_rebroadcast(
        &mut self,
        interface: InterfaceId,
        packet: Packet,
        emitted: u64,
        now: u64,
    ) {
        let hash = packet.hash();
        let ours = self.identity.hash();
        let draw = u16::from_le_bytes([
            hash.as_bytes()[0] ^ ours.as_bytes()[0],
            hash.as_bytes()[1] ^ ours.as_bytes()[1],
        ]);
        let delay = u64::from(draw) % (REBROADCAST_WINDOW + 1);
        let rebroadcast = Rebroadcast {
            destination: packet.destination,
            packet,
            interface,
            emitted,
            answer: None,
        };
        if !self.rebroadcasts.schedule(rebroadcast, now, delay) {
            self.transport_counters.refused_rebroadcasts = self
                .transport_counters
                .refused_rebroadcasts
                .saturating_add(1);
        }
    }

    /// A relayed copy of an announce we are rebroadcasting was heard (`Transport.py`
    /// 2180-2203).
    pub(super) fn hear_rebroadcast(&mut self, destination: AddressHash, hops: u8, now: u64) {
        if self.rebroadcasts.heard(destination, hops, now) {
            self.transport_counters.suppressed_rebroadcasts = self
                .transport_counters
                .suppressed_rebroadcasts
                .saturating_add(1);
        }
    }

    /// [`Self::hear_rebroadcast`] for an announce freshness turned away before verifying it:
    /// heard only as a copy of the announce we verified and hold.
    pub(super) fn hear_rebroadcast_copy(&mut self, packet: &Packet, now: u64) {
        if self.rebroadcasts.heard_copy(packet, now) {
            self.transport_counters.suppressed_rebroadcasts = self
                .transport_counters
                .suppressed_rebroadcasts
                .saturating_add(1);
        }
    }

    /// Send what the caps now allow, then what the table has due, while actions have room.
    pub(super) fn poll_rebroadcasts(&mut self, now: u64, actions: &mut Actions<ACTIONS>) {
        for index in 0..self.announce_caps.len() {
            let interface = self.announce_caps[index].0;
            let rate = CapRate::new(self.first_hop_airtime(interface));
            while actions.len() < ACTIONS {
                let Some(packet) = self.announce_caps[index].1.pop_due(now, rate) else {
                    break;
                };
                self.send_rebroadcast(interface, packet, actions);
            }
        }
        // What an earlier poll could not hold goes first.
        while actions.len() < ACTIONS {
            let (rebroadcast, owed) = match self.rebroadcasts.take_owed() {
                Some((rebroadcast, owed)) => (rebroadcast, Some(owed)),
                None => match self.rebroadcasts.pop_due(now) {
                    Some(rebroadcast) => (rebroadcast, None),
                    None => break,
                },
            };
            self.offer_rebroadcast(rebroadcast, owed, now, actions);
        }
        // An idle cap, or one whose airtime was cleared, holds nothing worth a slot.
        let first_hop_airtime = &self.first_hop_airtime;
        self.announce_caps.retain(|(interface, cap)| {
            !cap.idle(now) && first_hop_airtime.iter().any(|(id, _)| id == interface)
        });
    }

    /// Offer a due rebroadcast to every known interface, or to those an earlier poll still
    /// owes, each through its mode rule, then its cap where the airtime is known
    /// (`Transport.py` 1458-1585), as the endpoint does (D2). The next hop is the destination's
    /// live route at release (1461-1469); without one the relay is refused. An interface whose
    /// send does not fit in `actions` stays owed (D22).
    fn offer_rebroadcast(
        &mut self,
        rebroadcast: Rebroadcast,
        only: Option<Owed>,
        now: u64,
        actions: &mut Actions<ACTIONS>,
    ) {
        let route_ttl = self.transport.route_ttl;
        let from = release_from(
            &self.routes,
            &rebroadcast.destination,
            now,
            super::routes::lifetimes(route_ttl),
        )
        .map(|hop| self.interface_mode(hop));
        let known = self
            .interfaces
            .iter()
            .map(|&id| (id, self.interface_mode(id), ModeFlags::default()));
        let targets: Owed = relay_targets(from, known, only.as_deref()).collect();
        let mut owed = Owed::new();
        for interface in targets {
            if actions.len() >= ACTIONS {
                // Both are bounded by the interface table.
                let _ = owed.push(interface);
                continue;
            }
            self.offer_on(interface, rebroadcast.clone(), now, actions);
        }
        if !owed.is_empty() {
            self.rebroadcasts.owe(rebroadcast, owed, now);
        }
    }

    /// One interface's share of a relay: sent, or offered to its cap. An interface without a
    /// cap slot sends uncapped.
    fn offer_on(
        &mut self,
        interface: InterfaceId,
        rebroadcast: Rebroadcast,
        now: u64,
        actions: &mut Actions<ACTIONS>,
    ) {
        let airtime = self.first_hop_airtime(interface);
        if airtime == 0 {
            self.send_rebroadcast(interface, rebroadcast.packet, actions);
            return;
        }
        let index = match self
            .announce_caps
            .iter()
            .position(|(id, _)| *id == interface)
        {
            Some(index) => index,
            None => {
                if self
                    .announce_caps
                    .push((interface, AnnounceCap::new(QUEUED_ANNOUNCES)))
                    .is_err()
                {
                    self.send_rebroadcast(interface, rebroadcast.packet, actions);
                    return;
                }
                self.announce_caps.len() - 1
            }
        };
        let rate = CapRate::new(airtime);
        match self.announce_caps[index].1.offer(rebroadcast, now, rate) {
            Offer::Send(packet) => self.send_rebroadcast(interface, packet, actions),
            Offer::Queued => {
                self.transport_counters.capped_announces =
                    self.transport_counters.capped_announces.saturating_add(1);
            }
            Offer::Dropped => {
                self.transport_counters.dropped_announces =
                    self.transport_counters.dropped_announces.saturating_add(1);
            }
        }
    }

    fn send_rebroadcast(
        &mut self,
        interface: InterfaceId,
        packet: Packet,
        actions: &mut Actions<ACTIONS>,
    ) {
        if actions.push(Action::Send { interface, packet }) {
            self.transport_counters.forwarded_announces = self
                .transport_counters
                .forwarded_announces
                .saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::announce::{AnnounceBlob, RAND_HASH_LEN};
    use crate::destination::DestinationName;

    fn blob(seed: u8) -> AnnounceBlob {
        AnnounceBlob::from_wire([seed; RAND_HASH_LEN])
    }

    fn sent<const N: usize>(actions: &Actions<N>) -> alloc::vec::Vec<AddressHash> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Send { packet, .. } => Some(packet.destination),
                _ => None,
            })
            .collect()
    }

    fn relay<const R: usize>() -> Node<32, 8, 4, R> {
        Node::new(
            PrivateIdentity::from_secret_bytes(&[0x61; 64]),
            DestinationName::new("retinue", ["relay"]).name_hash(),
        )
        .with_transport_config(TransportConfig::transit())
    }

    fn announcer(seed: u8) -> Node<32, 8, 4> {
        Node::new(
            PrivateIdentity::from_secret_bytes(&[seed; 64]),
            DestinationName::new("retinue", ["announcer"]).name_hash(),
        )
    }

    /// The mode rule reads the route as it stands at release: heard on a roaming interface,
    /// then re-routed by a path response through a full one, the relay goes out on the roaming
    /// interface as well as the full one.
    #[test]
    fn the_next_hop_is_the_route_at_release() {
        let mut relay = relay::<4>();
        relay.set_interface_mode(1, InterfaceMode::Roaming).unwrap();
        let source = announcer(0x71);
        relay.ingest(1, &source.announce(&blob(0x71), None), 1_000);
        let mut response = source.announce(&blob(0x72), None);
        response.context = crate::path::CTX_PATH_RESPONSE;
        relay.ingest(2, &response, 1_001);
        assert_eq!(relay.route_to(source.destination(), 1_001).unwrap().0, 2);
        let due = relay.next_rebroadcast().unwrap();
        assert_eq!(sent(&relay.poll(due, 1, None)), [source.destination(); 2]);
    }

    /// A route the bounded table has evicted cannot supply the relay's source interface.
    #[test]
    fn an_evicted_route_blocks_the_relay() {
        let mut relay = relay::<1>();
        let first = announcer(0x71);
        relay.ingest(1, &first.announce(&blob(0x71), None), 1_000);
        let due = relay.next_rebroadcast().unwrap();
        let second = announcer(0x73);
        relay.ingest(1, &second.announce(&blob(0x73), None), 1_000);
        assert!(relay.route_to(first.destination(), 1_000).is_none());
        let relayed = sent(&relay.poll(due + REBROADCAST_WINDOW, 1, None));
        assert!(!relayed.contains(&first.destination()));
        assert!(relayed.contains(&second.destination()));
    }
}
