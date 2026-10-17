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
    let public = |n| {
        tucket::identity::LocalIdentity::from_seed([n; 32])
            .identity()
            .pub_key
    };
    let scope = Scope::new(
        public(3),
        selvage::PhyProfile::meshcore(906875000, 250000, 10, 5),
    );
    let initial = load(store, scope)?;
    let initial_keys = initial.as_ref().map(|s| s.snapshot.keys.len()).unwrap_or(0);
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
    let expected = Snapshot::new(scope, &keys)?;
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
