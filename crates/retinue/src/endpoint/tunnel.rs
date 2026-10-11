//! Tunnel synthesis: a reconnecting client's paths are kept for it by the node it rejoins
//! (`TCPInterface.py` 169-180, 298-299; `Transport.py` 2463-2473, 2764-2860).
//!
//! A client signs `public key || interface hash || random` with its transport identity; the
//! receiving node keys a tunnel by `full_hash(public key || interface hash)`, mirrors every
//! route learned over that interface into it, and restores them onto whichever interface the
//! same tunnel next appears on.

use alloc::vec::Vec;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::destination::DestinationName;
use crate::hash::{AddressHash, HASH_LEN, full_hash};
use crate::identity::{IDENTITY_LEN, Identity, PrivateIdentity, SIGNATURE_LEN};
use crate::packet::{DestinationType, HeaderType, Packet, PacketType, Propagation};

use super::entropy::fill_random;
use super::interface::InterfaceId;
use super::paths::{PATH_TABLE_CAPACITY, PathEntry};
use super::shared::Shared;

/// A tunnel, and each path mirrored into it, lives this long unrefreshed (`TUNNEL_TIMEOUT`,
/// `TUNNEL_PATH_TIMEOUT`, `Transport.py` 157-158).
pub(super) const TUNNEL_TIMEOUT: Duration = Duration::from_secs(8 * 60 * 60);

/// Tunnels kept at once. RNS keeps every one; a signed synthesis costs only a fresh identity,
/// so this is bounded and the soonest to expire makes room.
const TUNNEL_CAPACITY: usize = 256;

/// Paths kept across all tunnels; the oldest mirrored makes room.
const TUNNEL_PATH_CAPACITY: usize = PATH_TABLE_CAPACITY;

/// Random bytes in a synthesis (`RNS.Reticulum.TRUNCATED_HASHLENGTH`).
const RANDOM_LEN: usize = 16;

/// A synthesis payload: key, interface hash, random, signature over the first three: 176
/// bytes (`Transport.py` 2788-2800).
pub(super) const SYNTHESIS_LEN: usize = IDENTITY_LEN + HASH_LEN + RANDOM_LEN + SIGNATURE_LEN;

type TunnelId = [u8; HASH_LEN];

#[derive(Debug)]
struct Tunnel {
    /// The interface the tunnel is on now; `None` while its client is away.
    iface: Option<InterfaceId>,
    paths: HashMap<AddressHash, Mirrored>,
    expires: Instant,
}

#[derive(Clone, Copy, Debug)]
struct Mirrored {
    entry: PathEntry,
    at: Instant,
}

/// Tunnels this node keeps for its clients, by tunnel id.
#[derive(Debug)]
pub(super) struct Tunnels {
    /// `PLAIN rnstransport.tunnel.synthesize`.
    destination: AddressHash,
    table: Mutex<HashMap<TunnelId, Tunnel>>,
}

impl Default for Tunnels {
    fn default() -> Self {
        Self {
            destination: synthesis_destination(),
            table: Mutex::default(),
        }
    }
}

/// The plain destination a synthesis is addressed to (`Transport.py` 362).
pub(super) fn synthesis_destination() -> AddressHash {
    DestinationName::new("rnstransport", ["tunnel", "synthesize"]).plain_hash()
}

/// A synthesis for the interface named `label`, signed by `identity` (`Transport.py`
/// 2764-2784). The tunnel id is stable for as long as the identity and label are.
pub(super) fn synthesis(identity: &PrivateIdentity, label: &str) -> Packet {
    let mut payload = Vec::with_capacity(SYNTHESIS_LEN);
    payload.extend_from_slice(&identity.public().to_public_bytes());
    payload.extend_from_slice(&full_hash(label.as_bytes()));
    let mut random = [0u8; RANDOM_LEN];
    fill_random(&mut random);
    payload.extend_from_slice(&random);
    let signature = identity.sign(&payload);
    payload.extend_from_slice(&signature);
    Packet {
        ifac: false,
        header_type: HeaderType::Type1,
        context_flag: false,
        propagation: Propagation::Broadcast,
        destination_type: DestinationType::Plain,
        packet_type: PacketType::Data,
        hops: 0,
        transport: None,
        destination: synthesis_destination(),
        context: 0,
        payload,
    }
}

/// The tunnel id a valid synthesis payload names, or `None` if its length or signature is
/// wrong (`Transport.py` 2788-2805).
fn verify(payload: &[u8]) -> Option<TunnelId> {
    if payload.len() != SYNTHESIS_LEN {
        return None;
    }
    let (signed, signature) = payload.split_at(SYNTHESIS_LEN - SIGNATURE_LEN);
    let key: &[u8; IDENTITY_LEN] = signed[..IDENTITY_LEN].try_into().ok()?;
    let signature: &[u8; SIGNATURE_LEN] = signature.try_into().ok()?;
    Identity::from_public_bytes(key)
        .ok()?
        .verify(signed, signature)
        .then(|| full_hash(&signed[..IDENTITY_LEN + HASH_LEN]))
}

impl Tunnels {
    fn cull(table: &mut HashMap<TunnelId, Tunnel>, now: Instant) {
        table.retain(|_, tunnel| {
            tunnel
                .paths
                .retain(|_, path| now.saturating_duration_since(path.at) < TUNNEL_TIMEOUT);
            now < tunnel.expires
        });
    }

    /// Drop the oldest mirrored path across all tunnels once they hold the cap.
    fn make_path_room(table: &mut HashMap<TunnelId, Tunnel>) {
        let held: usize = table.values().map(|t| t.paths.len()).sum();
        if held < TUNNEL_PATH_CAPACITY {
            return;
        }
        let oldest = table
            .iter()
            .flat_map(|(id, t)| t.paths.iter().map(move |(dest, p)| (p.at, *id, *dest)))
            .min_by_key(|(at, ..)| *at);
        if let Some((_, id, dest)) = oldest
            && let Some(tunnel) = table.get_mut(&id)
        {
            tunnel.paths.remove(&dest);
        }
    }
}

impl Shared {
    /// Handle `pkt` if it is a tunnel synthesis; true if it was one and is consumed. One that
    /// fails validation counts as a protocol violation on `iface`.
    pub(super) fn tunnel_intercept(&self, iface: InterfaceId, pkt: &Packet) -> bool {
        if pkt.packet_type != PacketType::Data || pkt.destination != self.tunnels.destination {
            return false;
        }
        match verify(&pkt.payload) {
            Some(id) => self.handle_tunnel(id, iface, Instant::now()),
            None => {
                self.with_iface(iface, |i| i.stats.note_protocol_violation());
            }
        }
        true
    }

    /// Create tunnel `id` on `iface`, or re-point it there and restore its paths
    /// (`Transport.py` 2820-2860). A path is restored if the current route has no fewer hops
    /// or has expired, or if there is no current route and the mirrored one has not expired.
    /// The timebase test (2846-2850) needs no timebases here: a route learned for the same
    /// destination elsewhere removes the mirrored one, since freshness only admits a newer
    /// emission. Lock order: tunnels, then the path table.
    fn handle_tunnel(&self, id: TunnelId, iface: InterfaceId, now: Instant) {
        let mut table = self.tunnels.table.lock().unwrap();
        Tunnels::cull(&mut table, now);
        for tunnel in table.values_mut() {
            if tunnel.iface == Some(iface) {
                tunnel.iface = None;
            }
        }
        if !table.contains_key(&id) && table.len() >= TUNNEL_CAPACITY {
            let soonest = table.iter().min_by_key(|(_, t)| t.expires).map(|(k, _)| *k);
            if let Some(soonest) = soonest {
                table.remove(&soonest);
            }
        }
        let tunnel = table.entry(id).or_insert_with(|| Tunnel {
            iface: None,
            paths: HashMap::new(),
            expires: now,
        });
        tunnel.iface = Some(iface);
        tunnel.expires = now + TUNNEL_TIMEOUT;
        if tunnel.paths.is_empty() {
            return;
        }
        let route_ttl = self.route_ttl();
        let mode = self.interface_mode(iface);
        let restored = self.write_diagnostic(|| {
            let mut paths = self.path_table.lock().unwrap();
            let mut restored = 0u64;
            tunnel.paths.retain(|dest, mirrored| {
                let keep = match paths.get(dest) {
                    Some(current) => {
                        mirrored.entry.hops <= current.hops || !current.live_at(now, route_ttl)
                    }
                    None => mirrored.entry.live_at(now, route_ttl),
                };
                if keep {
                    paths.insert(
                        *dest,
                        PathEntry {
                            iface,
                            mode,
                            ..mirrored.entry
                        },
                    );
                    restored += 1;
                }
                keep
            });
            (restored, restored > 0)
        });
        self.routing_stats
            .tunnel_paths_restored
            .fetch_add(restored, Ordering::Relaxed);
    }

    /// `dest`'s route was just learned or replaced as `entry`. Called outside the path-table
    /// lock. A route on a tunnel's interface is mirrored into it and refreshes it
    /// (`Transport.py` 2463-2473); one elsewhere supersedes any mirrored copy (1040-1056).
    pub(super) fn tunnel_path_learned(&self, dest: AddressHash, entry: &PathEntry) {
        let now = Instant::now();
        let mut table = self.tunnels.table.lock().unwrap();
        if table.is_empty() {
            return;
        }
        let mut owner = None;
        for (id, tunnel) in table.iter_mut() {
            if tunnel.iface == Some(entry.iface) {
                owner = Some(*id);
            } else {
                tunnel.paths.remove(&dest);
            }
        }
        let Some(owner) = owner else { return };
        if !table[&owner].paths.contains_key(&dest) {
            Tunnels::make_path_room(&mut table);
        }
        if let Some(tunnel) = table.get_mut(&owner) {
            let mirrored = Mirrored {
                entry: *entry,
                at: now,
            };
            tunnel.paths.insert(dest, mirrored);
            tunnel.expires = now + TUNNEL_TIMEOUT;
        }
    }

    /// Interface `id` is gone; its routes have been culled (`void_tunnel_interface`,
    /// `Transport.py` 2813-2817).
    pub(super) fn tunnel_interface_lost(&self, id: InterfaceId) {
        for tunnel in self.tunnels.table.lock().unwrap().values_mut() {
            if tunnel.iface == Some(id) {
                tunnel.iface = None;
            }
        }
    }

    /// Forget the tunnels on interface `id` (`Transport.py` 3703).
    pub(super) fn drop_tunnels_on(&self, id: InterfaceId) {
        self.tunnels
            .table
            .lock()
            .unwrap()
            .retain(|_, tunnel| tunnel.iface != Some(id));
    }

    #[cfg(test)]
    pub(super) fn tunnel_count(&self) -> usize {
        self.tunnels.table.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::Endpoint;
    use crate::endpoint::router::route;

    fn endpoint() -> Endpoint {
        Endpoint::new(PrivateIdentity::from_secret_bytes(&[9; 64]))
    }

    fn dest(byte: u8) -> AddressHash {
        AddressHash::from_bytes([byte; 16])
    }

    #[test]
    fn synthesis_is_176_signed_bytes_naming_a_stable_tunnel() {
        let client = PrivateIdentity::from_secret_bytes(&[1; 64]);
        let (a, b) = (synthesis(&client, "x"), synthesis(&client, "x"));
        assert_eq!(a.payload.len(), 176);
        assert_ne!(a.payload, b.payload, "fresh random bytes each time");
        let mut id_data = client.public().to_public_bytes().to_vec();
        id_data.extend_from_slice(&full_hash(b"x"));
        assert_eq!(verify(&a.payload), Some(full_hash(&id_data)));
        assert_eq!(verify(&a.payload), verify(&b.payload));
        let mut forged = a.payload.clone();
        forged[100] ^= 1;
        assert_eq!(verify(&forged), None);
        assert_eq!(verify(&a.payload[..175]), None);
    }

    #[tokio::test]
    async fn invalid_synthesis_is_consumed_as_a_protocol_violation() {
        let ep = endpoint();
        let iface = ep.attach_interface().id();
        let mut pkt = synthesis(&PrivateIdentity::from_secret_bytes(&[1; 64]), "x");
        pkt.payload.pop();
        assert!(ep.shared.tunnel_intercept(iface, &pkt));
        assert_eq!(ep.shared.tunnel_count(), 0);
        assert_eq!(ep.interface_counters(iface).unwrap().protocol_violations, 1);
    }

    /// A client's routes survive its reconnect onto a new interface, with no new announce.
    #[tokio::test]
    async fn reappearing_tunnel_restores_its_paths() {
        let ep = endpoint();
        let client = PrivateIdentity::from_secret_bytes(&[1; 64]);
        let first = ep.attach_interface().id();
        route(&ep.shared, first, synthesis(&client, "c"));
        assert_eq!(ep.shared.tunnel_count(), 1);
        ep.shared.learn_path(dest(1), first, 2, None);
        ep.detach_interface(first);
        assert!(ep.route_to(dest(1)).is_none());

        let second = ep.attach_interface().id();
        route(&ep.shared, second, synthesis(&client, "c"));
        assert_eq!(ep.route_to(dest(1)), Some((second, 2)));
        assert_eq!(ep.routing_counters().tunnel_paths_restored, 1);
    }

    #[tokio::test]
    async fn restore_keeps_a_shorter_or_newer_route() {
        let ep = endpoint();
        let client = PrivateIdentity::from_secret_bytes(&[1; 64]);
        let (tunnel, other) = (ep.attach_interface().id(), ep.attach_interface().id());
        route(&ep.shared, tunnel, synthesis(&client, "c"));
        ep.shared.learn_path(dest(1), tunnel, 3, None);
        ep.shared.learn_path(dest(2), tunnel, 3, None);
        // A shorter live route stays; the tunnel copy survives its own interface's loss.
        ep.detach_interface(tunnel);
        ep.shared.path_table.lock().unwrap().insert(
            dest(1),
            PathEntry {
                iface: other,
                transport: None,
                hops: 1,
                learned: Instant::now(),
                mode: crate::node::InterfaceMode::Full,
            },
        );
        // A route learned for dest(2) elsewhere is newer and supersedes the mirrored one.
        ep.shared.learn_path(dest(2), other, 5, None);
        let back = ep.attach_interface().id();
        route(&ep.shared, back, synthesis(&client, "c"));
        assert_eq!(ep.route_to(dest(1)), Some((other, 1)));
        assert_eq!(ep.route_to(dest(2)), Some((other, 5)));
        assert_eq!(ep.routing_counters().tunnel_paths_restored, 0);
    }

    #[test]
    fn tunnels_and_paths_expire_after_eight_hours() {
        let start = Instant::now();
        let mut table = HashMap::new();
        let mut paths = HashMap::new();
        let entry = PathEntry {
            iface: 0,
            transport: None,
            hops: 0,
            learned: start,
            mode: crate::node::InterfaceMode::Full,
        };
        paths.insert(dest(1), Mirrored { entry, at: start });
        table.insert(
            [1; HASH_LEN],
            Tunnel {
                iface: None,
                paths,
                expires: start + TUNNEL_TIMEOUT + TUNNEL_TIMEOUT,
            },
        );
        Tunnels::cull(&mut table, start + TUNNEL_TIMEOUT);
        assert!(table[&[1; HASH_LEN]].paths.is_empty());
        Tunnels::cull(&mut table, start + TUNNEL_TIMEOUT * 2);
        assert!(table.is_empty());
    }

    #[test]
    fn tunnel_paths_are_bounded_across_tunnels() {
        let start = Instant::now();
        let mut table = HashMap::new();
        let mut paths = HashMap::new();
        for n in 0..TUNNEL_PATH_CAPACITY {
            let entry = PathEntry {
                iface: 0,
                transport: None,
                hops: 0,
                learned: start,
                mode: crate::node::InterfaceMode::Full,
            };
            let at = start + Duration::from_secs(n as u64);
            paths.insert(dest(n as u8), Mirrored { entry, at });
        }
        let expires = start + TUNNEL_TIMEOUT;
        table.insert(
            [1; HASH_LEN],
            Tunnel {
                iface: None,
                paths,
                expires,
            },
        );
        Tunnels::make_path_room(&mut table);
        let held = &table[&[1; HASH_LEN]].paths;
        assert_eq!(held.len(), TUNNEL_PATH_CAPACITY - 1);
        assert!(!held.contains_key(&dest(0)), "the oldest goes");
    }
}
