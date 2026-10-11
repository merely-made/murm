//! Path requests (`Transport.py` 1838-1884, 3279-3325, 3418-3600): answered for our own
//! destinations; as a transport, answered from the announce behind a live route, or searched
//! for on the requester's behalf, the matching announce passed back the moment it arrives
//! (2430-2455). Per-interface rate control is in [`rates`].
//!
//! A transport is a node with [`RoutingPolicy::forward_announces`] set, the relay role: what
//! RNS calls `transport_enabled()`.
//!
//! [`RoutingPolicy::forward_announces`]: super::RoutingPolicy::forward_announces

mod rates;

use alloc::vec::Vec;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::hash::AddressHash;
use crate::iface_mode::{InterfaceMode, discovers_paths};
use crate::packet::{HeaderType, Packet};
use crate::path::{PathRequest, path_request_via};

use super::iface_policy::PathRequestPolicy;
use super::interface::{InterfaceId, QueueAdmission};
use super::paths::{PATH_REQUEST_GATE, PATH_TABLE_CAPACITY, PathEntry};
use super::queue::TrafficClass;
use super::shared::Shared;
use rates::PrRates;

/// Wait before answering from the cache, so a closer node may answer first
/// (`PATH_REQUEST_GRACE`).
const ANSWER_GRACE: Duration = Duration::from_millis(400);
/// More wait when answering on a roaming interface (`PATH_REQUEST_RG`).
const ROAMING_GRACE: Duration = Duration::from_millis(1_500);
/// The least time a search waits for its announce (`PATH_REQUEST_TIMEOUT`).
const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Added to the slowest interface's MTU round trip (`DEFAULT_PER_HOP_TIMEOUT`).
const PER_HOP_TIMEOUT: Duration = Duration::from_secs(6);
/// Searches, and in-flight requests, held at once. RNS's tables are unbounded; past this a new
/// search is not started.
const WAITING_CAPACITY: usize = 1_024;

/// Requesters waiting for one destination's announce (`discovery_path_requests`).
#[derive(Debug)]
struct Waiting {
    requesters: Vec<InterfaceId>,
    expires: Instant,
    /// A search went out; an entry only batched from in-flight requests has not.
    engaged: bool,
}

#[derive(Debug, Default)]
struct State {
    /// Requests being handled, by target, so a second copy joins the first (1862-1884).
    inflight: HashMap<AddressHash, Instant>,
    waiting: HashMap<AddressHash, Waiting>,
    rates: HashMap<InterfaceId, PrRates>,
}

/// Path-discovery state: searches, request rates and the announce cache.
#[derive(Debug, Default)]
pub(super) struct Discovery {
    state: Mutex<State>,
    /// The verified announce behind each route, which a transport answers with (RNS caches it
    /// by packet hash, 2456). Bounded with the path table and dropped with its route.
    pub(super) announces: Mutex<HashMap<AddressHash, Packet>>,
}

impl State {
    fn expire(&mut self, now: Instant) {
        self.inflight
            .retain(|_, at| now.saturating_duration_since(*at) < PATH_REQUEST_GATE);
        self.waiting.retain(|_, waiting| now < waiting.expires);
    }

    fn rates(&mut self, iface: InterfaceId, now: Instant) -> &mut PrRates {
        self.rates.entry(iface).or_insert_with(|| PrRates::new(now))
    }

    /// Add `iface` to the requesters for `target`, opening an entry if there is room.
    fn wait_on(&mut self, target: AddressHash, iface: InterfaceId, expires: Instant) -> bool {
        let full = self.waiting.len() >= WAITING_CAPACITY;
        let Some(waiting) = (match self.waiting.get_mut(&target) {
            Some(waiting) => Some(waiting),
            None if full => None,
            None => Some(self.waiting.entry(target).or_insert(Waiting {
                requesters: Vec::new(),
                expires,
                engaged: false,
            })),
        }) else {
            return false;
        };
        if !waiting.requesters.contains(&iface) {
            waiting.requesters.push(iface);
        }
        true
    }

    /// Whether a request for `target` joins one in flight. A limited one is dropped rather than
    /// joined; otherwise the first is recorded as in flight (1862-1884).
    fn batch(
        &mut self,
        target: AddressHash,
        iface: InterfaceId,
        limited: bool,
        expires: Instant,
        now: Instant,
    ) -> bool {
        if self.inflight.contains_key(&target) {
            if !limited {
                self.wait_on(target, iface, expires);
            }
            return true;
        }
        if self.inflight.len() < WAITING_CAPACITY {
            self.inflight.insert(target, now);
        }
        false
    }
}

/// An attached interface, as a search sees it.
struct Seen {
    id: InterfaceId,
    mode: InterfaceMode,
    online: bool,
}

fn interfaces(shared: &Shared) -> Vec<Seen> {
    let interfaces = shared.interfaces.lock().unwrap();
    interfaces
        .iter()
        .map(|i| Seen {
            id: i.id,
            mode: i.mode,
            online: i.online.load(Ordering::Acquire),
        })
        .collect()
}

/// How long a search waits: an MTU round trip on the slowest online interface plus a hop's
/// allowance, and never under [`SEARCH_TIMEOUT`] (`Transport.py` 3205-3208, 3557).
fn search_timeout(shared: &Shared, ifaces: &[Seen]) -> Duration {
    let slowest = ifaces
        .iter()
        .filter(|i| i.online)
        .map(|i| shared.first_hop_airtime(i.id))
        .max()
        .unwrap_or(0);
    let round_trip = match slowest {
        0 => Duration::ZERO,
        ms => Duration::from_millis(2 * ms) + PER_HOP_TIMEOUT,
    };
    SEARCH_TIMEOUT.max(round_trip)
}

/// `announce` as this transport passes it to one requester: a path response at `hops`,
/// addressed through us (`Transport.py` 2438-2453, 3486-3519).
fn path_response_copy(shared: &Shared, mut announce: Packet, hops: u8) -> Packet {
    announce.header_type = HeaderType::Type2;
    announce.transport = Some(shared.identity.public().hash());
    announce.hops = hops;
    announce.context = crate::path::CTX_PATH_RESPONSE;
    announce
}

/// Answer or search for a path request heard on `iface`. A tagless or repeated target and tag
/// is dropped (1838-1856); every other is counted toward the interface's burst (1858), whose
/// state the inbound queue reads to lane its next requests. `ingress_limited` (that lane)
/// or a burst forbids a search (3542-3547).
pub(super) fn handle_path_request(
    shared: &Arc<Shared>,
    iface: InterfaceId,
    request: PathRequest,
    ingress_limited: bool,
) {
    let fresh = request
        .unique_tag()
        .is_some_and(|tag| shared.path_request_tags.lock().unwrap().insert(tag));
    if !fresh {
        shared
            .routing_stats
            .filtered_packets
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    let now = Instant::now();
    let target = request.target;
    let transport = shared.routing.lock().unwrap().forward_announces;
    let policy = shared.iface_policy(iface);
    let ingress = policy
        .ingress
        .unwrap_or_else(|| shared.announce_admission.lock().unwrap().policy());
    let ifaces = interfaces(shared);
    let expires = now + search_timeout(shared, &ifaces);
    let (burst, batched) = {
        let mut state = shared.discovery.state.lock().unwrap();
        state.expire(now);
        if state.rates.len() > ifaces.len() {
            state
                .rates
                .retain(|id, _| ifaces.iter().any(|i| i.id == *id));
        }
        let rates = state.rates(iface, now);
        rates.received(now);
        let burst = rates.ingress_limited(now, &policy.path_requests, &ingress);
        // Batching is a transport's: an ordinary node answers only for itself.
        let limited = ingress_limited || burst;
        (
            burst,
            transport && state.batch(target, iface, limited, expires, now),
        )
    };
    shared.with_iface(iface, |i| {
        i.stats.path_request_burst.store(burst, Ordering::Relaxed)
    });
    let limited = ingress_limited || burst;
    if batched {
        return;
    }
    let answered = if let Some(response) = shared.path_response(target) {
        shared.send_on_class(iface, response, TrafficClass::Control);
        true
    } else if transport && let Some((route, announce)) = cached_route(shared, target) {
        answer_from_cache(shared, iface, &request, route, announce);
        true
    } else {
        if transport {
            let search = Search {
                iface,
                request: &request,
                limited,
                expires,
                now,
            };
            search.run(shared, &ifaces, &policy.path_requests);
        }
        false
    };
    if answered {
        shared
            .discovery
            .state
            .lock()
            .unwrap()
            .inflight
            .remove(&target);
    }
}

/// The live route to `target` and the announce that taught it.
fn cached_route(shared: &Shared, target: AddressHash) -> Option<(PathEntry, Packet)> {
    let route = shared.live_route(target)?;
    let announce = shared
        .discovery
        .announces
        .lock()
        .unwrap()
        .get(&target)
        .cloned()?;
    Some((route, announce))
}

/// Queue a path response from the cache, unless the route leads back where the request came
/// from: learned on this same roaming interface (3468-3471), or through the requester
/// (3476-3484). It waits a grace for closer nodes, longer on a roaming interface (3496-3506).
fn answer_from_cache(
    shared: &Shared,
    iface: InterfaceId,
    request: &PathRequest,
    route: PathEntry,
    announce: Packet,
) {
    let roaming = shared.interface_mode(iface) == InterfaceMode::Roaming;
    if (roaming && route.iface == iface)
        || (request.requestor.is_some() && request.requestor == route.transport)
    {
        return;
    }
    let delay = ANSWER_GRACE
        + if roaming {
            ROAMING_GRACE
        } else {
            Duration::ZERO
        };
    // RNS's hop count includes the hop that brought the announce to us.
    let response = path_response_copy(shared, announce, route.hops.saturating_add(1));
    if shared.schedule_path_response(iface, response, delay) {
        shared
            .routing_stats
            .path_requests_answered
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// A recursive search for an unknown destination (3528-3583).
struct Search<'a> {
    iface: InterfaceId,
    request: &'a PathRequest,
    limited: bool,
    expires: Instant,
    now: Instant,
}

impl Search<'_> {
    /// Search on every other online interface the requesting one's mode allows (all of them
    /// for `recursive`; BOUNDARY reaches only BOUNDARY and GATEWAY, 3426-3433), once per
    /// destination at a time, with the requester's tag and our transport id.
    fn run(&self, shared: &Shared, ifaces: &[Seen], rates: &PathRequestPolicy) {
        let mode = shared.interface_mode(self.iface);
        let Some(modes) = (if rates.recursive {
            Some(&InterfaceMode::ALL[..])
        } else {
            discovers_paths(mode)
        }) else {
            return;
        };
        let target = self.request.target;
        {
            let mut state = shared.discovery.state.lock().unwrap();
            if state.waiting.get(&target).is_some_and(|w| w.engaged) {
                return;
            }
            if self.limited {
                shared
                    .routing_stats
                    .path_requests_ingress_limited
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
            if !state.wait_on(target, self.iface, self.expires) {
                return;
            }
            let waiting = state.waiting.get_mut(&target).expect("just added");
            waiting.engaged = true;
            waiting.expires = self.expires;
        }
        let ours = shared.identity.public().hash();
        let tag = self.request.tag().unwrap_or_default();
        let packet = path_request_via(target, Some(ours), tag);
        for seen in ifaces {
            if seen.id != self.iface && seen.online && modes.contains(&seen.mode) {
                send_path_request(shared, seen.id, &packet, true, self.now);
            }
        }
        shared
            .routing_stats
            .path_requests_searched
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// Send a path request on one interface, counting it toward its egress rate. A recursive one
/// is withheld while egress control is active (`Interface.py` 241-250) or the interface's
/// announce budget is spent (`Transport.py` 3286-3318).
fn send_path_request(
    shared: &Shared,
    iface: InterfaceId,
    packet: &Packet,
    recursive: bool,
    now: Instant,
) -> bool {
    let rates = shared.iface_policy(iface).path_requests;
    if recursive {
        let limited = shared
            .discovery
            .state
            .lock()
            .unwrap()
            .rates(iface, now)
            .egress_limited(now, &rates);
        if limited {
            shared
                .routing_stats
                .path_requests_egress_limited
                .fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if !shared.admit_recursive_path_request(iface, packet.encoded_len()) {
            return false;
        }
    }
    let queued = matches!(
        shared.push_to(iface, packet.clone(), TrafficClass::Control),
        QueueAdmission::Queued
    );
    if queued {
        shared
            .discovery
            .state
            .lock()
            .unwrap()
            .rates(iface, now)
            .sent(now);
    }
    queued
}

/// Ask every interface for `target`'s path; a transport names itself (`Transport.py`
/// 3279-3325).
pub(super) fn broadcast_path_request(shared: &Shared, target: AddressHash, tag: &[u8]) {
    let transport = shared.routing.lock().unwrap().forward_announces;
    let requestor = transport.then(|| shared.identity.public().hash());
    let packet = path_request_via(target, requestor, tag);
    let now = Instant::now();
    for seen in interfaces(shared) {
        send_path_request(shared, seen.id, &packet, false, now);
    }
}

impl Discovery {
    /// Whether a search waits for `destination`, which exempts its announce from ingress holds
    /// (`Transport.py` 1815-1821).
    pub(super) fn awaits(&self, destination: AddressHash) -> bool {
        self.state
            .lock()
            .unwrap()
            .waiting
            .contains_key(&destination)
    }

    /// Drop the cached announce of a route that is gone.
    pub(super) fn forget(&self, destination: AddressHash) {
        self.announces.lock().unwrap().remove(&destination);
    }
}

/// A route to `announce`'s destination was learned from it: cache it, and pass it at once to
/// every interface waiting for it, as a path response (`Transport.py` 2430-2455, 2479-2481).
pub(super) fn announce_learned(shared: &Shared, announce: &Packet) {
    let destination = announce.destination;
    let over = {
        let mut cache = shared.discovery.announces.lock().unwrap();
        cache.insert(destination, announce.clone());
        cache.len() > PATH_TABLE_CAPACITY
    };
    if over {
        let routed: HashSet<AddressHash> =
            shared.path_table.lock().unwrap().keys().copied().collect();
        let mut cache = shared.discovery.announces.lock().unwrap();
        cache.retain(|destination, _| routed.contains(destination));
    }
    let waiting = {
        let mut state = shared.discovery.state.lock().unwrap();
        state.inflight.remove(&destination);
        state.waiting.remove(&destination)
    };
    for iface in waiting.map(|w| w.requesters).unwrap_or_default() {
        let hops = announce.hops.saturating_add(1);
        let response = path_response_copy(shared, announce.clone(), hops);
        shared.push_to(iface, response, TrafficClass::Control);
    }
}
