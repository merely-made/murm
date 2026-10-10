//! Attaching to a stock shared instance (rnsd) as a local client (`LocalInterface.py` 53-302;
//! `Transport.py` 1405-1425, 1936-1939, 3690-3709).
//!
//! Seam stub: the settings, the state slot and the ingress hook. Unit V1 (A6) implements them.

use alloc::string::String;

use std::io;

use crate::packet::Packet;

use super::interface::InterfaceId;
use super::runtime::Endpoint;
use super::shared::Shared;

/// RNS's default shared-instance TCP port (`Reticulum.py` 296-305).
pub const SHARED_INSTANCE_PORT: u16 = 37428;

/// How to reach the shared instance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SharedInstanceKind {
    /// The abstract unix socket `\0rns/<name>` on Linux and Android, TCP elsewhere, as RNS
    /// chooses (`Reticulum.py` 350-362; `platformutils.py` 55-61).
    #[default]
    Auto,
    /// TCP on 127.0.0.1 (`shared_instance_type = tcp`).
    Tcp,
    /// The abstract unix socket only.
    Unix,
}

/// A shared instance to attach to: RNS's `instance_name`, `shared_instance_port` and
/// `shared_instance_type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedInstance {
    /// Names the abstract socket `\0rns/<instance_name>`.
    pub instance_name: String,
    /// TCP port on 127.0.0.1.
    pub port: u16,
    pub kind: SharedInstanceKind,
}

impl Default for SharedInstance {
    /// RNS's defaults: instance `default`, port 37428, the platform's socket.
    fn default() -> Self {
        Self {
            instance_name: String::from("default"),
            port: SHARED_INSTANCE_PORT,
            kind: SharedInstanceKind::Auto,
        }
    }
}

/// Shared-instance state: which interfaces lead to one.
#[derive(Debug, Default)]
pub(super) struct LocalState {}

impl Shared {
    /// Adjust a packet received on `iface` before routing: a shared instance's link counts no
    /// hop (`Transport.py` 1936-1939).
    pub(super) fn local_ingress(&self, iface: InterfaceId, pkt: Packet) -> Packet {
        let _ = (&self.local, iface);
        pkt
    }
}

impl Endpoint {
    /// Attach to a shared instance as a local client, reconnecting every 8 s
    /// (`LocalInterface.py` 54, 153-185).
    pub async fn attach_shared_instance(
        &self,
        instance: SharedInstance,
    ) -> io::Result<InterfaceId> {
        let _ = instance;
        Err(io::ErrorKind::Unsupported.into())
    }
}
