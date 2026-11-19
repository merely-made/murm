//! The transport tables both drivers keep: routes, link bridges, return paths and the transit
//! duplicate memory, bounded and clock-free.
//!
//! As in [`crate::rebroadcast`], time is the caller's: an operation takes `now` and returns
//! what it did. Each driver owns its locks, sends and counters. Storage is a small trait,
//! [`Store`], so the board keeps its bounded arrays and the desktop its `HashMap`s, and the
//! logic over them is written once (plan decisions D16 to D18).
//!
//! Lookups check age themselves; sweeping is the driver's call (D18). Capacities and time
//! limits are arguments, so each driver states its own profile (D10).

// The board's and desktop's drivers need `alloc`; the floor builds the tables but holds none.
#![cfg_attr(
    not(feature = "alloc"),
    allow(
        dead_code,
        unused_imports,
        reason = "the drivers that use these tables need alloc"
    )
)]

use crate::hash::AddressHash;

mod array;
mod entries;
#[cfg(feature = "tokio")]
mod map;
mod memory;
mod tables;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use array::Position;
pub(crate) use entries::{LinkBridge, ReverseEntry, RouteEntry};
pub(crate) use memory::{MemoryProfile, TransitMemory};
pub(crate) use tables::*;

/// An interface number, as both drivers name one.
pub(crate) type InterfaceId = u32;

/// A point in time as a driver keeps it: milliseconds on the board, `Instant` on the desktop.
pub(crate) trait Tick: Copy + Ord {
    /// Whole milliseconds from `earlier` to `self`; zero if `earlier` is not before.
    fn millis_since(self, earlier: Self) -> u64;
}

impl Tick for u64 {
    fn millis_since(self, earlier: Self) -> u64 {
        self.saturating_sub(earlier)
    }
}

#[cfg(feature = "tokio")]
impl Tick for std::time::Instant {
    fn millis_since(self, earlier: Self) -> u64 {
        // Whole milliseconds: every limit compared against this is a whole number of them.
        u64::try_from(self.saturating_duration_since(earlier).as_millis()).unwrap_or(u64::MAX)
    }
}

/// What a table needs from its container, by key.
///
/// Order matters on the board, where ties in eviction go to the earliest entry: `remove`
/// swaps the last entry into the gap, `retain` keeps order, and `least` returns the first
/// of equal ranks. A map has no order to keep.
pub(crate) trait Store<V> {
    /// Where a removed entry stood, so [`Store::restore`] puts it back exactly.
    type Place: Copy;

    fn len(&self) -> usize;
    fn get(&self, key: &AddressHash) -> Option<&V>;
    fn get_mut(&mut self, key: &AddressHash) -> Option<&mut V>;
    /// Replace `key`'s entry in place, returning it, or add one. A bounded container that is
    /// full refuses a new key and hands the value back.
    fn insert(&mut self, key: AddressHash, value: V) -> Result<Option<V>, V>;
    fn remove(&mut self, key: &AddressHash) -> Option<(V, Self::Place)>;
    /// Undo a [`Store::remove`]. Several are undone in the reverse of their order.
    fn restore(&mut self, key: AddressHash, value: V, place: Self::Place);
    /// The key of the entry with the least rank, skipping those ranked `None`.
    fn least<R: Ord>(&self, rank: impl FnMut(&V) -> Option<R>) -> Option<AddressHash>;
    /// Keep the entries `keep` accepts, in order, handing each removed one to `removed`.
    fn retain(
        &mut self,
        keep: impl FnMut(&AddressHash, &V) -> bool,
        removed: impl FnMut(AddressHash, V, Self::Place),
    );
    /// Remove the entries `dead` names, each as [`Store::remove`] would.
    fn sweep(&mut self, dead: impl FnMut(&V) -> bool, removed: impl FnMut(AddressHash, V));
}

/// One element of a bounded table: an entry and the key it is found by.
pub(crate) trait Slot {
    type Entry;
    fn key(&self) -> AddressHash;
    fn entry(&self) -> &Self::Entry;
    fn entry_mut(&mut self) -> &mut Self::Entry;
    /// A fresh slot; anything a slot carries beyond the entry starts at its default.
    fn new(key: AddressHash, entry: Self::Entry) -> Self;
    fn into_entry(self) -> Self::Entry;
}

/// The plain slot: a key beside its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Keyed<E> {
    pub(crate) key: AddressHash,
    pub(crate) entry: E,
}

impl<E> Slot for Keyed<E> {
    type Entry = E;
    fn key(&self) -> AddressHash {
        self.key
    }
    fn entry(&self) -> &E {
        &self.entry
    }
    fn entry_mut(&mut self) -> &mut E {
        &mut self.entry
    }
    fn new(key: AddressHash, entry: E) -> Self {
        Self { key, entry }
    }
    fn into_entry(self) -> E {
        self.entry
    }
}
