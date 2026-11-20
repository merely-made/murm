//! What the radio is running, not what was asked for (wall node D20).
//!
//! The `applied` line and the `sync` probe, both unsigned and served at a loop's boundary
//! like `air`, in the modem, RNode and node loops alike. They read the owner's confirmed
//! profile, so they follow boot recovery, provisional apply, commit, revert, expiry and
//! excursions without a path of their own.

use core::fmt::Write as _;

use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::applied::{AppliedRadio, Relay};
use radio_hand::link::HostLink;

use super::V4RadioOwner;

impl<RK: RadioKind, DLY: DelayNs> V4RadioOwner<RK, DLY> {
    /// The profile in force with its clamped power, and in the node build the relay flags.
    pub fn applied(&self) -> AppliedRadio {
        #[cfg(feature = "node")]
        let relay = match self.relay_in_force() {
            radio_hand::node_relay::RelaySource::Unclaimed => Relay::Unclaimed,
            radio_hand::node_relay::RelaySource::Claimed {
                relay_announces,
                relay_packets,
            } => Relay::Claimed {
                announces: relay_announces,
                packets: relay_packets,
            },
        };
        #[cfg(not(feature = "node"))]
        let relay = Relay::NotServed;
        AppliedRadio::new(self.region, &self.radio.profile, relay)
    }

    /// Serve `applied` or `sync`; `false` for any other packet.
    pub(super) async fn probe_applied<L: HostLink>(&self, packet: &[u8], host: &mut L) -> bool {
        let applied = self.applied();
        if radio_hand::applied::is_query(packet) {
            let _ = host.write_all(applied.render().as_str().as_bytes()).await;
            return true;
        }
        if packet == b"sync\n" || packet == b"sync\r\n" {
            let mut reply = radio_face::Text::<16>::empty();
            let _ = write!(
                &mut reply,
                "{:02x} {:04x}\r\n",
                applied.sync_word,
                applied.sync_register()
            );
            let _ = host.write_all(reply.as_str().as_bytes()).await;
            return true;
        }
        false
    }
}
