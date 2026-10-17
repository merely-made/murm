//! Public contact knowledge shares the atomic correspondence journal and scope.
use super::*;
use radio_hand::instances::Runtime;

pub(super) fn validate_book(scope: &Scope, keys: &[[u8; 32]]) -> Result<(), AdapterError> {
    if !keys.is_empty() && !matches!(scope, Scope::Resident(_)) {
        return Err(backend("stock companion contacts belong to its firmware"));
    }
    tucket::node::validate_contact_keys(keys)
        .map_err(|e| backend(format!("invalid cached MeshCore contacts: {e:?}")))
}

impl RadioComms {
    /// Previously learned/operator-selected public peer keys in this resident
    /// identity/realm. Private keys and pending sends are never cached here.
    pub fn meshcore_contacts(&self) -> Result<Vec<[u8; 32]>, AdapterError> {
        let inner = self.lock()?;
        if !matches!(inner.book.scope, Scope::Resident(_)) {
            return Err(backend("stock companion contacts belong to its firmware"));
        }
        Ok(inner.book.meshcore_contacts.clone())
    }

    pub(super) fn persist_tucket_contacts(&self, runtime: &Runtime) -> Result<(), AdapterError> {
        let Some(tucket) = runtime.installed_tucket() else {
            return Ok(());
        };
        let mut keys: Vec<_> = tucket.node().contact_keys().collect();
        keys.sort_unstable();
        {
            let inner = self.lock()?;
            validate_book(&inner.book.scope, &keys)?;
            if inner.book.meshcore_contacts == keys {
                return Ok(());
            }
        }
        // edit publishes the new book only after its atomic commit succeeds.
        self.edit(|book| {
            // Older writers accept only version 1 and must refuse this cache.
            book.version = 2;
            book.meshcore_contacts = keys;
            Ok(())
        })
    }
}
