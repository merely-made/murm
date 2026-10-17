//! Public contact restoration. Storage and local identity custody stay with the caller.
use ed25519_dalek::VerifyingKey;

use super::{Node, state::Contact};
use crate::identity::Identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactRestoreError {
    InvalidPublicKey,
    DuplicateKey,
    HashCollision,
    ContactsFull,
}

/// Check an owner-supplied public-key cache before changing protocol state.
/// This checks encoding and wire-address uniqueness, not advert authenticity.
/// The storage owner must bind previously learned/operator-selected keys to
/// the original local identity and realm. No private key belongs in this cache.
pub fn validate_contact_keys(keys: &[[u8; 32]]) -> Result<(), ContactRestoreError> {
    // The current endpoint wire address is one byte, independently of a
    // caller's smaller configured contact capacity.
    if keys.len() > 256 {
        return Err(ContactRestoreError::ContactsFull);
    }
    for (index, key) in keys.iter().enumerate() {
        if !VerifyingKey::from_bytes(key).is_ok_and(|key| !key.is_weak()) {
            return Err(ContactRestoreError::InvalidPublicKey);
        }
        for prior in &keys[..index] {
            if prior == key {
                return Err(ContactRestoreError::DuplicateKey);
            }
            if prior[0] == key[0] {
                return Err(ContactRestoreError::HashCollision);
            }
        }
    }
    Ok(())
}

impl Node {
    /// Public keys only. Learned routes, dedup and pending work are not a
    /// durable contact cache and must not be replayed implicitly at restart.
    pub fn contact_keys(&self) -> impl Iterator<Item = [u8; 32]> + '_ {
        self.contacts
            .iter()
            .map(|(_, contact)| contact.identity.pub_key)
    }

    /// Merge a validated cache atomically into this node's configured capacity.
    /// Existing contacts/routes survive every refusal. New contacts start
    /// without a route, so ordinary flood discovery can learn a fresh path.
    /// Uses the contact storage already reserved at construction.
    pub fn restore_contacts(&mut self, keys: &[[u8; 32]]) -> Result<(), ContactRestoreError> {
        validate_contact_keys(keys)?;
        let mut additional = 0;
        for key in keys {
            match self.contact(key[0]) {
                Some(existing) if existing.pub_key != *key => {
                    return Err(ContactRestoreError::HashCollision);
                }
                Some(_) => {}
                None => additional += 1,
            }
        }
        if additional > self.capacity.contacts - self.contacts.len() {
            return Err(ContactRestoreError::ContactsFull);
        }
        // Every fallible check precedes mutation; capacity was reserved by new.
        for key in keys {
            if self.contact(key[0]).is_none() {
                self.contacts.push((
                    key[0],
                    Contact {
                        identity: Identity::new(*key),
                        route: None,
                    },
                ));
            }
        }
        Ok(())
    }
}
