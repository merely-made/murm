//! Relayed announces: the shared rebroadcast table, the interface-mode rules and per-interface
//! announce caps, driven by one task on the endpoint's announce clock.

use alloc::vec::Vec;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::hash::AddressHash;
use crate::iface_mode::{InterfaceMode, ModeFlags, announce_permitted};
use crate::packet::Packet;
use crate::rebroadcast::{AnnounceCap, CapRate, Offer, Rebroadcast, Rebroadcasts};

use super::entropy::fill_random;
use super::interface::{InterfaceId, QueueAdmission};
use super::queue::TrafficClass;
use super::runtime::track;
use super::shared::Shared;

/// Relayed announces awaiting transmission or retry. At capacity one already sent makes room.
const REBROADCAST_CAPACITY: usize = 4_096;

/// Announces one capped interface queues: RNS's `MAX_QUEUED_ANNOUNCES` (`Reticulum.py` 111).
const QUEUED_ANNOUNCES: usize = 4_096;

/// The rebroadcast table and the caps of the interfaces whose airtime is known.
pub(super) struct Rebroadcasting {
    table: Rebroadcasts,
    caps: HashMap<InterfaceId, AnnounceCap>,
}

impl Rebroadcasting {
    pub(super) fn new() -> Self {
        Self {
            table: Rebroadcasts::new(REBROADCAST_CAPACITY),
            caps: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn scheduled(&self, destination: AddressHash) -> bool {
        self.table.contains(destination)
    }
}

impl Shared {
    /// Hold a relayed announce for a first transmission after a random delay of up to
    /// [`Endpoint::set_relay_jitter`](super::Endpoint::set_relay_jitter) (`Transport.py` 2338).
    pub(super) fn schedule_rebroadcast(&self, iface: InterfaceId, packet: Packet, emitted: u64) {
        let max = self.relay_jitter_ms.load(Ordering::Relaxed);
        let delay = if max == 0 {
            0
        } else {
            let mut draw = [0u8; 8];
            fill_random(&mut draw);
            u64::from_le_bytes(draw) % (max + 1)
        };
        let rebroadcast = Rebroadcast {
            destination: packet.destination,
            packet,
            interface: iface,
            emitted,
            answer: None,
        };
        let now = self.announce_admission_now_ms();
        if self
            .rebroadcasts
            .lock()
            .unwrap()
            .table
            .schedule(rebroadcast, now, delay)
        {
            self.rebroadcast_wake.notify_one();
        } else {
            self.routing_stats
                .refused_rebroadcasts
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Queue a path response for transmission on `iface` alone after `delay`, through the
    /// announce table as RNS does, so a neighbour's answer heard first ends it
    /// (`Transport.py` 3486-3519).
    pub(super) fn schedule_path_response(
        &self,
        iface: InterfaceId,
        packet: Packet,
        delay: Duration,
    ) -> bool {
        let rebroadcast = Rebroadcast {
            destination: packet.destination,
            packet,
            interface: iface,
            emitted: 0,
            answer: Some(iface),
        };
        let now = self.announce_admission_now_ms();
        let delay = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        let queued = self
            .rebroadcasts
            .lock()
            .unwrap()
            .table
            .answer(rebroadcast, now, delay);
        if queued {
            self.rebroadcast_wake.notify_one();
        }
        queued
    }

    /// Whether a recursive path request of `len` bytes may go out on `iface` now, charging its
    /// announce budget (`Transport.py` 3301-3318). An interface of unknown airtime is uncapped.
    pub(super) fn admit_recursive_path_request(&self, iface: InterfaceId, len: usize) -> bool {
        let Some(rate) = self.cap_rate(iface) else {
            return true;
        };
        let now = self.announce_admission_now_ms();
        self.rebroadcasts
            .lock()
            .unwrap()
            .caps
            .entry(iface)
            .or_insert_with(|| AnnounceCap::new(QUEUED_ANNOUNCES))
            .admit_path_request(len, now, rate)
    }

    /// `iface`'s announce budget: its share of the configured bitrate, or of the carrier's MTU
    /// airtime; `None` when neither is known.
    fn cap_rate(&self, iface: InterfaceId) -> Option<CapRate> {
        let policy = self.iface_policy(iface);
        let percent = u64::from(policy.cap_percent);
        match policy.bitrate_bps.filter(|&bps| bps > 0) {
            Some(bps) => Some(CapRate::from_bitrate(bps, percent)),
            None => {
                let carrier = self.first_hop_airtime_ms.lock().unwrap();
                let ms = carrier.get(&iface).copied().filter(|&ms| ms > 0)?;
                Some(CapRate::from_airtime(ms, percent))
            }
        }
    }

    /// A relayed copy of an announce we are rebroadcasting was heard (`Transport.py`
    /// 2180-2203).
    pub(super) fn hear_rebroadcast(&self, destination: AddressHash, hops: u8) {
        let now = self.announce_admission_now_ms();
        if self
            .rebroadcasts
            .lock()
            .unwrap()
            .table
            .heard(destination, hops, now)
        {
            self.routing_stats
                .suppressed_rebroadcasts
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// [`Self::hear_rebroadcast`] for an announce freshness turned away before verifying it:
    /// heard only as a copy of the announce we verified and hold.
    pub(super) fn hear_rebroadcast_copy(&self, packet: &Packet) {
        let now = self.announce_admission_now_ms();
        if self
            .rebroadcasts
            .lock()
            .unwrap()
            .table
            .heard_copy(packet, now)
        {
            self.routing_stats
                .suppressed_rebroadcasts
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Send everything due at `now` on every interface that egress policy, `outgoing` and the
    /// mode rules permit, the ingress one included, through the caps of interfaces with a
    /// known airtime. A path response goes to its requester's interface alone, uncapped. The next hop is the destination's live path interface at release, so an
    /// announce whose path has lapsed is not relayed (`Transport.py` 1461-1469). RNS caps only
    /// the modes its rule chain leaves to its last branch (1518-1585), so relays to roaming,
    /// boundary and internal interfaces go uncapped there; every mode is capped here. Returns
    /// when to come back.
    fn release_rebroadcasts(&self, now: u64) -> Option<u64> {
        let egress = self.routing.lock().unwrap().allowed_egress.clone();
        let policies = self.iface_policies.lock().unwrap().clone();
        let policy = |id| policies.get(&id).copied().unwrap_or_default();
        let modes: Vec<(InterfaceId, InterfaceMode)> = self
            .interfaces
            .lock()
            .unwrap()
            .iter()
            .map(|i| (i.id, i.mode))
            .collect();
        let ifaces: Vec<(InterfaceId, InterfaceMode)> = modes
            .iter()
            .copied()
            .filter(|(id, _)| egress.allows(*id) && policy(*id).outgoing)
            .collect();
        let carrier = self.first_hop_airtime_ms.lock().unwrap().clone();
        let rate = |id: InterfaceId| {
            let policy = policy(id);
            let percent = u64::from(policy.cap_percent);
            match policy.bitrate_bps.filter(|&bps| bps > 0) {
                Some(bps) => Some(CapRate::from_bitrate(bps, percent)),
                None => carrier
                    .get(&id)
                    .filter(|&&ms| ms > 0)
                    .map(|&ms| CapRate::from_airtime(ms, percent)),
            }
        };
        let due: Vec<Rebroadcast> = {
            let mut state = self.rebroadcasts.lock().unwrap();
            core::iter::from_fn(|| state.table.pop_due(now)).collect()
        };
        let next_hops: Vec<Option<InterfaceId>> = {
            let (paths, at, ttl) = (
                self.path_table.lock().unwrap(),
                Instant::now(),
                self.route_ttl(),
            );
            due.iter()
                .map(|r| {
                    paths
                        .get(&r.destination)
                        .filter(|e| e.live_at(at, ttl))
                        .map(|e| e.iface)
                })
                .collect()
        };
        let (mut capped, mut dropped) = (0, 0);
        let mut sends: Vec<Vec<(InterfaceId, Packet)>> = Vec::new();
        let mut answers: Vec<(InterfaceId, Packet)> = Vec::new();
        let next = {
            let mut state = self.rebroadcasts.lock().unwrap();
            let Rebroadcasting { table, caps } = &mut *state;
            caps.retain(|&id, cap| {
                ifaces.iter().any(|(i, _)| *i == id) && rate(id).is_some() && !cap.idle(now)
            });
            for (&id, cap) in caps.iter_mut() {
                while let Some(packet) = rate(id).and_then(|rate| cap.pop_due(now, rate)) {
                    sends.push(alloc::vec![(id, packet)]);
                }
            }
            for (rebroadcast, next_hop) in due.into_iter().zip(next_hops) {
                if let Some(id) = rebroadcast.answer {
                    if ifaces.iter().any(|(i, _)| *i == id) {
                        answers.push((id, rebroadcast.packet));
                    }
                    continue;
                }
                let from = next_hop.and_then(|hop| modes.iter().find(|(id, _)| *id == hop));
                let to_internal = next_hop.is_some_and(|hop| policy(hop).announces_to_internal);
                let mut out = Vec::new();
                for &(id, mode) in &ifaces {
                    let flags = ModeFlags {
                        from_internal: policy(id).announces_from_internal,
                        to_internal,
                    };
                    if !announce_permitted(mode, from.map(|m| m.1), false, flags) {
                        continue;
                    }
                    let Some(rate) = rate(id) else {
                        out.push((id, rebroadcast.packet.clone()));
                        continue;
                    };
                    let cap = caps
                        .entry(id)
                        .or_insert_with(|| AnnounceCap::new(QUEUED_ANNOUNCES));
                    match cap.offer(rebroadcast.clone(), now, rate) {
                        Offer::Send(packet) => out.push((id, packet)),
                        Offer::Queued => capped += 1,
                        Offer::Dropped => dropped += 1,
                    }
                }
                sends.push(out);
            }
            caps.values()
                .filter_map(AnnounceCap::next_due)
                .chain(table.next_due())
                .min()
        };
        let stats = &self.routing_stats;
        stats.capped_announces.fetch_add(capped, Ordering::Relaxed);
        stats
            .dropped_announces
            .fetch_add(dropped, Ordering::Relaxed);
        for (id, packet) in answers {
            self.push_to(id, packet, TrafficClass::Transit);
        }
        for transmission in sends {
            let mut queued = false;
            for (id, packet) in transmission {
                queued |= matches!(
                    self.push_to(id, packet, TrafficClass::Transit),
                    QueueAdmission::Queued
                );
            }
            if queued {
                stats.forwarded_announces.fetch_add(1, Ordering::Relaxed);
            }
        }
        next
    }
}

/// Run the rebroadcast table: sleep until the next due transmission, or until a new
/// announce is scheduled.
pub(super) fn start_rebroadcast_driver(shared: &Arc<Shared>) {
    let owner = Arc::clone(shared);
    track(shared, async move {
        loop {
            let wake = owner.rebroadcast_wake.notified();
            let next = owner.release_rebroadcasts(owner.announce_admission_now_ms());
            match next {
                Some(at) => {
                    let wait = at.saturating_sub(owner.announce_admission_now_ms());
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                        _ = wake => {}
                    }
                }
                None => wake.await,
            }
        }
    });
}
