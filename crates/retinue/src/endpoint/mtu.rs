//! Per-interface hardware MTU and link MTU signalling (`Interface.py` 94-95, 251-265;
//! `Transport.py` 2063-2088, 2543-2563, 3173-3178; `Link.py` 300-315).
//!
//! Only links carry packets past the 500-byte protocol MTU, and only when every hop signalled
//! it: an initiator offers what its next hop signals, transit hops and the responder clamp.

use core::sync::atomic::Ordering;

use crate::link::{self, LinkTrailer};
use crate::packet::{MTU, Packet};

use super::interface::InterfaceId;
use super::shared::Shared;

/// How an interface's hardware MTU is set, as RNS's `HW_MTU`, `AUTOCONFIGURE_MTU` and
/// `FIXED_MTU`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HwMtu {
    /// Not signalled to links (UDP, Pipe, serial and radio carriers): links stay at the
    /// 500-byte protocol MTU and the carrier's own frame limit bounds frames.
    #[default]
    Protocol,
    /// A carrier-fixed MTU, signalled to links (`FIXED_MTU`): AutoInterface's 1196, or a TCP
    /// client's `fixed_mtu`.
    Fixed(usize),
    /// Derived from the bitrate by RNS's `optimise_mtu` table and signalled to links
    /// (`AUTOCONFIGURE_MTU`): TCP guesses 10 Mbit/s, Backbone 100 Mbit/s, Local 1 Gbit/s.
    Auto {
        /// The interface's bitrate in bits per second.
        bitrate: u64,
    },
}

/// RNS's `optimise_mtu` table (`Interface.py` 251-265). Below 62.5 kbit/s RNS sets `HW_MTU`
/// to `None`, which later raises on every frame; here the protocol MTU stands instead.
pub const fn autoconfig_mtu(bitrate: u64) -> usize {
    const TABLE: [(u64, usize); 10] = [
        (1_000_000_000, 524_288),
        (750_000_000, 262_144),
        (400_000_000, 131_072),
        (200_000_000, 65_536),
        (100_000_000, 32_768),
        (10_000_000, 16_384),
        (5_000_000, 8_192),
        (2_000_000, 4_096),
        (1_000_000, 2_048),
        (62_500, 1_024),
    ];
    let mut i = 0;
    while i < TABLE.len() {
        if bitrate >= TABLE[i].0 {
            return TABLE[i].1;
        }
        i += 1;
    }
    MTU
}

impl HwMtu {
    /// The largest packet this interface carries, before any access code.
    pub fn mtu(self) -> usize {
        match self {
            Self::Protocol => MTU,
            Self::Fixed(mtu) => mtu,
            Self::Auto { bitrate } => autoconfig_mtu(bitrate),
        }
    }

    /// Whether links learn this MTU (`Transport.py` 3173-3178).
    pub fn signalled(self) -> bool {
        !matches!(self, Self::Protocol)
    }

    /// The complete-frame limit with `overhead` bytes of access code. `max_frame_len` caps a
    /// protocol or fixed carrier; an autoconfigured one is a stream whose framing has no
    /// bound of its own, so its MTU alone sets the limit.
    pub(super) fn frame_limit(self, max_frame_len: usize, overhead: usize) -> usize {
        let limit = self.mtu() + overhead;
        match self {
            Self::Auto { .. } => limit,
            _ => max_frame_len.min(limit),
        }
    }
}

/// The MTU a responder offers for a link request heard on `iface` (`Transport.py`
/// 2543-2563; `Link.py` 194): the request's signalled MTU, or the protocol MTU when it
/// signals none or 0, no larger than the interface signals or the configured ceiling. A
/// smaller request is floored at [`crate::node::MIN_LOGICAL_MTU`].
pub(super) fn link_request_mtu(shared: &Shared, iface: InterfaceId, pkt: &Packet) -> u32 {
    let requested = pkt
        .payload
        .get(link::LINK_KEYS_LEN..)
        .and_then(|bytes| <&[u8; link::TRAILER_LEN]>::try_from(bytes).ok())
        .and_then(|trailer| LinkTrailer::decode(trailer).ok())
        .map(|trailer| trailer.mtu)
        .filter(|&mtu| mtu != 0)
        .map_or(MTU as u32, |mtu| mtu.max(crate::node::MIN_LOGICAL_MTU));
    let ceiling = shared.link_mtu.load(Ordering::Relaxed);
    requested
        .min(ceiling)
        .min(shared.link_mtu_on(iface).unwrap_or(MTU as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoconfig_follows_rns_table() {
        assert_eq!(autoconfig_mtu(10_000_000), 16_384);
        assert_eq!(autoconfig_mtu(100_000_000), 32_768);
        assert_eq!(autoconfig_mtu(1_000_000_000), 524_288);
        assert_eq!(autoconfig_mtu(62_500), 1_024);
        // RNS's HW_MTU None is not ported.
        assert_eq!(autoconfig_mtu(62_499), MTU);
        assert_eq!(autoconfig_mtu(0), MTU);
    }

    #[test]
    fn frame_limits_per_policy() {
        assert_eq!(HwMtu::Protocol.frame_limit(564, 16), 516);
        assert_eq!(HwMtu::Protocol.frame_limit(255, 8), 255);
        assert_eq!(HwMtu::Fixed(1196).frame_limit(usize::MAX, 16), 1212);
        assert_eq!(HwMtu::Fixed(1196).frame_limit(1000, 0), 1000);
        let tcp = HwMtu::Auto {
            bitrate: 10_000_000,
        };
        assert_eq!(tcp.frame_limit(crate::iface::hdlc::MAX_FRAME, 16), 16_400);
        assert!(tcp.signalled() && !HwMtu::Protocol.signalled());
    }
}
