//! The node build's boot lease, taken through the owner so it cannot land beside live radio.

use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::announce_reservation::ActiveLease;

use super::V4RadioOwner;
use crate::store::ReservationError;

/// Why this boot has no announce lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLeaseError {
    /// Receive was armed or a frame was sent: flash work is no longer before radio service.
    RadioServiceStarted,
    Store(ReservationError),
}

impl<RK: RadioKind, DLY: DelayNs> V4RadioOwner<RK, DLY> {
    /// Reserve and verify this boot's announce ordinals. Erase and write stall the chip, so
    /// this is refused once receive has been armed or anything has been transmitted.
    pub fn reserve_boot_announce_lease(
        &mut self,
        lease: u64,
    ) -> Result<ActiveLease, NodeLeaseError> {
        if self.receive_armed_once || self.diag.tx_ok != 0 || self.diag.tx_err != 0 {
            return Err(NodeLeaseError::RadioServiceStarted);
        }
        self.store
            .reserve_announce_lease(lease)
            .map_err(NodeLeaseError::Store)
    }
}
