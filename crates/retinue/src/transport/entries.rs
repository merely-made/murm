//! The entries of the three keyed tables. Each is generic over the driver's time, `T`.

use super::{InterfaceId, Tick};
use crate::hash::AddressHash;
use crate::iface_mode::InterfaceMode;

/// A learned route to a destination, keyed by the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteEntry<T> {
    pub(crate) iface: InterfaceId,
    /// The next transport hop that announced this destination, if it is not direct.
    pub(crate) transport: Option<AddressHash>,
    pub(crate) hops: u8,
    /// When the route was learned, or last carried traffic.
    pub(crate) learned: T,
    /// The learning interface's mode when the route was learned.
    pub(crate) mode: InterfaceMode,
}

impl<T: Tick> RouteEntry<T> {
    /// Whether the route is usable at `now`. `lifetime` gives a route's lifetime in
    /// milliseconds by the mode it was learned in; the drivers pass
    /// `InterfaceMode::route_ttl` over their configured full-mode lifetime.
    pub(crate) fn live(&self, now: T, lifetime: impl Fn(InterfaceMode) -> u64) -> bool {
        now.millis_since(self.learned) < lifetime(self.mode)
    }
}

/// A link carried through this node, keyed by the link id: its two interfaces, and whether
/// its destination has proved it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkBridge<T> {
    pub(crate) from: InterfaceId,
    pub(crate) out: InterfaceId,
    /// The destination the request named, whose identity signs the proof.
    pub(crate) destination: AddressHash,
    pub(crate) seen: T,
    /// When an unproved link lapses; `None` once the destination's proof has passed.
    pub(crate) proof_deadline: Option<T>,
}

impl<T: Tick> LinkBridge<T> {
    /// Whether the bridge has lapsed at `now`: an unproved one at its deadline, a validated
    /// one `ttl` milliseconds after its last packet.
    pub(crate) fn lapsed(&self, now: T, ttl: u64) -> bool {
        self.proof_deadline
            .map_or(now.millis_since(self.seen) >= ttl, |deadline| {
                now >= deadline
            })
    }

    /// Whether the bridge has been unheard for `idle` milliseconds, so in a full table of
    /// validated links a new request may take its slot.
    pub(crate) fn idle(&self, now: T, idle: u64) -> bool {
        now.millis_since(self.seen) >= idle
    }
}

/// The way back for a carried packet's proof, keyed by the truncated packet hash the proof
/// is addressed to (RNS's reverse-table entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReverseEntry<T> {
    pub(crate) received: InterfaceId,
    pub(crate) outbound: InterfaceId,
    pub(crate) at: T,
}

impl<T: Tick> ReverseEntry<T> {
    /// Whether the return path has outlived `timeout` milliseconds at `now`.
    pub(crate) fn expired(&self, now: T, timeout: u64) -> bool {
        now.millis_since(self.at) >= timeout
    }
}
