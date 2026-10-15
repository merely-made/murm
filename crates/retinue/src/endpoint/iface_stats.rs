//! Per-interface traffic counters (`Interface.py` 98-180, 303-336; `Reticulum.py` 1498-1660).
//!
//! `rx_bytes` and `tx_bytes` count deframed carrier frames with their access code. That
//! matches stock's `rxb`; stock's `txb` also counts the carrier's framing (`TCPInterface.py`
//! 320-329), so retinue's `tx_bytes` equals the peer's `rxb`, not its own `txb`.

use core::fmt;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::hash::AddressHash;
use crate::node::InterfaceMode;
use crate::packet::{DestinationType, Packet, PacketType};
use crate::path::path_request_destination;

use super::interface::InterfaceId;
use super::runtime::Endpoint;

/// One interface's traffic since it was attached, as RNS's `get_interface_stats` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterfaceCounters {
    /// Received deframed frames and their bytes, access code included (`rxb`).
    pub rx_bytes: u64,
    pub rx_packets: u64,
    /// Transmitted frames and their bytes, access code included but framing not, unlike
    /// stock's `txb`: this equals the receiving peer's `rxb`.
    pub tx_bytes: u64,
    pub tx_packets: u64,
    /// Received announces and their bytes, without the access code (`arxc`, `arxb`).
    pub announces_rx: u64,
    pub announce_rx_bytes: u64,
    /// Transmitted announces (`atxc`, `atxb`).
    pub announces_tx: u64,
    pub announce_tx_bytes: u64,
    /// Received path requests (`prxc`, `prxb`).
    pub path_requests_rx: u64,
    pub path_request_rx_bytes: u64,
    /// Transmitted path requests (`ptxc`, `ptxb`).
    pub path_requests_tx: u64,
    pub path_request_tx_bytes: u64,
    /// Frames whose access code was missing, wrong or forged, or that carried the IFAC flag
    /// on an interface without one.
    pub ifac_violations: u64,
    pub protocol_violations: u64,
    /// Queued packets dropped before the wire: a full class queue, an offline carrier's
    /// backlog, or a packet the carrier could not encode (`txdrp`).
    pub tx_dropped: u64,
    /// Announces held while the interface bursts (`held_announces`).
    pub held_announces: u64,
    /// The interface is in an announce burst (`burst_active`).
    pub burst_active: bool,
    /// The interface is in a path-request burst (`pr_burst_active`).
    pub path_request_burst: bool,
    /// The carrier is up (`status`).
    pub online: bool,
    pub mode: InterfaceMode,
    /// The largest frame the interface admits, access code included.
    pub frame_limit: usize,
}

/// One JSON object, keys as the field names.
impl fmt::Display for InterfaceCounters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts = [
            ("rx_bytes", self.rx_bytes),
            ("rx_packets", self.rx_packets),
            ("tx_bytes", self.tx_bytes),
            ("tx_packets", self.tx_packets),
            ("announces_rx", self.announces_rx),
            ("announce_rx_bytes", self.announce_rx_bytes),
            ("announces_tx", self.announces_tx),
            ("announce_tx_bytes", self.announce_tx_bytes),
            ("path_requests_rx", self.path_requests_rx),
            ("path_request_rx_bytes", self.path_request_rx_bytes),
            ("path_requests_tx", self.path_requests_tx),
            ("path_request_tx_bytes", self.path_request_tx_bytes),
            ("ifac_violations", self.ifac_violations),
            ("protocol_violations", self.protocol_violations),
            ("tx_dropped", self.tx_dropped),
            ("held_announces", self.held_announces),
            ("frame_limit", self.frame_limit as u64),
        ];
        f.write_str("{")?;
        for (key, value) in counts {
            write!(f, "\"{key}\":{value},")?;
        }
        write!(
            f,
            "\"burst_active\":{},\"path_request_burst\":{},\"online\":{},\"mode\":\"{}\"}}",
            self.burst_active,
            self.path_request_burst,
            self.online,
            mode_name(self.mode)
        )
    }
}

/// RNS's configuration name for `mode` (`Reticulum.py` 845-880), a fixed JSON-safe token.
fn mode_name(mode: InterfaceMode) -> &'static str {
    match mode {
        InterfaceMode::Full => "full",
        InterfaceMode::PointToPoint => "pointtopoint",
        InterfaceMode::AccessPoint => "accesspoint",
        InterfaceMode::Roaming => "roaming",
        InterfaceMode::Boundary => "boundary",
        InterfaceMode::Gateway => "gateway",
        InterfaceMode::Internal => "internal",
    }
}

/// The live counter cells of one interface, shared by its record, its sink and its queues.
#[derive(Debug, Default)]
pub(super) struct InterfaceStats {
    rx_bytes: AtomicU64,
    rx_packets: AtomicU64,
    tx_bytes: AtomicU64,
    tx_packets: AtomicU64,
    announce_rx: [AtomicU64; 2],
    announce_tx: [AtomicU64; 2],
    path_request_rx: [AtomicU64; 2],
    path_request_tx: [AtomicU64; 2],
    /// Frames whose access code was missing, wrong or forged (`Transport.py` 1272-1322).
    ifac_violations: AtomicU64,
    /// Frames that did not decode as a packet, or exceeded the interface's frame limit.
    protocol_violations: AtomicU64,
    /// Set by path-request ingress control while the interface bursts (`Interface.py`
    /// 213-239); the inbound queue gives its path requests the ingress-limited lane.
    pub(super) path_request_burst: AtomicBool,
}

/// Add one packet of `bytes` to a `[count, bytes]` pair.
fn tally(cell: &[AtomicU64; 2], bytes: usize) {
    cell[0].fetch_add(1, Ordering::Relaxed);
    cell[1].fetch_add(bytes as u64, Ordering::Relaxed);
}

/// A path request: plain data to the well-known path-request destination (`Transport.py`
/// 1827). Its hash is computed once.
pub(super) fn is_path_request(pkt: &Packet) -> bool {
    static DESTINATION: OnceLock<AddressHash> = OnceLock::new();
    pkt.packet_type == PacketType::Data
        && pkt.destination_type == DestinationType::Plain
        && pkt.destination == *DESTINATION.get_or_init(path_request_destination)
}

impl InterfaceStats {
    /// A frame of `bytes` arrived, before the access code was checked.
    pub(super) fn note_rx(&self, bytes: usize) {
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
    }

    /// `packet` left as a frame of `bytes`, access code included.
    pub(super) fn note_tx(&self, packet: &Packet, bytes: usize) {
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
        if packet.packet_type == PacketType::Announce {
            tally(&self.announce_tx, packet.encoded_len());
        } else if is_path_request(packet) {
            tally(&self.path_request_tx, packet.encoded_len());
        }
    }

    /// An announce of `bytes`, without the access code, was classified for the router.
    pub(super) fn note_announce_rx(&self, bytes: usize) {
        tally(&self.announce_rx, bytes);
    }

    pub(super) fn note_path_request_rx(&self, bytes: usize) {
        tally(&self.path_request_rx, bytes);
    }

    pub(super) fn note_ifac_violation(&self) {
        self.ifac_violations.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn note_protocol_violation(&self) {
        self.protocol_violations.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> InterfaceCounters {
        let load = |cell: &AtomicU64| cell.load(Ordering::Relaxed);
        let pair = |cell: &[AtomicU64; 2]| (load(&cell[0]), load(&cell[1]));
        let (announces_rx, announce_rx_bytes) = pair(&self.announce_rx);
        let (announces_tx, announce_tx_bytes) = pair(&self.announce_tx);
        let (path_requests_rx, path_request_rx_bytes) = pair(&self.path_request_rx);
        let (path_requests_tx, path_request_tx_bytes) = pair(&self.path_request_tx);
        InterfaceCounters {
            rx_bytes: load(&self.rx_bytes),
            rx_packets: load(&self.rx_packets),
            tx_bytes: load(&self.tx_bytes),
            tx_packets: load(&self.tx_packets),
            announces_rx,
            announce_rx_bytes,
            announces_tx,
            announce_tx_bytes,
            path_requests_rx,
            path_request_rx_bytes,
            path_requests_tx,
            path_request_tx_bytes,
            ifac_violations: load(&self.ifac_violations),
            protocol_violations: load(&self.protocol_violations),
            path_request_burst: self.path_request_burst.load(Ordering::Relaxed),
            ..InterfaceCounters::default()
        }
    }
}

impl Endpoint {
    /// Interface `id`'s traffic counters and state; `None` if it is not attached.
    pub fn interface_counters(&self, id: InterfaceId) -> Option<InterfaceCounters> {
        let shared = &self.shared;
        let mut counters = shared.with_iface(id, |iface| {
            let (_, dropped) = iface.outbound.counters();
            InterfaceCounters {
                tx_dropped: dropped.iter().sum::<u64>() + iface.unsendable.load(Ordering::Relaxed),
                online: iface.online.load(Ordering::Acquire),
                mode: iface.mode,
                frame_limit: iface.frame_limit.load(Ordering::Acquire),
                ..iface.stats.snapshot()
            }
        })?;
        counters.held_announces = shared.held_announces.lock().unwrap().held(id) as u64;
        let now = shared.announce_admission_now_ms();
        counters.burst_active = shared
            .announce_admission
            .lock()
            .unwrap()
            .bursting(id, now);
        Some(counters)
    }
}
