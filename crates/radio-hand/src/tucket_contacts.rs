//! Board-owned public Tucket cache. No identity seeds, routes or pending work.
//!
//! The named firmware profile has eight contacts. Callers can select a smaller
//! runtime capacity; restore refuses overflow rather than truncating the cache.
//! Flash stalls belong to the board owner's declared quiet window, not here.
use crate::store::{self, Slot};
use heapless::Vec;
use sha2::{Digest, Sha256};

mod codec;
#[cfg(feature = "tucket-contact-store-bench")]
pub mod fixture;
pub const MAX_KEYS: usize = 8;
pub const SLOT_LEN: usize = store::encoded_len(72 + 32 * MAX_KEYS);

/// Public identity plus an exact receive-profile digest. TX power is not a realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    pub local_public: [u8; 32],
    pub profile_digest: [u8; 32],
}
impl Scope {
    pub fn new(local_public: [u8; 32], profile: selvage::PhyProfile) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"tucket-contact-profile-v1\0");
        hash.update(profile.frequency_hz.to_le_bytes());
        hash.update(profile.bandwidth_hz.to_le_bytes());
        hash.update([profile.spreading_factor, profile.coding_rate_denominator]);
        hash.update(profile.preamble_symbols.to_le_bytes());
        hash.update([
            profile.sync_word,
            u8::from(profile.explicit_header),
            u8::from(profile.crc),
            u8::from(profile.invert_iq),
        ]);
        Self {
            local_public,
            profile_digest: hash.finalize().into(),
        }
    }
}

/// Pseudonymous peer associations, deliberately classified in the flash policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub scope: Scope,
    pub keys: Vec<[u8; 32], MAX_KEYS>,
}
impl Snapshot {
    pub fn new(scope: Scope, keys: &[[u8; 32]]) -> Result<Self, Fault> {
        if keys.len() > MAX_KEYS {
            return Err(Fault::Capacity);
        }
        tucket::node::validate_contact_keys(core::slice::from_ref(&scope.local_public))
            .map_err(|_| Fault::Keys)?;
        tucket::node::validate_contact_keys(keys).map_err(|_| Fault::Keys)?;
        let mut keys = Vec::from_slice(keys).map_err(|_| Fault::Capacity)?;
        keys.sort_unstable();
        Ok(Self { scope, keys })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Io,
    Corrupt,
    Unsupported,
    Scope,
    Capacity,
    Keys,
    Ambiguous,
    Exhausted,
    Verify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commit {
    pub slot: Slot,
    pub sequence: u32,
    pub changed: bool,
}
pub struct Loaded {
    pub snapshot: Snapshot,
    pub slot: Slot,
    pub sequence: u32,
}

/// The adapter owns two dedicated erase pages. Writes are aligned and the
/// commit magic is programmed LAST; no other durable pair may be touched.
pub trait Storage {
    type Error;
    fn read(&mut self, slot: Slot, out: &mut [u8; SLOT_LEN]) -> Result<(), Self::Error>;
    fn erase(&mut self, slot: Slot) -> Result<(), Self::Error>;
    fn write(&mut self, slot: Slot, offset: usize, bytes: &[u8]) -> Result<(), Self::Error>;
}

pub fn load(store: &mut impl Storage, scope: Scope) -> Result<Option<Loaded>, Fault> {
    let mut a = [0xff; SLOT_LEN];
    let mut b = [0xff; SLOT_LEN];
    store.read(Slot::A, &mut a).map_err(|_| Fault::Io)?;
    store.read(Slot::B, &mut b).map_err(|_| Fault::Io)?;
    let Some((slot, sequence, snapshot)) = codec::select(&a, &b)? else {
        return Ok(None);
    };
    if snapshot.scope != scope {
        return Err(Fault::Scope);
    }
    Ok(Some(Loaded {
        snapshot,
        slot,
        sequence,
    }))
}

pub fn save(store: &mut impl Storage, snapshot: &Snapshot) -> Result<Commit, Fault> {
    // Validate again because Snapshot's public fields are caller-owned data.
    let canonical = Snapshot::new(snapshot.scope, &snapshot.keys)?;
    let loaded = load(store, snapshot.scope)?;
    if let Some(old) = &loaded
        && old.snapshot == canonical
    {
        return Ok(Commit {
            slot: old.slot,
            sequence: old.sequence,
            changed: false,
        });
    }
    let (slot, sequence) = match loaded {
        Some(old) => (
            old.slot.other(),
            old.sequence.checked_add(1).ok_or(Fault::Exhausted)?,
        ),
        None => (Slot::A, 0),
    };
    let mut record = [0xff; SLOT_LEN];
    let len = codec::encode(sequence, &canonical, &mut record)?;
    store.erase(slot).map_err(|_| Fault::Io)?;
    store
        .write(slot, 4, &record[4..len])
        .map_err(|_| Fault::Io)?;
    store.write(slot, 0, &record[..4]).map_err(|_| Fault::Io)?;
    let mut check = [0xff; SLOT_LEN];
    store.read(slot, &mut check).map_err(|_| Fault::Io)?;
    let observed = codec::decode(&check)?.ok_or(Fault::Verify)?;
    if observed.0 != sequence || observed.1 != canonical {
        return Err(Fault::Verify);
    }
    Ok(Commit {
        slot,
        sequence,
        changed: true,
    })
}
