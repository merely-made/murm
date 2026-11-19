//! The transit duplicate memory: recently carried packet hashes, so a packet heard twice is
//! carried once.

use super::Tick;
use crate::hash::AddressHash;

/// How much a [`TransitMemory`] remembers, and for how long: each driver states its own (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryProfile {
    /// Hashes per generation.
    pub(crate) capacity: usize,
    /// Milliseconds after which a generation turns over, and after twice which both are
    /// cleared. `None` remembers until capacity alone turns a generation over.
    pub(crate) max_age: Option<u64>,
}

/// A set of hashes that one generation is kept in.
pub(crate) trait Generation: Default {
    fn contains(&self, hash: &AddressHash) -> bool;
    /// Add a hash not already held. A bounded set that is full drops it.
    fn add(&mut self, hash: AddressHash);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn clear(&mut self);
}

impl<const N: usize> Generation for heapless::Vec<AddressHash, N> {
    fn contains(&self, hash: &AddressHash) -> bool {
        self.as_slice().contains(hash)
    }
    fn add(&mut self, hash: AddressHash) {
        let _ = self.push(hash);
    }
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn clear(&mut self) {
        heapless::Vec::clear(self);
    }
}

#[cfg(feature = "tokio")]
impl Generation for std::collections::HashSet<AddressHash> {
    fn contains(&self, hash: &AddressHash) -> bool {
        std::collections::HashSet::contains(self, hash)
    }
    fn add(&mut self, hash: AddressHash) {
        self.insert(hash);
    }
    fn len(&self) -> usize {
        std::collections::HashSet::len(self)
    }
    fn clear(&mut self) {
        std::collections::HashSet::clear(self);
    }
}

/// Hashes remembered in two generations, as RNS keeps its packet hash list and the one before
/// it (`Transport.py` 832-834). A full generation becomes the previous one, so a burst forgets
/// the older half rather than everything. Under a `max_age`, a generation also turns over
/// with time, and both are cleared after twice it, so a quiet node does not remember a hash
/// indefinitely.
#[derive(Debug)]
pub(crate) struct TransitMemory<G, T> {
    pub(crate) current: G,
    previous: G,
    /// When the current generation began.
    started: T,
    profile: MemoryProfile,
}

impl<G: Generation, T: Tick> TransitMemory<G, T> {
    /// An empty memory whose first generation begins at `start`.
    pub(crate) fn new(profile: MemoryProfile, start: T) -> Self {
        Self {
            current: G::default(),
            previous: G::default(),
            started: start,
            profile,
        }
    }

    /// Record `hash` at `now`; false if it was already remembered. A generation turns over
    /// before the hash is checked, so a check always sees the last full generation.
    pub(crate) fn insert(&mut self, hash: AddressHash, now: T) -> bool {
        let age = now.millis_since(self.started);
        let full = self.current.len() >= self.profile.capacity;
        match self.profile.max_age {
            Some(max_age) if age >= max_age.saturating_mul(2) => {
                self.previous.clear();
                self.current.clear();
                self.started = now;
            }
            Some(max_age) if age >= max_age || full => self.turn_over(now),
            None if full => self.turn_over(now),
            _ => {}
        }
        if self.current.contains(&hash) || self.previous.contains(&hash) {
            return false;
        }
        self.current.add(hash);
        true
    }

    fn turn_over(&mut self, now: T) {
        self.previous = core::mem::take(&mut self.current);
        self.started = now;
    }
}
