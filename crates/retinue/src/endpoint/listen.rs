//! TCP listeners and the interfaces they spawn per connection (`TCPInterface.py` 511-688).

use alloc::string::String;
use alloc::vec::Vec;

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream, lookup_host};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use crate::ifac::Ifac;
use crate::iface::hdlc;
use crate::node::InterfaceMode;

use super::dial::TCP_BITRATE_GUESS;
use super::iface_policy::IfacePolicy;
use super::interface::InterfaceId;
use super::mtu::HwMtu;
use super::pump::{Framing, carry_tcp};
use super::runtime::{Endpoint, endpoint_closed, track};
use super::shared::Shared;
use super::sockopt;

/// Back-off after a failed `accept`, so descriptor exhaustion does not spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);
/// Remote addresses one listener tracks for fast flapping. RNS's table is unbounded.
const FLAP_TABLE_MAX: usize = 4096;

/// What every connection a listener accepts inherits, as RNS copies a server interface's
/// settings onto each spawned client (`TCPInterface.py` 587-650). Egress control, the
/// path-request rates and `recursive_prs` travel in [`IfacePolicy`]; RNS copies `IN` too, but
/// nothing in 1.5.7 reads it.
#[derive(Clone, Debug, Default)]
pub struct ListenPolicy {
    /// Mode of each spawned interface.
    pub mode: InterfaceMode,
    /// Access code each spawned interface requires.
    pub ifac: Option<Ifac>,
    /// Use RNS's slower keepalive profile for connections tunnelled through I2P.
    pub i2p_tunneled: bool,
    /// Announce and transmit policy of each spawned interface.
    pub iface: IfacePolicy,
    /// Told each spawned interface's id, as RNS lists `spawned_interfaces`. Advisory: an id
    /// that finds the channel full or closed is dropped, so a slow consumer never stalls
    /// accepting.
    pub spawned: Option<mpsc::Sender<InterfaceId>>,
    /// Frame accepted connections with KISS. RNS has no KISS server; this pairs with a stock
    /// `TCPClientInterface` set to `kiss_framing = yes`.
    pub kiss_framing: bool,
    /// Refuse peers that reconnect too fast, as BackboneInterface does; `None` accepts all,
    /// as TCPServerInterface does.
    pub flap: Option<FlapPolicy>,
}

/// Backbone's fast-flap blocking (`BackboneInterface.py` 48-53, 126-161, 807-835).
///
/// Each listener keeps its own table; stock's is one per process, so an address blocked
/// on one listener here can still reach another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlapPolicy {
    /// A connection that ends sooner than this after its accept is a flap.
    pub threshold: Duration,
    /// Flaps tolerated from one address before it is blocked.
    pub grace: u32,
    /// How long an address stays blocked after its last flap.
    pub block: Duration,
}

impl Default for FlapPolicy {
    /// Backbone's defaults: 20 s, 5 flaps, 12 h.
    fn default() -> Self {
        Self {
            threshold: Duration::from_secs(20),
            grace: 5,
            block: Duration::from_secs(12 * 60 * 60),
        }
    }
}

/// Where a listener binds: RNS's `listen_ip`/`listen_port` and `device` keys
/// (`TCPInterface.py` 469-551; `BackboneInterface.py` 84-170).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenAddr {
    /// A resolved address.
    Socket(SocketAddr),
    /// A host name or address literal, resolved on bind.
    Host {
        name: String,
        port: u16,
        prefer_ipv6: bool,
    },
    /// The first address of a network device, with its scope for an IPv6 link-local one.
    Device {
        name: String,
        port: u16,
        prefer_ipv6: bool,
    },
}

/// A running listener. Dropping the handle leaves it listening; [`Self::close`] stops it.
#[derive(Debug)]
pub struct Listener {
    local: SocketAddr,
    stop: Arc<Notify>,
}

impl Listener {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Stop accepting and release the port. Connections already accepted keep their
    /// interfaces, as RNS's server detach leaves its spawned clients (`TCPInterface.py` 675-688).
    pub fn close(&self) {
        self.stop.notify_one();
    }
}

impl Endpoint {
    /// Listen on TCP; every accepted connection becomes an interface. Returns the bound
    /// address (pass port 0 to get an OS-assigned one).
    pub async fn listen_tcp(&self, addr: SocketAddr) -> io::Result<SocketAddr> {
        let listener = self.listen_tcp_with(addr, ListenPolicy::default()).await?;
        Ok(listener.local_addr())
    }

    /// Listen for IFAC-authenticated TCP connections. Stock TCP and Local peers without
    /// `ifac_size` use [`Ifac::for_stream`]'s 16-byte codes.
    pub async fn listen_tcp_with_ifac(
        &self,
        addr: SocketAddr,
        ifac: Ifac,
    ) -> io::Result<SocketAddr> {
        let policy = ListenPolicy {
            ifac: Some(ifac),
            ..ListenPolicy::default()
        };
        Ok(self.listen_tcp_with(addr, policy).await?.local_addr())
    }

    /// Listen on TCP, giving every accepted connection `policy`.
    ///
    /// A failed `accept` (descriptor exhaustion, an aborted handshake) is retried after
    /// 100 ms rather than ending the listener, as RNS's server keeps serving.
    pub async fn listen_tcp_with(
        &self,
        addr: SocketAddr,
        policy: ListenPolicy,
    ) -> io::Result<Listener> {
        if !self.shared.is_running() {
            return Err(endpoint_closed());
        }
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let stop = Arc::new(Notify::new());
        let (shared, stopped) = (Arc::clone(&self.shared), Arc::clone(&stop));
        let started = track(&self.shared, async move {
            // Connection ends come back here, so the flap table has a single owner.
            let (ends, mut ended) = mpsc::unbounded_channel();
            let mut flaps = policy.flap.map(Flaps::new);
            loop {
                let accepted = tokio::select! {
                    biased;
                    () = stopped.notified() => return,
                    Some((ip, at)) = ended.recv() => {
                        if let Some(flaps) = &mut flaps {
                            flaps.ended(ip, at, Instant::now());
                        }
                        continue;
                    }
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, peer)) = accepted else {
                    tokio::time::sleep(ACCEPT_RETRY).await;
                    continue;
                };
                if !shared.is_running() {
                    return;
                }
                let ip = peer.ip().to_canonical();
                let blocked = flaps.as_mut().map(|f| f.blocked(ip, Instant::now()));
                // Refused by closing, as Backbone ignores the connection (807-822).
                if blocked == Some(true) {
                    continue;
                }
                let report = blocked.map(|_| (ends.clone(), ip));
                let (id, attached) = spawn(&shared, stream, &policy, report);
                if let (true, Some(spawned)) = (attached, &policy.spawned) {
                    let _ = spawned.try_send(id);
                }
            }
        });
        if !started {
            return Err(endpoint_closed());
        }
        Ok(Listener { local, stop })
    }

    /// Listen at `addr`, giving every accepted connection `policy`. A device needs the
    /// `auto` feature on unix, and is `Unsupported` otherwise.
    pub async fn listen_on(&self, addr: ListenAddr, policy: ListenPolicy) -> io::Result<Listener> {
        self.listen_tcp_with(addr.resolve().await?, policy).await
    }

    /// Attach a connected TCP stream as an interface, and return its id.
    pub fn attach_stream(&self, stream: TcpStream) -> InterfaceId {
        spawn(&self.shared, stream, &ListenPolicy::default(), None).0
    }

    /// Attach an IFAC-authenticated connected TCP stream; see [`Ifac::for_stream`].
    pub fn attach_stream_with_ifac(&self, stream: TcpStream, ifac: Ifac) -> InterfaceId {
        let policy = ListenPolicy {
            ifac: Some(ifac),
            ..ListenPolicy::default()
        };
        spawn(&self.shared, stream, &policy, None).0
    }
}

/// Where a connection reports its remote address and accept time when it ends.
type EndReport = (mpsc::UnboundedSender<(IpAddr, Instant)>, IpAddr);

/// Attach one accepted connection under `policy`, carried until it ends and then forgotten.
/// The flag is false if the endpoint has stopped.
fn spawn(
    shared: &Arc<Shared>,
    stream: TcpStream,
    policy: &ListenPolicy,
    report: Option<EndReport>,
) -> (InterfaceId, bool) {
    let accepted_at = Instant::now();
    let _ = sockopt::tune(&stream, policy.i2p_tunneled);
    let hw_mtu = HwMtu::Auto {
        bitrate: TCP_BITRATE_GUESS,
    };
    let attached = shared.add_interface(
        hdlc::MAX_FRAME,
        policy.ifac.clone(),
        policy.mode,
        true,
        hw_mtu,
    );
    let id = attached.id;
    if !attached.registered {
        return (id, false);
    }
    if policy.iface != IfacePolicy::default() {
        shared
            .iface_policies
            .lock()
            .unwrap()
            .insert(id, policy.iface);
    }
    let (online, mut outbound) = (attached.online, attached.outbound);
    let framing = if policy.kiss_framing {
        Framing::Kiss
    } else {
        Framing::Hdlc
    };
    let owner = Arc::clone(shared);
    let started = track(shared, async move {
        carry_tcp(&owner, id, &online, &mut outbound, stream, framing).await;
        if let Some((ends, ip)) = report {
            let _ = ends.send((ip, accepted_at));
        }
        owner.forget_interface(id);
    });
    if !started {
        shared.forget_interface(id);
    }
    (id, started)
}

impl ListenAddr {
    /// The socket address to bind (`TCPInterface.py` 469-501).
    async fn resolve(self) -> io::Result<SocketAddr> {
        match self {
            Self::Socket(addr) => Ok(addr),
            Self::Host {
                name,
                port,
                prefer_ipv6,
            } => {
                let addrs: Vec<SocketAddr> = lookup_host((name.as_str(), port)).await?.collect();
                // The first of the preferred family, else the resolver's first.
                let addr = addrs.iter().find(|a| a.is_ipv6() == prefer_ipv6);
                addr.or(addrs.first()).copied().ok_or_else(no_address)
            }
            Self::Device {
                name,
                port,
                prefer_ipv6,
            } => device_addr(&name, port, prefer_ipv6),
        }
    }
}

fn no_address() -> io::Error {
    io::Error::new(io::ErrorKind::AddrNotAvailable, "no address to listen on")
}

/// The device's first IPv4 address, or its first IPv6 one when preferred or the only family;
/// a link-local one is scoped to the device (`TCPInterface.py` 469-487).
#[cfg(all(feature = "auto", unix))]
fn device_addr(name: &str, port: u16, prefer_ipv6: bool) -> io::Result<SocketAddr> {
    let found = crate::iface::netinfo::first_addresses(name)?;
    bind_on(
        found.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?,
        port,
        prefer_ipv6,
    )
}

#[cfg(all(feature = "auto", unix))]
fn bind_on(
    found: crate::iface::netinfo::DeviceAddrs,
    port: u16,
    prefer_ipv6: bool,
) -> io::Result<SocketAddr> {
    use std::net::SocketAddrV6;
    match (found.v4, found.v6) {
        (v4, Some(v6)) if prefer_ipv6 || v4.is_none() => {
            let scope = if v6.segments()[0] == 0xfe80 {
                found.index
            } else {
                0
            };
            Ok(SocketAddrV6::new(v6, port, 0, scope).into())
        }
        (Some(v4), _) => Ok((v4, port).into()),
        _ => Err(no_address()),
    }
}

#[cfg(not(all(feature = "auto", unix)))]
fn device_addr(_name: &str, _port: u16, _prefer_ipv6: bool) -> io::Result<SocketAddr> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Fast-flap records per remote address: first flap, last flap and count
/// (`BackboneInterface.py` 807-835, 1084-1109). Never reset but by expiry, as in RNS.
struct Flaps {
    policy: FlapPolicy,
    seen: HashMap<IpAddr, (Instant, Instant, u32)>,
}

impl Flaps {
    fn new(policy: FlapPolicy) -> Self {
        Self {
            policy,
            seen: HashMap::new(),
        }
    }

    /// Whether `ip` has flapped more than the grace allows, within the block time.
    fn blocked(&mut self, ip: IpAddr, now: Instant) -> bool {
        let Some(&(_, last, count)) = self.seen.get(&ip) else {
            return false;
        };
        if now.duration_since(last) > self.policy.block {
            self.seen.remove(&ip);
            return false;
        }
        count > self.policy.grace
    }

    /// A connection from `ip` accepted at `accepted` has ended at `now`.
    fn ended(&mut self, ip: IpAddr, accepted: Instant, now: Instant) {
        if now.duration_since(accepted) >= self.policy.threshold {
            return;
        }
        if !self.seen.contains_key(&ip) && self.seen.len() >= FLAP_TABLE_MAX {
            let block = self.policy.block;
            self.seen.retain(|_, e| now.duration_since(e.1) <= block);
            if self.seen.len() >= FLAP_TABLE_MAX
                && let Some(stalest) = self.seen.iter().min_by_key(|e| e.1.1).map(|e| *e.0)
            {
                self.seen.remove(&stalest);
            }
        }
        let entry = self.seen.entry(ip).or_insert((now, now, 0));
        entry.1 = now;
        entry.2 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> FlapPolicy {
        FlapPolicy {
            threshold: Duration::from_secs(20),
            grace: 2,
            block: Duration::from_secs(60),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flaps_block_after_grace_until_expiry() {
        let mut flaps = Flaps::new(policy());
        let (a, b): (IpAddr, IpAddr) = ("10.0.0.1".parse().unwrap(), "::1".parse().unwrap());
        let t0 = Instant::now();
        flaps.ended(a, t0, t0 + Duration::from_secs(20)); // lived the threshold: no flap
        for _ in 0..3 {
            assert!(!flaps.blocked(a, Instant::now()));
            tokio::time::advance(Duration::from_secs(1)).await;
            let now = Instant::now();
            flaps.ended(a, now - Duration::from_secs(1), now);
        }
        assert!(flaps.blocked(a, Instant::now()));
        assert!(!flaps.blocked(b, Instant::now()));
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            flaps.blocked(a, Instant::now()),
            "block runs from the last flap"
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!flaps.blocked(a, Instant::now()));
        assert!(flaps.seen.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn flap_table_is_bounded() {
        let mut flaps = Flaps::new(policy());
        let now = Instant::now();
        for i in 0..=FLAP_TABLE_MAX as u32 {
            flaps.ended(IpAddr::from(i.to_be_bytes()), now, now);
        }
        assert_eq!(flaps.seen.len(), FLAP_TABLE_MAX);
    }

    #[tokio::test]
    async fn hosts_resolve_by_family_preference() {
        let host = |prefer_ipv6| ListenAddr::Host {
            name: "localhost".into(),
            port: 7,
            prefer_ipv6,
        };
        let v4 = host(false).resolve().await.unwrap();
        assert_eq!(v4, "127.0.0.1:7".parse().unwrap());
        let v6 = host(true).resolve().await.unwrap();
        assert!(v6.ip().is_loopback() && v6.port() == 7);
    }

    #[cfg(all(feature = "auto", unix))]
    #[tokio::test]
    async fn devices_resolve_to_their_first_address() {
        let all = crate::iface::netinfo::interfaces().unwrap();
        let lo = all
            .iter()
            .find(|i| i.v4.iter().any(|(a, _)| a.is_loopback()));
        let device = |name: &str, prefer_ipv6| ListenAddr::Device {
            name: name.into(),
            port: 9,
            prefer_ipv6,
        };
        let v4 = device(&lo.unwrap().name, false).resolve().await.unwrap();
        assert_eq!(v4, "127.0.0.1:9".parse().unwrap());
        let missing = device("no-such-if0", false).resolve().await.unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);

        // A link-local address carries its device as scope; IPv4 wins unless IPv6 is preferred.
        let found = crate::iface::netinfo::DeviceAddrs {
            index: 4,
            v4: Some("10.1.2.3".parse().unwrap()),
            v6: Some("fe80::1".parse().unwrap()),
        };
        assert_eq!(
            bind_on(found, 9, false).unwrap(),
            "10.1.2.3:9".parse().unwrap()
        );
        assert_eq!(
            bind_on(found, 9, true).unwrap(),
            "[fe80::1%4]:9".parse().unwrap()
        );
        let only_v6 = crate::iface::netinfo::DeviceAddrs { v4: None, ..found };
        assert_eq!(
            bind_on(only_v6, 9, false).unwrap(),
            "[fe80::1%4]:9".parse().unwrap()
        );
    }
}
