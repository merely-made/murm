//! Configurable stamp policy for the executor-neutral node.

use super::Node;
use crate::network_stamp::{self, NetworkStampPolicy};
use crate::stamp::{StampCost, detached_valid};

/// Bounded synchronous work for a default Node link attempt. Hosts may raise or
/// lower it through NetworkStampPolicy and supply a clock-aware cancellation check.
pub const DEFAULT_LINK_STAMP_ATTEMPTS: u64 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeAnnounceError {
    AppDataTooLarge,
    StampUnavailable,
}

impl core::fmt::Display for NodeAnnounceError {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        out.write_str(match self {
            Self::AppDataTooLarge => "announce exceeds logical MTU",
            Self::StampUnavailable => {
                "destination stamp has not been provisioned at the configured value"
            }
        })
    }
}

impl core::error::Error for NodeAnnounceError {}

impl<const PEERS: usize, const ACTIONS: usize, const LINKS: usize, const ROUTES: usize>
    Node<PEERS, ACTIONS, LINKS, ROUTES>
{
    pub fn network_stamp_policy(&self) -> NetworkStampPolicy {
        self.stamp_policy
    }

    /// Set instance stamp policy. The shell provisions a destination stamp separately.
    /// Mining in this clock-free layer is bounded by attempts; a host that needs a wall
    /// clock timeout should mine with the cancellable network_stamp helpers instead.
    pub fn set_network_stamp_policy(&mut self, policy: NetworkStampPolicy) {
        self.stamp_policy = policy;
        self.retry_announce();
    }

    /// Install a precomputed destination stamp only if it meets the configured target.
    /// This lets constrained nodes use an externally mined public stamp.
    pub fn set_destination_stamp(&mut self, stamp: [u8; network_stamp::STAMP_LEN]) -> bool {
        if !detached_valid(
            &self.destination_stamp_material(),
            &stamp,
            self.stamp_policy.destination_value,
        ) {
            return false;
        }
        self.destination_stamp = Some(stamp);
        self.retry_announce();
        true
    }

    pub fn destination_stamp_material(&self) -> [u8; 90] {
        network_stamp::destination_material(
            self.destination(),
            self.identity.public(),
            self.name_hash,
        )
    }

    pub(super) fn local_stamp_valid(&self) -> bool {
        self.destination_stamp.is_some_and(|stamp| {
            detached_valid(
                &self.destination_stamp_material(),
                &stamp,
                self.stamp_policy.destination_value,
            )
        })
    }

    pub(super) fn refuse_stamp(&mut self) {
        self.transport_counters.refused_stamps =
            self.transport_counters.refused_stamps.saturating_add(1);
    }

    pub(super) fn link_seed(
        &self,
        destination: crate::hash::AddressHash,
        seed: &[u8; 64],
        now: u64,
        should_cancel: impl FnMut() -> bool,
    ) -> Option<[u8; 64]> {
        let cost = self
            .routes
            .iter()
            .find(|route| {
                route.destination == destination && route.live(now, self.transport.route_ttl)
            })
            .and_then(|route| route.link_cost);
        let required = if self.stamp_policy.stamp_link_requests {
            cost.or(self
                .stamp_policy
                .always_stamp_links
                .then_some(self.stamp_policy.link_cost))
        } else {
            None
        };
        let Some(required) = required else {
            return Some(*seed);
        };
        if required == StampCost::ZERO {
            return Some(*seed);
        }
        let mut counter = 0_u64;
        network_stamp::generate_link_seed(
            destination,
            *seed,
            required,
            self.stamp_policy.max_attempts,
            |out| {
                // The caller supplies a fresh unpredictable seed per attempt. Derive each
                // candidate from that secret, with a domain and a non-repeating ordinal.
                use sha2::{Digest, Sha256};
                counter += 1;
                let candidate = Sha256::new()
                    .chain_update(b"retinue-node-lr-stamp-v1")
                    .chain_update(seed)
                    .chain_update(counter.to_be_bytes())
                    .finalize();
                out.copy_from_slice(&candidate[..out.len()]);
            },
            should_cancel,
        )
    }
}
