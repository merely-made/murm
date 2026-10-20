//! Explicit non-RF board storage fixture. Never enabled by a product profile.
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct Result {
    pub initial_keys: usize,
    pub final_keys: usize,
    pub commits: usize,
    pub sequence: u32,
}
#[inline(never)]
pub fn run(store: &mut impl Storage) -> core::result::Result<Result, Fault> {
    let expected = snapshot()?;
    let scope = expected.scope;
    let keys = &expected.keys;
    let initial = load(store, scope)?;
    let initial_keys = initial.as_ref().map(|s| s.snapshot.keys.len()).unwrap_or(0);
    if let Some(old) = &initial
        && !old.snapshot.keys.iter().all(|k| expected.keys.contains(k))
    {
        return Err(Fault::Verify);
    }
    let mut commits = 0;
    if initial_keys == 0 {
        commits += usize::from(save(store, &Snapshot::new(scope, &keys[..1])?)?.changed);
    }
    commits += usize::from(save(store, &expected)?.changed);
    if save(store, &expected)?.changed {
        return Err(Fault::Verify);
    }
    let cold = load(store, scope)?.ok_or(Fault::Verify)?;
    if cold.snapshot != expected {
        return Err(Fault::Verify);
    }
    Ok(Result {
        initial_keys,
        final_keys: cold.snapshot.keys.len(),
        commits,
        sequence: cold.sequence,
    })
}
/// Public synthetic state, for an explicit bench-only handoff. Never RF keys.
pub fn snapshot() -> core::result::Result<Snapshot, Fault> {
    let public = |n| {
        tucket::identity::LocalIdentity::from_seed([n; 32])
            .identity()
            .pub_key
    };
    let scope = Scope::new(
        public(3),
        selvage::PhyProfile::meshcore(906875000, 250000, 10, 5),
    );
    let mut keys = Vec::<[u8; 32], MAX_KEYS>::new();
    for seed in 1..=255 {
        let key = public(seed);
        if key[0] != scope.local_public[0] && !keys.iter().any(|old| old[0] == key[0]) {
            keys.push(key).map_err(|_| Fault::Capacity)?;
        }
        if keys.len() == MAX_KEYS {
            break;
        }
    }
    Snapshot::new(scope, &keys)
}
/// Explicit pre-radio handoff out of this exact historical storage fixture.
/// Other identities, keys, sequences, corruption and already blank pairs refuse.
pub fn clear_fixture(store: &mut impl Storage) -> core::result::Result<(), Fault> {
    let expected = snapshot()?;
    let old = load(store, expected.scope)?.ok_or(Fault::Verify)?;
    if old.sequence != 1 || old.snapshot != expected {
        return Err(Fault::Verify);
    }
    store.erase(Slot::A).map_err(|_| Fault::Io)?;
    store.erase(Slot::B).map_err(|_| Fault::Io)?;
    if load(store, expected.scope)?.is_some() {
        return Err(Fault::Verify);
    }
    Ok(())
}
