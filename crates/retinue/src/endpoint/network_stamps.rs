//! Bounded network stamp generation and admission for host endpoints.
use super::entropy::{ephemeral_seed, fill_random};
use super::runtime::{Endpoint, track};
use super::shared::Shared;
use crate::announce::Announce;
use crate::network_stamp::{
    NetworkStamp, NetworkStampPolicy, destination_material, generate_detached, generate_link_seed,
    path_material,
};
use crate::stamp::StampCost;
use crate::{AddressHash, DestinationName};
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::Instant;

pub(super) struct NetworkStamps {
    pub policy: NetworkStampPolicy,
    pub own: HashMap<AddressHash, NetworkStamp>,
    pub costs: HashMap<AddressHash, StampCost>,
    pub peer_costs: HashMap<AddressHash, StampCost>,
    pending: HashSet<AddressHash>,
}
impl Default for NetworkStamps {
    fn default() -> Self {
        Self {
            policy: NetworkStampPolicy::default(),
            own: HashMap::new(),
            costs: HashMap::new(),
            peer_costs: HashMap::new(),
            pending: HashSet::new(),
        }
    }
}

impl Endpoint {
    /// Choose emission, admission and bounded-generation policy. Existing in-flight
    /// jobs retain their budget; the current policy is checked again before emission.
    pub fn set_network_stamp_policy(&self, policy: NetworkStampPolicy) {
        let mut stamps = self.shared.network_stamps.lock().unwrap();
        if policy.destination_value > stamps.policy.destination_value {
            stamps.own.clear();
        }
        stamps.policy = policy;
        let NetworkStamps { own, costs, .. } = &mut *stamps;
        for (destination, stamp) in own {
            if !costs.contains_key(destination) {
                stamp.link_cost = policy.link_cost;
            }
        }
        while stamps.own.len() > policy.destination_capacity {
            if let Some(key) = stamps.own.keys().min().copied() {
                stamps.own.remove(&key);
            }
        }
        while stamps.costs.len() > policy.destination_capacity {
            if let Some(key) = stamps.costs.keys().min().copied() {
                stamps.costs.remove(&key);
            }
        }
        while stamps.peer_costs.len() > policy.destination_capacity {
            if let Some(key) = stamps.peer_costs.keys().min().copied() {
                stamps.peer_costs.remove(&key);
            }
        }
    }
    pub fn network_stamp_policy(&self) -> NetworkStampPolicy {
        self.shared.network_stamps.lock().unwrap().policy
    }
    /// Set a destination's announced and enforced link request cost. Applies to
    /// its next announce; a caller should announce again after changing it.
    pub fn set_destination_link_cost(
        &self,
        name: &DestinationName,
        cost: StampCost,
    ) -> io::Result<()> {
        let destination = name.destination_hash(self.identity());
        let mut stamps = self.shared.network_stamps.lock().unwrap();
        if stamps.costs.len() >= stamps.policy.destination_capacity
            && !stamps.costs.contains_key(&destination)
        {
            return Err(io::Error::other(
                "network stamp destination capacity reached",
            ));
        }
        stamps.costs.insert(destination, cost);
        if let Some(stamp) = stamps.own.get_mut(&destination) {
            stamp.link_cost = cost;
        }
        Ok(())
    }
    /// The most recently admitted ratchet of a known destination.
    pub fn destination_ratchet(&self, destination: AddressHash) -> Option<[u8; 32]> {
        self.shared
            .address_book
            .lock()
            .unwrap()
            .resolve(destination)
            .and_then(|peer| peer.ratchet)
    }
    /// The most recently admitted, signed link cost of a known destination.
    pub fn destination_link_cost(&self, destination: AddressHash) -> Option<StampCost> {
        self.shared
            .network_stamps
            .lock()
            .unwrap()
            .peer_costs
            .get(&destination)
            .copied()
    }
}

impl Shared {
    pub(super) fn stamp_policy(&self) -> NetworkStampPolicy {
        self.network_stamps.lock().unwrap().policy
    }
    pub(super) fn own_link_cost(&self, destination: AddressHash) -> StampCost {
        let stamps = self.network_stamps.lock().unwrap();
        stamps
            .costs
            .get(&destination)
            .copied()
            .unwrap_or(stamps.policy.link_cost)
    }
    pub(super) fn admit_network_announce(&self, announce: &mut Announce) -> bool {
        let policy = self.stamp_policy();
        announce.apply_stamp_policy(policy.destination_cost, policy.enforce)
    }
    pub(super) fn remember_network_announce(&self, announce: &Announce) {
        let book = self.address_book.lock().unwrap();
        let mut stamps = self.network_stamps.lock().unwrap();
        if stamps.peer_costs.len() >= stamps.policy.destination_capacity
            && !stamps.peer_costs.contains_key(&announce.destination)
        {
            stamps
                .peer_costs
                .retain(|destination, _| book.knows(*destination));
        }
        if let Some(ext) = announce.network_stamp {
            if stamps.peer_costs.len() < stamps.policy.destination_capacity
                || stamps.peer_costs.contains_key(&announce.destination)
            {
                stamps
                    .peer_costs
                    .insert(announce.destination, ext.link_cost);
            }
        } else {
            stamps.peer_costs.remove(&announce.destination);
        }
    }
    pub(super) fn own_network_stamp(&self, destination: AddressHash) -> Option<NetworkStamp> {
        let stamps = self.network_stamps.lock().unwrap();
        stamps
            .policy
            .stamp_destinations
            .then(|| stamps.own.get(&destination).copied())
            .flatten()
    }
}

/// Queue bounded generation; return true when this method owns the announce emission.
/// Duplicate concurrent announces for the destination coalesce until its first stamp is ready.
pub(super) fn announce_stamped(
    endpoint: &Endpoint,
    name: &DestinationName,
    app_data: &[u8],
) -> bool {
    let destination = name.destination_hash(endpoint.identity());
    let shared = &endpoint.shared;
    let policy = shared.stamp_policy();
    if !policy.stamp_destinations {
        return false;
    }
    let material = destination_material(destination, endpoint.identity(), name.name_hash());
    {
        let mut stamps = shared.network_stamps.lock().unwrap();
        if stamps.own.get(&destination).is_some_and(|stamp| {
            crate::stamp::detached_valid(&material, &stamp.stamp, policy.destination_value)
        }) {
            return false;
        }
        if stamps.pending.contains(&destination)
            || stamps.pending.len() >= stamps.policy.generation_capacity
            || stamps.own.len() >= stamps.policy.destination_capacity
        {
            return true;
        }
        stamps.pending.insert(destination);
    }
    let name = name.clone();
    let data = app_data.to_vec();
    let owner = Arc::clone(shared);
    let guard = GenerationGuard {
        owner: Arc::clone(shared),
        key: destination,
    };
    track(shared, async move {
        let worker = Arc::clone(&owner);
        let generated = tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let generated = generate_detached(
                &material,
                policy.destination_value,
                policy.max_attempts,
                fill_random,
                || start.elapsed() >= policy.generation_timeout || !worker.is_running(),
            );
            (generated, guard)
        })
        .await;
        let Ok((generated, _guard)) = generated else {
            return;
        };
        let mut stamps = owner.network_stamps.lock().unwrap();
        let current = stamps.policy;
        if let Some(stamp) = generated
            && owner.is_running()
            && current.stamp_destinations
            && crate::stamp::detached_valid(&material, &stamp, current.destination_value)
        {
            // Dispatch reservations do not reserve cache rows. Other workers may
            // have completed, or the operator may have lowered the cap meanwhile.
            if !stamps.own.contains_key(&destination)
                && stamps.own.len() >= current.destination_capacity
            {
                return;
            }
            let link_cost = stamps
                .costs
                .get(&destination)
                .copied()
                .unwrap_or(current.link_cost);
            stamps
                .own
                .insert(destination, NetworkStamp { stamp, link_cost });
            drop(stamps);
            let seconds = super::registration::host_announce_seconds();
            let ratchet = owner.advertised_ratchet(destination, seconds);
            let packet = owner.build_announce_at(&name, ratchet.as_ref(), &data, seconds);
            owner.broadcast(packet);
        }
    });
    true
}

pub(super) async fn link_seed(
    shared: &Arc<Shared>,
    destination: AddressHash,
) -> io::Result<[u8; 64]> {
    let (policy, cost) = {
        let stamps = shared.network_stamps.lock().unwrap();
        (
            stamps.policy,
            stamps.peer_costs.get(&destination).copied().or_else(|| {
                stamps
                    .policy
                    .always_stamp_links
                    .then_some(stamps.policy.link_cost)
            }),
        )
    };
    let seed = ephemeral_seed();
    let Some(cost) = cost.filter(|cost| cost.to_fp() != 0 && policy.stamp_link_requests) else {
        return Ok(seed);
    };
    let owner = Arc::clone(shared);
    let key = AddressHash::of(&seed);
    {
        let mut stamps = owner.network_stamps.lock().unwrap();
        if stamps.pending.len() >= stamps.policy.generation_capacity {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "network stamp workers at capacity",
            ));
        }
        stamps.pending.insert(key);
    }
    let guard = GenerationGuard {
        owner: Arc::clone(shared),
        key,
    };
    let (seed, _guard) = tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        let seed = generate_link_seed(
            destination,
            seed,
            cost,
            policy.max_attempts,
            fill_random,
            || start.elapsed() >= policy.generation_timeout || !owner.is_running(),
        );
        (seed, guard)
    })
    .await
    .map_err(io::Error::other)?;
    seed.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "link stamp generation cancelled or budget exhausted",
        )
    })
}

pub(super) fn request_stamped_path(shared: &Arc<Shared>, target: AddressHash, tag: &[u8]) -> bool {
    let policy = shared.stamp_policy();
    if !policy.stamp_path_requests {
        return false;
    }
    let Ok(tag) = <[u8; 16]>::try_from(tag) else {
        return true;
    };
    // One work item per target, sharing the same bounded generation slots as announces.
    let key = AddressHash::of(&path_material(target, &tag));
    {
        let mut stamps = shared.network_stamps.lock().unwrap();
        if stamps.pending.len() >= stamps.policy.generation_capacity || !stamps.pending.insert(key)
        {
            return true;
        }
    }
    let owner = Arc::clone(shared);
    let guard = GenerationGuard {
        owner: Arc::clone(shared),
        key,
    };
    track(shared, async move {
        let worker = Arc::clone(&owner);
        let generated = tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let stamp = generate_detached(
                &path_material(target, &tag),
                policy.path_value,
                policy.max_attempts,
                fill_random,
                || start.elapsed() >= policy.generation_timeout || !worker.is_running(),
            );
            (stamp, guard)
        })
        .await;
        let Ok((stamp, _guard)) = generated else {
            return;
        };
        let current = owner.stamp_policy();
        if let Some(stamp) = stamp
            && owner.is_running()
            && current.stamp_path_requests
            && crate::stamp::detached_valid(
                &path_material(target, &tag),
                &stamp,
                current.path_value,
            )
        {
            let packet =
                crate::path::stamped_path_request(target, owner.identity.hash(), &tag, &stamp);
            super::discovery::broadcast_path_packet(&owner, &packet);
        }
    });
    true
}

/// Keep a worker counted through cancellation or panic until the blocking job ends.
struct GenerationGuard {
    owner: Arc<Shared>,
    key: AddressHash,
}
impl Drop for GenerationGuard {
    fn drop(&mut self) {
        self.owner
            .network_stamps
            .lock()
            .unwrap()
            .pending
            .remove(&self.key);
    }
}

#[cfg(test)]
mod tests;
