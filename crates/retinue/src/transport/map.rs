//! [`Store`] over a `HashMap`, the desktop's container. A map has no order, so a removed
//! entry needs no place to go back to.

use std::collections::HashMap;

use super::Store;
use crate::hash::AddressHash;

impl<V> Store<V> for HashMap<AddressHash, V> {
    type Place = ();

    fn len(&self) -> usize {
        HashMap::len(self)
    }

    fn get(&self, key: &AddressHash) -> Option<&V> {
        HashMap::get(self, key)
    }

    fn get_mut(&mut self, key: &AddressHash) -> Option<&mut V> {
        HashMap::get_mut(self, key)
    }

    fn insert(&mut self, key: AddressHash, value: V) -> Result<Option<V>, V> {
        Ok(HashMap::insert(self, key, value))
    }

    fn remove(&mut self, key: &AddressHash) -> Option<(V, ())> {
        HashMap::remove(self, key).map(|value| (value, ()))
    }

    fn restore(&mut self, key: AddressHash, value: V, (): ()) {
        HashMap::insert(self, key, value);
    }

    fn least<R: Ord>(&self, mut rank: impl FnMut(&V) -> Option<R>) -> Option<AddressHash> {
        self.iter()
            .filter_map(|(key, value)| rank(value).map(|r| (*key, r)))
            .min_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(key, _)| key)
    }

    fn retain(
        &mut self,
        mut keep: impl FnMut(&AddressHash, &V) -> bool,
        mut removed: impl FnMut(AddressHash, V, ()),
    ) {
        // `extract_if` hands the removed entries over in one pass, as `retain` would scan.
        for (key, value) in self.extract_if(|key, value| !keep(key, value)) {
            removed(key, value, ());
        }
    }

    fn sweep(&mut self, mut dead: impl FnMut(&V) -> bool, mut removed: impl FnMut(AddressHash, V)) {
        for (key, value) in self.extract_if(|_, value| dead(value)) {
            removed(key, value);
        }
    }
}
