//! Tunnel synthesis: a reconnecting client's paths are kept for it by the node it rejoins
//! (`TCPInterface.py` 169-180; `Transport.py` 2764-2860).
//!
//! Seam stub: the state slot and the hooks the router, the path table and interface removal
//! call. Unit V1 (B6) implements them.

use crate::hash::AddressHash;
use crate::packet::Packet;

use super::interface::InterfaceId;
use super::paths::PathEntry;
use super::shared::Shared;

/// Tunnels this node keeps for its clients, by tunnel id.
#[derive(Debug, Default)]
pub(super) struct Tunnels {}

impl Shared {
    /// Handle `pkt` if it is a tunnel synthesis; true if it was one and is consumed.
    pub(super) fn tunnel_intercept(&self, iface: InterfaceId, pkt: &Packet) -> bool {
        let _ = (&self.tunnels, iface, pkt);
        false
    }

    /// `dest`'s route was just learned or replaced as `entry`. Called outside the path-table
    /// lock.
    pub(super) fn tunnel_path_learned(&self, dest: AddressHash, entry: &PathEntry) {
        let _ = (&self.tunnels, dest, entry);
    }

    /// Interface `id` is gone; its routes have been culled.
    pub(super) fn tunnel_interface_lost(&self, id: InterfaceId) {
        let _ = (&self.tunnels, id);
    }
}
