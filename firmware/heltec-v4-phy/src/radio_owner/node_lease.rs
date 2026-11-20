//! The node build's owner facts: the boot lease, taken through the owner so it cannot land
//! beside live radio, and the configuration in force (D9), kept where every apply passes.

use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::announce_reservation::ActiveLease;
use radio_hand::control::DurableConfig;
use radio_hand::node_relay::RelaySource;

use super::V4RadioOwner;
use crate::store::ReservationError;

/// Why this boot has no announce lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLeaseError {
    /// Receive was armed or a frame was sent: flash work is no longer before radio service.
    RadioServiceStarted,
    Store(ReservationError),
}

/// The relay flags of the last configuration the owner applied. Boot recovery, provisional
/// apply, revert and expiry all apply through the owner; an unclaimed board applies none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InForce {
    relay: RelaySource,
    changed: bool,
}

impl Default for InForce {
    fn default() -> Self {
        Self {
            relay: RelaySource::Unclaimed,
            changed: false,
        }
    }
}

impl InForce {
    pub(super) fn applied(&mut self, configuration: &DurableConfig) {
        let transport = configuration.public.reticulum_transport();
        self.relay = RelaySource::Claimed {
            relay_announces: transport.relay_announces,
            relay_packets: transport.relay_packets,
        };
        self.changed = true;
    }
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

    /// The relay flags in force: the last applied configuration's, or unclaimed.
    pub fn relay_in_force(&self) -> RelaySource {
        self.in_force.relay
    }

    /// Whether a configuration was applied since the last call. The node loop follows it.
    pub fn take_config_change(&mut self) -> bool {
        core::mem::take(&mut self.in_force.changed)
    }
}
