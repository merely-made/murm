//! The router's inbound queue: every interface's received packets, tagged with the interface.
//!
//! Seam stub: one FIFO. Unit V3 replaces it with RNS's four bounded priority lanes (B4,
//! `Transport.py` 46-95, 1796-1912) behind this same interface.

use tokio::sync::mpsc;

use crate::packet::Packet;

use super::interface::InterfaceId;

/// Depth of the inbound queue. When it is full a stream reader awaits, back-pressuring the
/// peer, and a raw sink, which cannot await, drops.
const DEPTH: usize = 1024;

/// Why [`InboundTx::try_send`] did not queue a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrySendError {
    /// Its lane is full; the packet was dropped.
    Full,
    /// The router has stopped.
    Closed,
}

/// The sending half, cloned into every interface.
#[derive(Clone)]
pub(super) struct InboundTx {
    tx: mpsc::Sender<(InterfaceId, Packet)>,
}

/// A received packet, as the router takes it.
pub(super) struct Inbound {
    pub(super) iface: InterfaceId,
    pub(super) packet: Packet,
    /// Taken from the ingress-limited lane: a path request from an interface in a path-request
    /// burst, which must not start a recursive search (`Transport.py` 1859-1860, 3422, 3542).
    pub(super) ingress_limited: bool,
}

/// The router's receiving half.
pub(super) struct InboundRx {
    rx: mpsc::Receiver<(InterfaceId, Packet)>,
}

pub(super) fn channel() -> (InboundTx, InboundRx) {
    let (tx, rx) = mpsc::channel(DEPTH);
    (InboundTx { tx }, InboundRx { rx })
}

impl InboundTx {
    /// Queue a packet, waiting while its lane is full. False once the router has stopped.
    pub(super) async fn send(&self, iface: InterfaceId, pkt: Packet) -> bool {
        self.tx.send((iface, pkt)).await.is_ok()
    }

    /// Queue a packet without waiting.
    pub(super) fn try_send(&self, iface: InterfaceId, pkt: Packet) -> Result<(), TrySendError> {
        self.tx.try_send((iface, pkt)).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => TrySendError::Full,
            mpsc::error::TrySendError::Closed(_) => TrySendError::Closed,
        })
    }
}

impl InboundRx {
    /// The next packet to route; `None` once every sender is gone.
    pub(super) async fn recv(&mut self) -> Option<Inbound> {
        let (iface, packet) = self.rx.recv().await?;
        Some(Inbound {
            iface,
            packet,
            ingress_limited: false,
        })
    }
}
