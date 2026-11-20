//! The path table, path requests, and path responses.

use alloc::vec::Vec;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::announce::AnnounceBlob;
use crate::announce_freshness::{AnnounceFreshness, AnnounceFreshnessCandidate};
use crate::hash::AddressHash;
use crate::node::InterfaceMode;
use crate::packet::Packet;
use crate::transport::{self, Lookup, Removal, RouteEntry, RouteLife, find_live};

use super::announces::duration_ticks;
use super::dedup::SEEN_ANNOUNCES;
use super::entropy::fill_random;
use super::interface::InterfaceId;
use super::registration::host_announce_seconds;
use super::runtime::Endpoint;
use super::shared::Shared;

/// The most destinations a path table will hold, so strangers' announces cannot grow memory
/// without bound. Expired routes go first; then never-used routes, the soonest to expire
/// first; then the route used longest ago (D30). Far above what a LoRa mesh sees.
#[cfg(not(test))]
pub(super) const PATH_TABLE_CAPACITY: usize = DESKTOP_PATH_TABLE_CAPACITY;
#[cfg(test)]
pub(super) const PATH_TABLE_CAPACITY: usize = 4;

/// [`PATH_TABLE_CAPACITY`] outside the endpoint's tests, so the shared tests can run at it.
pub(crate) const DESKTOP_PATH_TABLE_CAPACITY: usize = 4096;

/// The least time between path requests we broadcast for one destination, so a peer sending
/// traffic we cannot verify cannot make us broadcast once per packet.
// Integration tests in dependent crates (outrider's, for one) build without cfg(test) and get
// the real 20-second floor: a second request for one destination there waits it out.
#[cfg(not(test))]
pub(super) const PATH_REQUEST_MIN_INTERVAL: Duration = Duration::from_secs(20);
#[cfg(test)]
pub(super) const PATH_REQUEST_MIN_INTERVAL: Duration = Duration::from_millis(60);

/// The most path requests broadcast in any [`PATH_REQUEST_MIN_INTERVAL`] window, across ALL
/// destinations: the peer provoking a request also names its destination, so fabricated
/// unique ones would never meet the per-destination floor. A refused request records nothing,
/// so this also bounds the budget table.
pub(super) const PATH_REQUEST_GLOBAL_MAX: usize = 8;

/// How long a path request we sent exempts its destination's announces from ingress holds
/// (RNS `PATH_REQUEST_GATE_TIMEOUT`, `Transport.py` 135, 980-986, 1815-1821).
pub(super) const PATH_REQUEST_GATE: Duration = Duration::from_secs(45);

/// A learned route to a destination, on the desktop's clock. Its transport node is per
/// destination, because one interface reaches different destinations through different nodes.
pub(super) type PathEntry = RouteEntry<Instant>;

/// How long a never-used route lingers on a transport node: RNS's `UNUSED_PATH_LINGER`,
/// 1.75 hours (`Transport.py` 228).
const UNUSED_PATH_LINGER: Duration = Duration::from_secs(105 * 60);
/// How long one lingers on a leaf, which does not forward or is a shared instance's client:
/// `UNUSED_PATH_LINGER_LEAF`, 12 minutes (`Transport.py` 229, 451-454).
const UNUSED_PATH_LINGER_LEAF: Duration = Duration::from_secs(12 * 60);
/// An interface with a bitrate below this, or a first-hop airtime longer than one MTU takes
/// at it, is slow: `PATH_SLOW_BITRATE` (`Transport.py` 232).
const PATH_SLOW_BITRATE: u64 = 25_000;
/// How many times longer a never-used route lingers on a slow interface:
/// `PATH_SLOW_LINGER_FACTOR` (`Transport.py` 233).
const PATH_SLOW_LINGER_FACTOR: u64 = 10;

/// Route life on the desktop: lifetimes by mode under the policy's full-mode route TTL, and
/// stock's linger for a route never used (D29).
#[derive(Clone, Copy)]
pub(super) struct PathLife<'a> {
    shared: &'a Shared,
    full: u64,
    linger: u64,
}

impl RouteLife for PathLife<'_> {
    fn lifetime(&self, mode: InterfaceMode) -> u64 {
        mode.route_ttl(self.full)
    }

    fn linger(&self, iface: InterfaceId) -> Option<u64> {
        // Slow by the configured bitrate, else by the carrier's airtime for one MTU, as the
        // announce cap reads it (`Transport.py` 1129-1130). Neither known is not slow.
        let slow = self.shared.first_hop_airtime(iface)
            > crate::node::first_hop_airtime(PATH_SLOW_BITRATE);
        Some(if slow {
            self.linger.saturating_mul(PATH_SLOW_LINGER_FACTOR)
        } else {
            self.linger
        })
    }
}

/// Make room for one more route in a full path table at `now`, as a learned route does
/// ([`transport::make_route_room`]). Returns whether a live route was evicted.
pub(super) fn make_room_for_path(
    paths: &mut HashMap<AddressHash, PathEntry>,
    now: Instant,
    life: PathLife<'_>,
) -> bool {
    let mut evicted = false;
    transport::make_route_room(paths, now, PATH_TABLE_CAPACITY, life, |_, _, removal| {
        evicted |= removal == Removal::Evicted;
    });
    evicted
}

impl PathEntry {
    pub(super) fn live_at(&self, now: Instant, life: PathLife<'_>) -> bool {
        self.live(now, &life)
    }
}

impl Shared {
    /// Mark `dest`'s live route used now; its life does not move.
    pub(super) fn mark_path_used(&self, dest: AddressHash) {
        let now = Instant::now();
        let life = self.path_life();
        self.write_diagnostic(|| {
            let mut paths = self.path_table.lock().unwrap();
            let marked = transport::mark_route_used(&mut *paths, &dest, now, life);
            ((), marked)
        });
    }

    /// Prove `dest`'s live route alive now: its life starts over in its interface's current
    /// mode, and it is marked used.
    pub(super) fn prove_path_alive(&self, dest: AddressHash) {
        let now = Instant::now();
        let life = self.path_life();
        self.write_diagnostic(|| {
            let mut paths = self.path_table.lock().unwrap();
            let proved = transport::prove_route_alive(
                &mut *paths,
                &dest,
                now,
                |iface| self.interface_mode(iface),
                life,
            );
            ((), proved)
        });
    }

    /// How routes live now: a transport node lingers longer than a leaf.
    pub(super) fn path_life(&self) -> PathLife<'_> {
        let transport = self.routing.lock().unwrap().forward_packets && !self.local.attached();
        let linger = if transport {
            UNUSED_PATH_LINGER
        } else {
            UNUSED_PATH_LINGER_LEAF
        };
        PathLife {
            shared: self,
            full: duration_ticks(self.route_ttl()),
            linger: duration_ticks(linger),
        }
    }

    pub(super) fn route_ttl(&self) -> Duration {
        Duration::from_millis(self.route_ttl_ms.load(Ordering::Relaxed))
    }

    /// A path response for `target` if it is one of our registered destinations: an announce
    /// with context [`crate::path::CTX_PATH_RESPONSE`]. A transport answers for others from
    /// its announce cache instead (see [`super::discovery`]).
    pub(super) fn path_response(&self, target: AddressHash) -> Option<Packet> {
        self.path_response_at(target, host_announce_seconds())
    }

    /// The deterministic half of [`Self::path_response`], with the clock supplied.
    pub(super) fn path_response_at(
        &self,
        target: AddressHash,
        source_seconds: u64,
    ) -> Option<Packet> {
        if self.stamp_policy().stamp_destinations && self.own_network_stamp(target).is_none() {
            return None;
        }
        let (name, app_data, source) = {
            let reg = self.registered.lock().unwrap();
            let r = reg.iter().find(|r| r.dest == target)?;
            (
                r.name.clone(),
                r.app_data.clone(),
                r.app_data_source.clone(),
            )
        };
        let app_data = source.map_or(app_data, |source| source(source_seconds));
        // A path response is an announce, so it rotates a due ratchet too (`Destination.py`
        // 285-288 runs for both).
        let ratchet = self.advertised_ratchet(target, source_seconds);
        let mut pkt = self.build_announce_at(&name, ratchet.as_ref(), &app_data, source_seconds);
        pkt.context = crate::path::CTX_PATH_RESPONSE;
        Some(pkt)
    }

    /// Whether `dest` has an unexpired route, without evicting anything.
    pub(super) fn has_live_route(&self, dest: AddressHash) -> bool {
        self.live_route(dest).is_some()
    }

    pub(super) fn live_route(&self, dest: AddressHash) -> Option<PathEntry> {
        let life = self.path_life();
        let paths = self.path_table.lock().unwrap();
        find_live(&*paths, &dest, |entry| entry.live_at(Instant::now(), life)).copied()
    }

    /// Whether a copy that `table` refused should still move the route to `iface`: the same
    /// emission as the route's, with no more hops, heard on an interface of higher gravity
    /// (`Transport.py` 2229-2251). Only the route moves; the emission was already published
    /// and relayed when it first arrived.
    pub(super) fn gravity_repoints(
        &self,
        table: &AnnounceFreshness,
        candidate: AnnounceFreshnessCandidate,
        iface: InterfaceId,
        hops: u8,
    ) -> bool {
        let Some(route) = self.live_route(candidate.destination) else {
            return false;
        };
        if hops > route.hops
            || self.iface_policy(iface).gravity <= self.iface_policy(route.iface).gravity
        {
            return false;
        }
        // The refusal means the route's timebase is not older than this one; accepting the
        // next second means it is not newer either, so it is the same emission.
        AnnounceBlob::mint(candidate.blob.nonce(), candidate.timebase() + 1).is_ok_and(|blob| {
            let next = AnnounceFreshnessCandidate { blob, ..candidate };
            table.evaluate(next, true).is_accepted()
        })
    }

    /// Drop `dest`'s route, if any, and the announce cached with it.
    pub(super) fn forget_path(&self, dest: AddressHash) {
        self.write_diagnostic(|| ((), self.path_table.lock().unwrap().remove(&dest).is_some()));
        self.discovery.forget(dest);
    }

    /// Record that `dest` is reachable via `iface` at `hops`.
    ///
    /// Freshness has already admitted this announce, so its route becomes the incumbent
    /// whatever its hop count: preferring the shortest here would let an older announce
    /// override the freshness ledger's decision.
    pub(super) fn learn_path(
        &self,
        dest: AddressHash,
        iface: InterfaceId,
        hops: u8,
        transport: Option<AddressHash>,
    ) {
        self.learn_path_at(dest, iface, hops, transport, Instant::now());
    }

    /// As [`Self::learn_path`], at a supplied instant, for tests independent of timing.
    pub(super) fn learn_path_at(
        &self,
        dest: AddressHash,
        iface: InterfaceId,
        hops: u8,
        transport: Option<AddressHash>,
        now: Instant,
    ) {
        let mode = self.interface_mode(iface);
        let mut evicted = Vec::new();
        let learned = self.write_diagnostic(|| {
            let mut t = self.path_table.lock().unwrap();
            let life = self.path_life();
            let next = PathEntry {
                iface,
                transport,
                hops,
                learned: now,
                mode,
                used: None,
            };
            let replaced = transport::record_route(
                &mut *t,
                dest,
                next,
                PATH_TABLE_CAPACITY,
                life,
                |gone, _, removal| {
                    evicted.push(gone);
                    if removal == Removal::Evicted {
                        self.routing_stats
                            .paths_evicted
                            .fetch_add(1, Ordering::Relaxed);
                    }
                },
            );
            (next, replaced != Some(next))
        });
        evicted
            .into_iter()
            .for_each(|gone| self.discovery.forget(gone));
        self.tunnel_path_learned(dest, &learned);
    }

    /// Destinations the address book must keep while it is full: those with an unexpired path
    /// or a live link. Each table is locked in turn, never two at once.
    pub(super) fn destinations_in_use(&self) -> HashSet<AddressHash> {
        let life = self.path_life();
        let now = Instant::now();
        let mut in_use: HashSet<AddressHash> = self
            .path_table
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, entry)| entry.live_at(now, life))
            .map(|(destination, _)| *destination)
            .collect();
        in_use.extend(
            self.links
                .lock()
                .unwrap()
                .values()
                .filter_map(|entry| entry.remote.destination),
        );
        in_use
    }

    /// The interface to reach `dest`, if a route is known and unexpired. Evicts an expired
    /// route as a side effect, so a stale path never lingers past a lookup.
    pub(super) fn path_iface(&self, dest: AddressHash) -> Option<InterfaceId> {
        self.path_entry(dest).map(|entry| entry.iface)
    }

    /// [`Self::path_iface`], with the whole route, read once.
    pub(super) fn path_entry(&self, dest: AddressHash) -> Option<PathEntry> {
        let life = self.path_life();
        let (entry, expired) = self.write_diagnostic(|| {
            let mut t = self.path_table.lock().unwrap();
            match transport::live_route(&mut *t, &dest, Instant::now(), life) {
                Lookup::Live(e) => ((Some(e), false), false),
                Lookup::Expired(_) => ((None, true), true),
                Lookup::Missing => ((None, false), false),
            }
        });
        if expired {
            self.discovery.forget(dest);
        }
        entry
    }

    /// Whether a path request for `dest` may go out now, under the per-destination floor and
    /// the global cap.
    ///
    /// Both checks pass before either records anything, so a request refused by one never
    /// spends the other's budget.
    fn path_request_within_budget(&self, dest: AddressHash) -> bool {
        let mut budget = self.path_request_budget.lock().unwrap();
        let mut stamps = self.path_request_stamps.lock().unwrap();
        let now = Instant::now();
        if let Some(&last) = budget.get(&dest)
            && now.duration_since(last) < PATH_REQUEST_MIN_INTERVAL
        {
            return false;
        }
        while let Some(&oldest) = stamps.front()
            && now.duration_since(oldest) >= PATH_REQUEST_MIN_INTERVAL
        {
            stamps.pop_front();
        }
        if stamps.len() >= PATH_REQUEST_GLOBAL_MAX {
            return false;
        }
        if budget.len() > SEEN_ANNOUNCES {
            let keep = PATH_REQUEST_MIN_INTERVAL.max(PATH_REQUEST_GATE);
            budget.retain(|_, t| now.duration_since(*t) < keep);
        }
        budget.insert(dest, now);
        stamps.push_back(now);
        true
    }

    /// Whether we sent a path request for `dest` within `window`.
    pub(super) fn path_requested_within(&self, dest: AddressHash, window: Duration) -> bool {
        let budget = self.path_request_budget.lock().unwrap();
        budget.get(&dest).is_some_and(|at| at.elapsed() < window)
    }
}

impl Endpoint {
    /// Broadcast a path request for `dest`. The path response is an announce, which fills the
    /// path table *and the address book*: use it when a route has gone stale, or to learn the
    /// identity of a source whose keys we lack. A transport sends the three-field form naming
    /// itself, as RNS does.
    ///
    /// Rate-limited per destination and overall, silently. Returns whether a request went out.
    pub fn request_path(&self, dest: AddressHash) -> bool {
        if !self.shared.path_request_within_budget(dest) {
            return false;
        }
        let mut tag = [0u8; crate::path::TAG_LEN];
        fill_random(&mut tag);
        super::discovery::broadcast_path_request(&self.shared, dest, &tag);
        true
    }
}
