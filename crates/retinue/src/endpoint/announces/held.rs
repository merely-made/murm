//! Bounded per-interface queues for already verified announces.

use crate::announce::Announce;
use crate::endpoint::interface::InterfaceId;
use crate::packet::Packet;
use alloc::vec::Vec;
use std::collections::HashMap;

/// A verified announce deferred by a noisy interface. It retains the ingress fact through
/// release so a later route cannot be accidentally attributed to a different bearer.
pub(in crate::endpoint) struct HeldAnnounce {
    pub(in crate::endpoint) interface: InterfaceId,
    pub(in crate::endpoint) packet: Packet,
    pub(in crate::endpoint) announce: Announce,
}

/// The endpoint holds at most this many interfaces' worth of the endpoint `held_capacity`,
/// so many noisy interfaces (a listener's clients) cannot each fill a full share.
pub(super) const HELD_SHARES: usize = 8;

/// Held announces by ingress interface (`Interface.py` 72, 270-297). No interface keeps an
/// empty queue, and `total` is the sum of the queue lengths.
#[derive(Default)]
pub(in crate::endpoint) struct HeldAnnounces {
    by_iface: HashMap<InterfaceId, Vec<HeldAnnounce>>,
    total: usize,
}

impl HeldAnnounces {
    /// Hold `held`, replacing one held for its destination in place (`Interface.py`
    /// 270-276). False when its interface holds `capacity` or the endpoint `ceiling`.
    pub(in crate::endpoint) fn hold(
        &mut self,
        held: HeldAnnounce,
        capacity: usize,
        ceiling: usize,
    ) -> bool {
        let iface = held.interface;
        let queue = self.by_iface.entry(iface).or_default();
        let destination = held.announce.destination;
        if let Some(existing) = queue
            .iter_mut()
            .find(|existing| existing.announce.destination == destination)
        {
            *existing = held;
            return true;
        }
        if queue.len() < capacity && self.total < ceiling {
            queue.push(held);
            self.total += 1;
            return true;
        }
        if queue.is_empty() {
            self.by_iface.remove(&iface);
        }
        false
    }

    /// How many announces `iface` holds (`len(interface.held_announces)`).
    pub(in crate::endpoint) fn held(&self, iface: InterfaceId) -> usize {
        self.by_iface.get(&iface).map_or(0, Vec::len)
    }

    /// The fewest-hops announce `iface` holds, the oldest among equals (`Interface.py` 285).
    pub(in crate::endpoint) fn take_nearest(&mut self, iface: InterfaceId) -> Option<HeldAnnounce> {
        let queue = self.by_iface.get_mut(&iface)?;
        let (index, _) = queue
            .iter()
            .enumerate()
            .min_by_key(|(_, held)| held.packet.hops)?;
        let held = queue.remove(index);
        if queue.is_empty() {
            self.by_iface.remove(&iface);
        }
        self.total -= 1;
        Some(held)
    }

    /// Drop everything `iface` holds: it was detached, or its admission row evicted.
    pub(in crate::endpoint) fn purge(&mut self, iface: InterfaceId) {
        if let Some(queue) = self.by_iface.remove(&iface) {
            self.total -= queue.len();
        }
    }
}
