//! Per-interface traffic counters (`Interface.py` 98-180; `Reticulum.py` 1498-1660).
//!
//! Seam: the cells every carrier feeds. Unit V3 (C2) adds announce and path-request counts,
//! the snapshot type and `Endpoint::interface_stats`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::interface::InterfaceId;
use super::runtime::Endpoint;

/// One interface's traffic since it was attached, as RNS's `get_interface_stats` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterfaceCounters {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub ifac_violations: u64,
    pub protocol_violations: u64,
    /// The interface is in a path-request burst (`ic_pr_burst_active`).
    pub path_request_burst: bool,
}

/// The live counter cells of one interface, shared by its record, its sink and its queues.
#[derive(Debug, Default)]
pub(super) struct InterfaceStats {
    pub(super) rx_bytes: AtomicU64,
    pub(super) rx_packets: AtomicU64,
    pub(super) tx_bytes: AtomicU64,
    pub(super) tx_packets: AtomicU64,
    /// Frames whose access code was missing, wrong or forged (`Transport.py` 1272-1322).
    pub(super) ifac_violations: AtomicU64,
    /// Frames that did not decode as a packet, or exceeded the interface's frame limit.
    pub(super) protocol_violations: AtomicU64,
    /// Set by path-request ingress control while the interface bursts (`Interface.py`
    /// 213-239); the inbound queue gives its path requests the ingress-limited lane.
    pub(super) path_request_burst: AtomicBool,
}

impl InterfaceStats {
    /// A frame of `bytes` arrived, before the access code was checked.
    pub(super) fn note_rx(&self, bytes: usize) {
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
    }

    /// A frame of `bytes` left, access code included.
    pub(super) fn note_tx(&self, bytes: usize) {
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn note_ifac_violation(&self) {
        self.ifac_violations.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn note_protocol_violation(&self) {
        self.protocol_violations.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> InterfaceCounters {
        let load = |cell: &AtomicU64| cell.load(Ordering::Relaxed);
        InterfaceCounters {
            rx_bytes: load(&self.rx_bytes),
            rx_packets: load(&self.rx_packets),
            tx_bytes: load(&self.tx_bytes),
            tx_packets: load(&self.tx_packets),
            ifac_violations: load(&self.ifac_violations),
            protocol_violations: load(&self.protocol_violations),
            path_request_burst: self.path_request_burst.load(Ordering::Relaxed),
        }
    }
}

impl Endpoint {
    /// Interface `id`'s traffic counters; `None` if it is not attached.
    pub fn interface_counters(&self, id: InterfaceId) -> Option<InterfaceCounters> {
        self.shared.with_iface(id, |iface| iface.stats.snapshot())
    }
}
