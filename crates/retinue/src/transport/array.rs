//! [`Store`] over a bounded array, the board's container: a linear find, and the removal
//! order the board has always had, so its ties break as they did.

use heapless::Vec as BoundedVec;

use super::{Slot, Store};
use crate::hash::AddressHash;

/// Where a removed slot stood in the array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Position {
    /// Taken by `swap_remove`: the last slot moved into its index.
    Swapped(usize),
    /// Taken by an order-keeping removal: the slots after it moved down one.
    Shifted(usize),
}

fn index_of<S: Slot, const N: usize>(slots: &BoundedVec<S, N>, key: &AddressHash) -> Option<usize> {
    slots.iter().position(|slot| slot.key() == *key)
}

impl<S: Slot, const N: usize> Store<S::Entry> for BoundedVec<S, N> {
    type Place = Position;

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn get(&self, key: &AddressHash) -> Option<&S::Entry> {
        self.iter().find(|slot| slot.key() == *key).map(Slot::entry)
    }

    fn get_mut(&mut self, key: &AddressHash) -> Option<&mut S::Entry> {
        self.iter_mut()
            .find(|slot| slot.key() == *key)
            .map(Slot::entry_mut)
    }

    fn insert(&mut self, key: AddressHash, value: S::Entry) -> Result<Option<S::Entry>, S::Entry> {
        if let Some(index) = index_of(self, &key) {
            // A whole new slot, as the board rewrites one: what it carries beside the entry
            // starts over.
            let old = core::mem::replace(&mut self[index], S::new(key, value));
            return Ok(Some(old.into_entry()));
        }
        self.push(S::new(key, value))
            .map(|()| None)
            .map_err(Slot::into_entry)
    }

    fn remove(&mut self, key: &AddressHash) -> Option<(S::Entry, Position)> {
        let index = index_of(self, key)?;
        Some((
            self.swap_remove(index).into_entry(),
            Position::Swapped(index),
        ))
    }

    fn restore(&mut self, key: AddressHash, value: S::Entry, place: Position) {
        match place {
            Position::Swapped(index) => {
                if self.push(S::new(key, value)).is_ok() {
                    let last = self.as_slice().len() - 1;
                    self.swap(index, last);
                }
            }
            Position::Shifted(index) => {
                let _ = BoundedVec::insert(self, index, S::new(key, value));
            }
        }
    }

    fn least<R: Ord>(&self, mut rank: impl FnMut(&S::Entry) -> Option<R>) -> Option<AddressHash> {
        // `min_by` keeps the first of equal ranks.
        self.iter()
            .filter_map(|slot| rank(slot.entry()).map(|r| (slot.key(), r)))
            .min_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(key, _)| key)
    }

    fn retain(
        &mut self,
        mut keep: impl FnMut(&AddressHash, &S::Entry) -> bool,
        mut removed: impl FnMut(AddressHash, S::Entry, Position),
    ) {
        let mut index = 0;
        while index < self.as_slice().len() {
            let slot = &self[index];
            if keep(&slot.key(), slot.entry()) {
                index += 1;
            } else {
                let slot = BoundedVec::remove(self, index);
                removed(slot.key(), slot.into_entry(), Position::Shifted(index));
            }
        }
    }

    fn sweep(
        &mut self,
        mut dead: impl FnMut(&S::Entry) -> bool,
        mut removed: impl FnMut(AddressHash, S::Entry),
    ) {
        // One at a time from the front, as the board's own sweeps did.
        while let Some(index) = self.iter().position(|slot| dead(slot.entry())) {
            let slot = self.swap_remove(index);
            removed(slot.key(), slot.into_entry());
        }
    }
}
