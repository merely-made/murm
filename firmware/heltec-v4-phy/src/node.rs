//! The node build: the board as a standalone Reticulum transport node.
//!
//! Boot order, each step before the next: WN1's control recovery (in `main`, unchanged), this
//! boot's announce lease written to flash before receive is first armed or anything is sent,
//! the node built from the stored identity, then [`serve`] forever. A failed lease or a
//! refused channel comes back as [`NodeRefused`], and `main` keeps the modem with a fault.

#[cfg(feature = "host-uart-low-power")]
compile_error!("the node build is a USB image: `node` excludes `host-uart-low-power`");
#[cfg(feature = "rf-sleep-proof")]
compile_error!("the node build excludes the `rf-sleep-proof` bench");

use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::announce_reservation::DEFAULT_LEASE;
use radio_hand::channel::node::{Echo, NodeChannel, RADIO};
use radio_hand::node_relay::{RelaySource, transport_config};
use radio_hand::settings::Settings;
use retinue::node::{FIRST_HOP_ALLOWANCE_BITS, Node};
use static_cell::StaticCell;

use crate::control_boot::ControlReady;
use crate::radio_owner::{NodeLeaseError, V4RadioOwner};

mod bounded_host;
mod control;
mod serve;

pub use serve::serve;

/// The node's bounded tables (peers, actions per step, links): the T114's figures.
pub type V4NodeChannel = NodeChannel<32, 8, 4>;

/// The node lives here, in linker-counted static RAM: off the main stack and outside the heap
/// whose peak the bench measures. Only a reference moves through `main` and the loop.
static NODE: StaticCell<V4NodeChannel> = StaticCell::new();

/// Why the node build fell back to the modem this boot. The boot line reports only the
/// fallback; the cause is kept for inspection.
#[derive(Debug)]
#[allow(dead_code)]
pub enum NodeRefused {
    /// No lease was reserved, so nothing may announce.
    Lease(NodeLeaseError),
    /// The shared channel refused the verified lease.
    Timebase(retinue::announce::TimebaseError),
}

/// The relay source boot found (D2): a claimed board's committed flags, or unclaimed. A claimed
/// board whose state is somehow absent relays nothing rather than counting as unclaimed.
fn relay_source(ready: Option<&ControlReady>) -> RelaySource {
    let Some(ready) = ready else {
        return RelaySource::Unclaimed;
    };
    let policy = ready.runtime.state().map(|state| {
        state
            .known_good()
            .configuration
            .public
            .reticulum_transport()
    });
    RelaySource::Claimed {
        relay_announces: policy.is_some_and(|policy| policy.relay_announces),
        relay_packets: policy.is_some_and(|policy| policy.relay_packets),
    }
}

/// Reserve this boot's lease, then build the node from the stored identity, relaying as the
/// control configuration boot recovered says.
pub fn assemble<RK: RadioKind, DLY: DelayNs>(
    owner: &mut V4RadioOwner<RK, DLY>,
    settings: &Settings,
    control: Option<&ControlReady>,
) -> Result<&'static mut V4NodeChannel, NodeRefused> {
    let lease = owner
        .reserve_boot_announce_lease(DEFAULT_LEASE)
        .map_err(NodeRefused::Lease)?;
    // The link deadline's first-hop allowance follows the profile the radio runs (Ruling 70),
    // which a committed control configuration may have moved off the board default.
    let profile = owner.profile();
    let allowance = radio_hand::phy::nominal_bits_ms(
        profile.spreading_factor,
        profile.bandwidth_hz,
        profile.coding_rate_denominator,
        FIRST_HOP_ALLOWANCE_BITS,
    )
    .unwrap_or(0);
    let mut node = Node::<32, 8, 4>::new(
        retinue::identity::PrivateIdentity::from_secret_bytes(&settings.identity),
        retinue::destination::DestinationName::new("retinue", ["node"]).name_hash(),
    )
    .with_transport_config(transport_config(relay_source(control)));
    let _ = node.set_first_hop_airtime(RADIO, allowance);
    // A wall relay does not echo (D5): a peer with a link must not spend its airtime.
    let channel = NodeChannel::new(node, lease, Echo::Off).map_err(NodeRefused::Timebase)?;
    Ok(NODE.init(channel))
}
