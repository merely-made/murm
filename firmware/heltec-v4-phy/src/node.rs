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
use radio_hand::channel::node::{Echo, NodeChannel};
use radio_hand::settings::Settings;
use retinue::node::Node;
use static_cell::StaticCell;

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

/// Reserve this boot's lease, then build the node from the stored identity. Its relay flags
/// and first-hop allowance come from the radio owner's configuration in force, the same place
/// the node loop reads after every command and expiry (D9): a claimed board's recovered
/// configuration, or relay both on an unclaimed board (D2).
pub fn assemble<RK: RadioKind, DLY: DelayNs>(
    owner: &mut V4RadioOwner<RK, DLY>,
    settings: &Settings,
) -> Result<&'static mut V4NodeChannel, NodeRefused> {
    let lease = owner
        .reserve_boot_announce_lease(DEFAULT_LEASE)
        .map_err(NodeRefused::Lease)?;
    let node = Node::<32, 8, 4>::new(
        retinue::identity::PrivateIdentity::from_secret_bytes(&settings.identity),
        retinue::destination::DestinationName::new("retinue", ["node"]).name_hash(),
    );
    // A wall relay does not echo (D5) and refuses every resource addressed to it (D8): a peer
    // with a link must not spend its airtime or its heap.
    let channel = NodeChannel::new(node, lease, Echo::Off, 0).map_err(NodeRefused::Timebase)?;
    let channel = NODE.init(channel);
    // Boot's own apply is followed here, so the loop starts with no change pending.
    owner.take_config_change();
    channel.follow(owner.relay_in_force(), &owner.profile());
    Ok(channel)
}
