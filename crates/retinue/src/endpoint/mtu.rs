//! Per-interface hardware MTU and link MTU signalling (`Interface.py` 94-95, 251-265;
//! `Transport.py` 2063-2088, 2543-2563, 3173-3178).
//!
//! Seam: every interface still carries at most the protocol MTU and links are offered as
//! before. Unit V6 (L1) makes [`HwMtu`] set the frame limit and the signalled link MTU.

use core::sync::atomic::Ordering;

use crate::link::{self, LinkTrailer};
use crate::packet::Packet;

use super::interface::InterfaceId;
use super::shared::Shared;

/// How an interface's hardware MTU is set, as RNS's `HW_MTU`, `AUTOCONFIGURE_MTU` and
/// `FIXED_MTU`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HwMtu {
    /// No hardware MTU of its own: frames and links stay at the 500-byte protocol MTU.
    #[default]
    Protocol,
    /// A carrier-fixed MTU, signalled to links (`FIXED_MTU`): AutoInterface's 1196.
    Fixed(usize),
    /// Derived from the bitrate and signalled to links (`AUTOCONFIGURE_MTU`): TCP guesses
    /// 10 Mbit/s, the shared instance 1 Gbit/s.
    Auto {
        /// The interface's bitrate in bits per second.
        bitrate: u64,
    },
}

impl HwMtu {
    /// The complete-frame limit for a carrier that can take `max_frame_len` bytes, with
    /// `overhead` bytes of access code.
    pub(super) fn frame_limit(self, max_frame_len: usize, overhead: usize) -> usize {
        max_frame_len.min(crate::packet::MTU + overhead)
    }
}

/// The MTU a responder offers for a link request heard on `iface`: the configured link MTU,
/// no larger than the interface, and no larger than the request signals (`Transport.py`
/// 2063-2088; a signalled 0 means the default, a smaller request is floored).
pub(super) fn link_request_mtu(shared: &Shared, iface: InterfaceId, pkt: &Packet) -> u32 {
    let configured = shared.link_mtu.load(Ordering::Relaxed);
    let configured = shared
        .link_mtu_on(iface)
        .map_or(configured, |limit| configured.min(limit));
    let requested = pkt
        .payload
        .get(link::LINK_KEYS_LEN..link::LINK_KEYS_LEN + link::TRAILER_LEN)
        .and_then(|bytes| bytes.try_into().ok())
        .and_then(|trailer| LinkTrailer::decode(trailer).ok())
        .map(|trailer| trailer.mtu)
        .filter(|&mtu| mtu != 0)
        .map_or(configured, |mtu| mtu.max(crate::node::MIN_LOGICAL_MTU));
    configured.min(requested)
}
