//! Public contact writes occur before the next protocol tick/transmission.
use super::*;
use radio_hand::tucket_contacts::{MAX_KEYS, Snapshot};

pub(super) async fn commit<RK: RadioKind, DLY: DelayNs>(
    state: &mut ResidentState,
    owner: &mut V4RadioOwner<RK, DLY>,
) -> bool {
    let mut keys = heapless::Vec::<[u8; 32], MAX_KEYS>::new();
    for key in state.runtime.tucket().node().contact_keys() {
        keys.push(key)
            .unwrap_or_else(|_| esp_hal::system::software_reset());
    }
    keys.sort_unstable();
    if keys == state.cache.keys {
        return true;
    }
    let snapshot = Snapshot::new(state.cache.scope, &keys)
        .unwrap_or_else(|_| esp_hal::system::software_reset());
    // A late completed frame must be collected before entry, just like a
    // profile transition. Defer the save AND all further protocol TX until
    // that boundary has been drained. No unsaved contact may enable a send.
    if owner.quiet_preflight() == V4QuietPreflight::CompletedFramePending {
        return false;
    }
    owner
        .ensure_rx()
        .await
        .unwrap_or_else(|_| esp_hal::system::software_reset());
    owner
        .save_tucket_contacts(&snapshot)
        .await
        .unwrap_or_else(|_| esp_hal::system::software_reset());
    state.cache = snapshot;
    true
}
