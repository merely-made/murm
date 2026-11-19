//! Bounded transport tables and small helpers shared across the node.

use alloc::vec::Vec;

use heapless::Vec as BoundedVec;

use crate::hash::AddressHash;
use crate::link;
use crate::transport::{Keyed, LinkBridge, ReverseEntry, RouteEntry, Slot, TransitMemory};

/// A route as the board keeps it: the shared entry, its destination, and the authenticated
/// link cost from its most recent announce, which a new route starts without.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RouteSlot {
    pub(super) key: AddressHash,
    pub(super) entry: RouteEntry<u64>,
    pub(super) link_cost: Option<crate::stamp::StampCost>,
}

impl Slot for RouteSlot {
    type Entry = RouteEntry<u64>;
    fn key(&self) -> AddressHash {
        self.key
    }
    fn entry(&self) -> &RouteEntry<u64> {
        &self.entry
    }
    fn entry_mut(&mut self) -> &mut RouteEntry<u64> {
        &mut self.entry
    }
    fn new(key: AddressHash, entry: RouteEntry<u64>) -> Self {
        Self {
            key,
            entry,
            link_cost: None,
        }
    }
    fn into_entry(self) -> RouteEntry<u64> {
        self.entry
    }
}

/// Learned routes, bounded by `ROUTES`.
pub(super) type Routes<const N: usize> = BoundedVec<RouteSlot, N>;
/// Carried links, keyed by link id.
pub(super) type Bridges<const N: usize> = BoundedVec<Keyed<LinkBridge<u64>>, N>;
/// Return paths, keyed by the truncated packet hash a proof is addressed to.
pub(super) type ReversePaths<const N: usize> = BoundedVec<Keyed<ReverseEntry<u64>>, N>;
/// A two-generation hash memory of `N` hashes a generation.
pub(super) type HashMemory<const N: usize> = TransitMemory<BoundedVec<AddressHash, N>, u64>;

/// One derived resource IV: `full_hash(tag || identity secret || link id || counter)`.
///
/// Deterministic on purpose — this layer holds no RNG — and unique by the counter, which
/// the node owns and never resets.
pub(super) fn derived_iv(
    seed: &[u8; 64],
    link_id: AddressHash,
    counter: &mut u32,
) -> [u8; crate::token::IV_LEN] {
    *counter = counter.wrapping_add(1);
    let mut input = Vec::with_capacity(48);
    input.extend_from_slice(b"retinue/node/resource-iv");
    input.extend_from_slice(seed);
    input.extend_from_slice(link_id.as_slice());
    input.extend_from_slice(&counter.to_le_bytes());
    let digest = crate::hash::full_hash(&input);
    let mut out = [0_u8; crate::token::IV_LEN];
    out.copy_from_slice(&digest[..crate::token::IV_LEN]);
    out
}

/// Whether a link-packet context belongs to a resource transfer.
pub(super) fn is_resource_context(context: u8) -> bool {
    matches!(
        context,
        link::CTX_RESOURCE
            | link::CTX_RESOURCE_ADV
            | link::CTX_RESOURCE_REQ
            | link::CTX_RESOURCE_HMU
            | link::CTX_RESOURCE_PRF
            | link::CTX_RESOURCE_ICL
            | link::CTX_RESOURCE_RCL
    )
}

/// Whether a link packet with this context is checked against the sent and received
/// windows. Resource contexts are not: a transfer has its own part and request bookkeeping,
/// and a re-sent part legitimately repeats its hash. Keepalives are not: every request on a
/// link is the same unencrypted byte, so each one repeats the last one's hash. Nor are cache
/// requests, which a sender repeats verbatim and RNS's packet filter also lets through.
pub(crate) fn is_deduplicated_link_context(context: u8) -> bool {
    !is_resource_context(context)
        && context != link::CTX_KEEPALIVE
        && context != link::CTX_CACHE_REQUEST
}

/// Remember a packet hash in a window, forgetting the oldest at capacity. A burst can outrun
/// the window; that only lets a late copy through, never drops a new packet.
pub(super) fn remember_hash<const N: usize>(
    window: &mut BoundedVec<AddressHash, N>,
    hash: AddressHash,
) {
    if window.is_full() {
        window.remove(0);
    }
    let _ = window.push(hash);
}
