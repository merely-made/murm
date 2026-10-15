//! Announce rebroadcast for both cores: RNS's announce table and per-interface announce cap,
//! bounded and clock-free. Times are the caller's millisecond ticks.
//!
//! The table holds each relayed announce for a jittered first transmission and one retry,
//! and drops it early once neighbours are heard doing the work (`Transport.py` 765-829,
//! 2180-2203, 2338). It also carries path responses a transport answers from its announce
//! cache (3486-3519). The cap queues relayed announces an interface has no airtime for, at a
//! share of its bitrate, [`CapRate`] (`Transport.py` 1522-1585, `Interface.py` 391-427).

use alloc::vec::Vec;

use crate::hash::AddressHash;
use crate::node::{
    ANNOUNCE_CAP_PERCENT, FIRST_HOP_ALLOWANCE_BITS, InterfaceId, LOCAL_REBROADCASTS_MAX,
    QUEUED_ANNOUNCE_LIFE, REBROADCAST_GRACE, REBROADCAST_WINDOW,
};
use crate::packet::Packet;

/// Transmissions per announce: the first and RNS's single `PATHFINDER_R` retry. RNS ends an
/// entry once `retries` reaches two (`Transport.py` 772-777).
const TRANSMISSIONS: u8 = 2;

/// A relayed announce, ready for the wire: hops already counted, this node stamped as its
/// transport.
#[derive(Debug, Clone)]
pub(crate) struct Rebroadcast {
    pub(crate) destination: AddressHash,
    pub(crate) packet: Packet,
    /// Where it was heard. A single-radio shell sends it back out there.
    pub(crate) interface: InterfaceId,
    /// The announce's emission timebase, so a queue keeps the newest of a destination.
    pub(crate) emitted: u64,
    /// A path response for one requester: sent once, on this interface only, outside the
    /// mode rules and the cap (RNS `block_rebroadcasts` with an attached interface).
    pub(crate) answer: Option<InterfaceId>,
}

#[derive(Debug)]
struct Entry {
    rebroadcast: Rebroadcast,
    due: u64,
    sent: u8,
    /// Neighbour rebroadcasts heard at our hop count (RNS `IDX_AT_LCL_RBRD`).
    heard: u8,
}

/// RNS's announce table, bounded.
#[derive(Debug)]
pub(crate) struct Rebroadcasts {
    entries: Vec<Entry>,
    /// Entries set aside behind a path response for their destination, put back newest first
    /// once it is done (`Transport.py` 811-819, 3511-3516): earlier answers to other
    /// interfaces, then at most one relay. Bounded by `capacity`.
    held: Vec<Entry>,
    capacity: usize,
}

impl Rebroadcasts {
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            held: Vec::new(),
            capacity,
        }
    }

    /// Schedule a first transmission `delay` after `now`, replacing any relay for the same
    /// destination as RNS does. At capacity an entry already sent once makes room; with none,
    /// the announce is refused rather than displacing one still waiting for its first send.
    ///
    /// A replacement keeps the old entry's send count: the newer announce takes over the
    /// remaining transmission, so re-announcing one destination cannot pin a slot as unsent
    /// and starve the table (RNS resets its retries; its table is unbounded). Behind a pending
    /// path response the relay is held instead, so the requester is still answered (RNS lets
    /// the relay overwrite it).
    pub(crate) fn schedule(&mut self, rebroadcast: Rebroadcast, now: u64, delay: u64) -> bool {
        let mut entry = Entry {
            rebroadcast,
            due: now.saturating_add(delay),
            sent: 0,
            heard: 0,
        };
        let destination = entry.rebroadcast.destination;
        let Some(existing) = self.find(destination) else {
            return self.insert(entry);
        };
        if self.entries[existing].rebroadcast.answer.is_none() {
            entry.sent = self.entries[existing].sent;
            self.entries[existing] = entry;
            return true;
        }
        match self.held.iter().position(|held| {
            held.rebroadcast.destination == destination && held.rebroadcast.answer.is_none()
        }) {
            Some(relay) => {
                entry.sent = self.held[relay].sent;
                self.held[relay] = entry;
            }
            None if self.held.len() >= self.capacity => return false,
            // Below any held answers, so it is restored last.
            None => self.held.insert(0, entry),
        }
        true
    }

    /// Queue a path response `delay` after `now`, in place of the entry pending for its
    /// destination, which is held and restored once the response is done; an earlier answer
    /// to the same interface is superseded. It starts as RNS's does, at `PATHFINDER_R`
    /// retries: it is sent once, and hearing it relayed ends it.
    #[cfg_attr(
        not(feature = "tokio"),
        allow(dead_code, reason = "used by the endpoint")
    )]
    pub(crate) fn answer(&mut self, rebroadcast: Rebroadcast, now: u64, delay: u64) -> bool {
        let entry = Entry {
            rebroadcast,
            due: now.saturating_add(delay),
            sent: TRANSMISSIONS - 1,
            heard: 0,
        };
        let Some(existing) = self.find(entry.rebroadcast.destination) else {
            return self.insert(entry);
        };
        let superseded = self.entries[existing].rebroadcast.answer == entry.rebroadcast.answer;
        if !superseded && self.held.len() >= self.capacity {
            return false;
        }
        let old = core::mem::replace(&mut self.entries[existing], entry);
        if !superseded {
            self.held.push(old);
        }
        true
    }

    /// Add an entry for a destination not in the table. At capacity an entry already sent
    /// once makes room; with none, it is refused.
    fn insert(&mut self, entry: Entry) -> bool {
        if self.entries.len() >= self.capacity {
            let Some(index) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.sent > 0)
                .min_by_key(|(_, entry)| entry.due)
                .map(|(index, _)| index)
            else {
                return false;
            };
            let evicted = self.entries.swap_remove(index).rebroadcast.destination;
            self.held
                .retain(|held| held.rebroadcast.destination != evicted);
        }
        self.entries.push(entry);
        true
    }

    /// Remove an entry, restoring a relay held behind it.
    fn remove(&mut self, index: usize) -> Entry {
        let entry = self.entries.swap_remove(index);
        let destination = entry.rebroadcast.destination;
        if let Some(held) = self
            .held
            .iter()
            .rposition(|held| held.rebroadcast.destination == destination)
        {
            let held = self.held.remove(held);
            self.entries.push(held);
        }
        entry
    }

    /// A relayed copy of `destination`'s announce was heard with `hops` on the wire. Returns
    /// whether that ended our rebroadcast: two neighbours relaying at our hop count, or one
    /// node passing ours on within the retry grace, after our first send (`Transport.py`
    /// 2183-2203).
    pub(crate) fn heard(&mut self, destination: AddressHash, hops: u8, now: u64) -> bool {
        let Some(index) = self.find(destination) else {
            return false;
        };
        let entry = &mut self.entries[index];
        let ours = entry.rebroadcast.packet.hops;
        let mut done = false;
        if hops == ours {
            entry.heard = entry.heard.saturating_add(1);
            done = entry.sent > 0 && entry.heard >= LOCAL_REBROADCASTS_MAX;
        }
        if u16::from(hops) == u16::from(ours) + 1 && entry.sent > 0 && now < entry.due {
            done = true;
        }
        if done {
            self.remove(index);
        }
        done
    }

    /// [`Self::heard`] for an announce turned away unverified, as a neighbour's relay of the
    /// one we hold is (it repeats the blob, so freshness calls it a replay). It counts only if
    /// its signed fields match the copy that verified, so it needs no signature check.
    pub(crate) fn heard_copy(&mut self, packet: &Packet, now: u64) -> bool {
        let Some(index) = self.find(packet.destination) else {
            return false;
        };
        let held = &self.entries[index].rebroadcast.packet;
        if held.destination_type != packet.destination_type
            || held.context_flag != packet.context_flag
            || held.payload != packet.payload
        {
            return false;
        }
        self.heard(packet.destination, packet.hops, now)
    }

    /// Take the next transmission due at `now`. The entry stays for its retry, after
    /// [`REBROADCAST_GRACE`] plus [`REBROADCAST_WINDOW`], and leaves after its last send.
    pub(crate) fn pop_due(&mut self, now: u64) -> Option<Rebroadcast> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.due <= now)
            .min_by_key(|(_, entry)| entry.due)
            .map(|(index, _)| index)?;
        let entry = &mut self.entries[index];
        entry.sent += 1;
        entry.due = now
            .saturating_add(REBROADCAST_GRACE)
            .saturating_add(REBROADCAST_WINDOW);
        if entry.sent >= TRANSMISSIONS {
            Some(self.remove(index).rebroadcast)
        } else {
            Some(entry.rebroadcast.clone())
        }
    }

    /// When [`Self::pop_due`] next has something.
    pub(crate) fn next_due(&self) -> Option<u64> {
        self.entries.iter().map(|entry| entry.due).min()
    }

    /// Drop every entry heard on `iface` or answering there, held ones included: what was
    /// learned through a carrier that went away (`Transport.py` 3690-3703, per interface).
    #[cfg_attr(
        not(feature = "tokio"),
        allow(dead_code, reason = "used by the endpoint")
    )]
    pub(crate) fn forget_interface(&mut self, iface: InterfaceId) {
        let keep = |entry: &Entry| {
            entry.rebroadcast.interface != iface && entry.rebroadcast.answer != Some(iface)
        };
        self.held.retain(keep);
        let mut index = 0;
        while index < self.entries.len() {
            if keep(&self.entries[index]) {
                index += 1;
            } else {
                // Puts back a surviving held entry for the destination.
                self.remove(index);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, destination: AddressHash) -> bool {
        self.find(destination).is_some()
    }

    fn find(&self, destination: AddressHash) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.rebroadcast.destination == destination)
    }
}

/// One interface's announce budget: the airtime of one 500-byte MTU and the share of it
/// announces may use, in percent (`Reticulum.py` 286-289). The airtime is the fraction
/// `mtu_ms / per` milliseconds, so a configured bitrate paces exactly (RNS works in float
/// seconds) rather than from a whole-millisecond [`first_hop_airtime`](crate::node::first_hop_airtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapRate {
    mtu_ms: u64,
    per: u64,
    percent: u64,
}

impl CapRate {
    /// From a carrier's MTU airtime in milliseconds.
    pub(crate) const fn from_airtime(mtu_ms: u64, percent: u64) -> Self {
        Self {
            mtu_ms,
            per: 1,
            percent,
        }
    }

    /// From a bitrate in bits per second, which must be nonzero.
    #[cfg_attr(
        not(feature = "tokio"),
        allow(dead_code, reason = "used by the endpoint")
    )]
    pub(crate) const fn from_bitrate(bps: u64, percent: u64) -> Self {
        Self {
            mtu_ms: FIRST_HOP_ALLOWANCE_BITS * 1_000,
            per: bps,
            percent,
        }
    }

    /// From a carrier's MTU airtime, at RNS's default share, [`ANNOUNCE_CAP_PERCENT`].
    pub(crate) const fn new(mtu_ms: u64) -> Self {
        Self::from_airtime(mtu_ms, ANNOUNCE_CAP_PERCENT)
    }
}

/// The wait after sending `len` bytes before the next relayed announce, in whole milliseconds
/// rounded up: the transmission time over the cap share (`Transport.py` 1526-1528). A zero
/// share counts as 1 %.
pub(crate) fn cap_wait(len: usize, rate: CapRate) -> u64 {
    let num = len as u128 * 100 * u128::from(rate.mtu_ms);
    let den = 500 * u128::from(rate.percent.max(1)) * u128::from(rate.per.max(1));
    u64::try_from(num.div_ceil(den)).unwrap_or(u64::MAX)
}

/// What [`AnnounceCap::offer`] did with an announce.
#[derive(Debug)]
pub(crate) enum Offer {
    /// Within the cap: send it now.
    Send(Packet),
    /// Queued, or merged into a queued announce for the same destination.
    Queued,
    /// The queue was full.
    Dropped,
}

#[derive(Debug)]
struct Queued {
    rebroadcast: Rebroadcast,
    at: u64,
}

/// One interface's relayed-announce budget and queue.
#[derive(Debug)]
pub(crate) struct AnnounceCap {
    allowed_at: u64,
    queue: Vec<Queued>,
    capacity: usize,
}

impl AnnounceCap {
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            allowed_at: 0,
            queue: Vec::new(),
            capacity,
        }
    }

    /// Send now if nothing is queued and the budget allows, else queue. A destination already
    /// queued keeps one entry, replaced by a newer emission (`Transport.py` 1531-1565).
    pub(crate) fn offer(&mut self, rebroadcast: Rebroadcast, now: u64, rate: CapRate) -> Offer {
        if self.queue.is_empty() && now >= self.allowed_at {
            self.allowed_at = now.saturating_add(cap_wait(rebroadcast.packet.encoded_len(), rate));
            return Offer::Send(rebroadcast.packet);
        }
        if let Some(queued) = self
            .queue
            .iter_mut()
            .find(|queued| queued.rebroadcast.destination == rebroadcast.destination)
        {
            if rebroadcast.emitted > queued.rebroadcast.emitted {
                *queued = Queued {
                    rebroadcast,
                    at: now,
                };
            }
            return Offer::Queued;
        }
        if self.queue.len() >= self.capacity {
            return Offer::Dropped;
        }
        self.queue.push(Queued {
            rebroadcast,
            at: now,
        });
        Offer::Queued
    }

    /// Release the queued announce with the fewest hops, oldest first, once the budget allows.
    /// Entries older than [`QUEUED_ANNOUNCE_LIFE`] are dropped (`Interface.py` 391-427).
    pub(crate) fn pop_due(&mut self, now: u64, rate: CapRate) -> Option<Packet> {
        if now < self.allowed_at {
            return None;
        }
        self.queue
            .retain(|queued| now <= queued.at.saturating_add(QUEUED_ANNOUNCE_LIFE));
        let index = self
            .queue
            .iter()
            .enumerate()
            .min_by_key(|(_, queued)| (queued.rebroadcast.packet.hops, queued.at))
            .map(|(index, _)| index)?;
        let packet = self.queue.remove(index).rebroadcast.packet;
        self.allowed_at = now.saturating_add(cap_wait(packet.encoded_len(), rate));
        Some(packet)
    }

    /// When [`Self::pop_due`] next has something.
    pub(crate) fn next_due(&self) -> Option<u64> {
        (!self.queue.is_empty()).then_some(self.allowed_at)
    }

    /// Charge a recursive path request of `len` bytes to this budget, or refuse it while
    /// announces queue or the budget is spent (`Transport.py` 3301-3318).
    #[cfg_attr(
        not(feature = "tokio"),
        allow(dead_code, reason = "used by the endpoint")
    )]
    pub(crate) fn admit_path_request(&mut self, len: usize, now: u64, rate: CapRate) -> bool {
        if !self.queue.is_empty() || now < self.allowed_at {
            return false;
        }
        self.allowed_at = now.saturating_add(cap_wait(len, rate));
        true
    }

    /// Whether this cap holds nothing and constrains nothing at `now`, so it can be forgotten.
    pub(crate) fn idle(&self, now: u64) -> bool {
        self.queue.is_empty() && now >= self.allowed_at
    }

    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests;
