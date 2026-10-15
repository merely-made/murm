//! Each RNS interface type's own keys, mapped onto retinue's carrier configs
//! (`Reticulum.py` 1104-1160 and each interface's `__init__`).

use alloc::string::{String, ToString};

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

#[cfg(any(feature = "serial", feature = "tulle-radio", feature = "pipe"))]
use crate::ifac::SERIAL_SIZE;
use crate::ifac::STREAM_SIZE;
use crate::iface::udp::{UdpConfig, UdpHandle};

use super::super::dial::TcpClient;
use super::super::interface::InterfaceId;
use super::super::listen::{FlapPolicy, ListenAddr, ListenPolicy, Listener};
use super::super::mtu::HwMtu;
use super::super::runtime::Endpoint;
use super::{InterfaceSettings, Keys, RnsConfigError, RnsInterface};

/// Backbone's bitrate guess, which sets its hardware MTU (`BackboneInterface.py` 44, 844).
const BACKBONE_BITRATE_GUESS: u64 = 100_000_000;

/// A station ID beacon: `id_callsign` sent every `id_interval` seconds (KISS and RNode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdBeacon {
    pub callsign: String,
    pub interval: Duration,
}

/// An RNS UDPInterface's addresses. A `device` stands for its subnet broadcast address
/// wherever an IP is unset (`UDPInterface.py` 83-87), resolved by [`Self::config`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UdpCarrier {
    pub listen_ip: Option<Ipv4Addr>,
    pub listen_port: Option<u16>,
    pub forward_ip: Option<Ipv4Addr>,
    pub forward_port: Option<u16>,
    pub device: Option<String>,
}

impl UdpCarrier {
    /// The carrier's [`UdpConfig`], resolving `device` on this host.
    pub fn config(&self) -> io::Result<UdpConfig> {
        #[cfg(all(feature = "auto", unix))]
        let broadcast = match &self.device {
            Some(device) => UdpConfig::device(device, 0, 0)?.listen.map(|a| *a.ip()),
            None => None,
        };
        #[cfg(not(all(feature = "auto", unix)))]
        let broadcast = match &self.device {
            Some(_) => return Err(io::ErrorKind::Unsupported.into()),
            None => None,
        };
        let addr = |ip: Option<Ipv4Addr>, port| Some(SocketAddrV4::new(ip.or(broadcast)?, port?));
        Ok(UdpConfig {
            listen: addr(self.listen_ip, self.listen_port),
            forward: addr(self.forward_ip, self.forward_port),
            reuse_address: false,
        })
    }
}

/// The carrier an RNS interface section configures, in retinue's own config types.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Carrier {
    /// `TCPClientInterface`, or a `BackboneInterface` with a `target_host`/`remote`. A
    /// `fixed_mtu` is the client's [`HwMtu::Fixed`].
    TcpClient { client: TcpClient, backbone: bool },
    /// `TCPServerInterface`, or a listening `BackboneInterface` with fast-flap blocking in
    /// `policy.flap`.
    TcpServer {
        addr: ListenAddr,
        policy: ListenPolicy,
        backbone: bool,
    },
    /// `UDPInterface`.
    Udp(UdpCarrier),
    /// `AutoInterface`.
    #[cfg(all(feature = "auto", unix))]
    Auto(crate::iface::auto::AutoConfig),
    /// `SerialInterface`.
    #[cfg(feature = "serial")]
    Serial(crate::iface::serial::SerialConfig),
    /// `KISSInterface`.
    #[cfg(feature = "serial")]
    Kiss {
        port: crate::iface::serial::SerialConfig,
        tnc: crate::iface::kiss::TncConfig,
        beacon: Option<IdBeacon>,
    },
    /// `AX25KISSInterface`: its `callsign` and `ssid` (`None` for RNS's -1) are kept as data.
    #[cfg(feature = "serial")]
    Ax25Kiss {
        port: crate::iface::serial::SerialConfig,
        tnc: crate::iface::kiss::TncConfig,
        callsign: String,
        ssid: Option<u8>,
    },
    /// `RNodeInterface`.
    #[cfg(feature = "tulle-radio")]
    RNode {
        port: tulle::serial::RNodePort,
        config: tulle::rnode::RNodeConfig,
        beacon: Option<IdBeacon>,
    },
    /// `PipeInterface`.
    #[cfg(feature = "pipe")]
    Pipe(super::super::pipe::PipeConfig),
}

/// Map one section's carrier keys, with the carrier's default IFAC size (A2).
pub(super) fn carrier(kind: &str, keys: &Keys<'_>) -> Result<(Carrier, usize), RnsConfigError> {
    let carrier = match kind {
        "TCPClientInterface" => tcp_client(keys, "target_host", "target_port", false)?,
        "BackboneClientInterface" => backbone_client(keys)?,
        "BackboneInterface" if keys.str("target_host").or(keys.str("remote")).is_some() => {
            backbone_client(keys)?
        }
        "TCPServerInterface" => tcp_server(keys, false)?,
        "BackboneInterface" => tcp_server(keys, true)?,
        "UDPInterface" => udp(keys)?,
        #[cfg(all(feature = "auto", unix))]
        "AutoInterface" => auto(keys)?,
        #[cfg(feature = "serial")]
        "SerialInterface" | "KISSInterface" | "AX25KISSInterface" => {
            return Ok((super::radio::serial(kind, keys)?, SERIAL_SIZE));
        }
        #[cfg(feature = "tulle-radio")]
        "RNodeInterface" => return Ok((super::radio::rnode(keys)?, SERIAL_SIZE)),
        #[cfg(feature = "pipe")]
        "PipeInterface" => return Ok((pipe(keys)?, SERIAL_SIZE)),
        _ => {
            return Err(RnsConfigError::UnsupportedType {
                interface: keys.name.to_string(),
                kind: kind.to_string(),
            });
        }
    };
    Ok((carrier, STREAM_SIZE))
}

fn backbone_client(keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    // `remote` and `port` overwrite their target_ spellings (`Reticulum.py` 1116-1118).
    let host = if keys.str("remote").is_some() {
        "remote"
    } else {
        "target_host"
    };
    let port = if keys.str("port").is_some() {
        "port"
    } else {
        "target_port"
    };
    tcp_client(keys, host, port, true)
}

fn tcp_client(
    keys: &Keys<'_>,
    host: &str,
    port: &str,
    backbone: bool,
) -> Result<Carrier, RnsConfigError> {
    let target = keys.str(host).ok_or_else(|| keys.missing(host))?;
    let port = keys.int(port)?.ok_or_else(|| keys.missing(port))?;
    let mut client = TcpClient::new(target, port);
    client.kiss_framing = !backbone && keys.bool("kiss_framing")?.unwrap_or(false);
    client.i2p_tunneled = keys.bool("i2p_tunneled")?.unwrap_or(false);
    client.prefer_ipv6 = keys.bool("prefer_ipv6")?.unwrap_or(false);
    if let Some(secs) = keys.int::<u64>("connect_timeout")? {
        client.connect_timeout = Duration::from_secs(secs);
    }
    client.max_reconnect_tries = keys.int("max_reconnect_tries")?;
    if backbone {
        client.hw_mtu = HwMtu::Auto {
            bitrate: BACKBONE_BITRATE_GUESS,
        };
    }
    // RNS refuses a fixed MTU below the protocol MTU (`TCPInterface.py` 110-116); one past
    // what a link signals is refused too.
    if let Some(mtu) = keys.int::<usize>("fixed_mtu")?.filter(|&mtu| mtu != 0) {
        client.hw_mtu = HwMtu::Fixed(mtu)
            .checked()
            .map_err(|_| keys.bad("fixed_mtu", &mtu.to_string()))?;
    }
    Ok(Carrier::TcpClient { client, backbone })
}

fn tcp_server(keys: &Keys<'_>, backbone: bool) -> Result<Carrier, RnsConfigError> {
    // `port` stands for `listen_port`, and Backbone's `listen_on` for `listen_ip`
    // (`TCPInterface.py` 523-524; `Reticulum.py` 1115-1119).
    let port_key = if keys.str("port").is_some() {
        "port"
    } else {
        "listen_port"
    };
    let port = keys
        .int(port_key)?
        .ok_or_else(|| keys.missing("listen_port"))?;
    let prefer_ipv6 = keys.bool("prefer_ipv6")?.unwrap_or(false);
    let ip_key = if backbone && keys.str("listen_on").is_some() {
        "listen_on"
    } else {
        "listen_ip"
    };
    let addr = match (keys.str("device"), keys.str(ip_key)) {
        (Some(device), _) => ListenAddr::Device {
            name: device.to_string(),
            port,
            prefer_ipv6,
        },
        (None, Some(ip)) => match ip.parse::<IpAddr>() {
            Ok(ip) => ListenAddr::Socket(SocketAddr::new(ip, port)),
            Err(_) => ListenAddr::Host {
                name: ip.to_string(),
                port,
                prefer_ipv6,
            },
        },
        (None, None) => return Err(keys.missing("listen_ip")),
    };
    let flap = if backbone && keys.bool("block_fast_flapping")?.unwrap_or(true) {
        let defaults = FlapPolicy::default();
        Some(FlapPolicy {
            threshold: keys
                .duration("fast_flapping_threshold", 1.0)?
                .unwrap_or(defaults.threshold),
            grace: keys.int("fast_flapping_grace")?.unwrap_or(defaults.grace),
            block: keys
                .duration("fast_flapping_block_time", 60.0)?
                .unwrap_or(defaults.block),
        })
    } else {
        None
    };
    let policy = ListenPolicy {
        i2p_tunneled: keys.bool("i2p_tunneled")?.unwrap_or(false),
        flap,
        hw_mtu: backbone.then_some(HwMtu::Auto {
            bitrate: BACKBONE_BITRATE_GUESS,
        }),
        ..ListenPolicy::default()
    };
    Ok(Carrier::TcpServer {
        addr,
        policy,
        backbone,
    })
}

fn udp(keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    let ip = |key| keys.parsed(key, |v: &str| v.trim().parse::<Ipv4Addr>().ok());
    let port = keys.int::<u16>("port")?;
    Ok(Carrier::Udp(UdpCarrier {
        listen_ip: ip("listen_ip")?,
        listen_port: keys.int("listen_port")?.or(port),
        forward_ip: ip("forward_ip")?,
        forward_port: keys.int("forward_port")?.or(port),
        device: keys.str("device").map(ToString::to_string),
    }))
}

#[cfg(all(feature = "auto", unix))]
fn auto(keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    use crate::auto::{AddrType, Scope};
    use crate::iface::auto::AutoConfig;
    let mut cfg = AutoConfig::default();
    if let Some(group) = keys.str("group_id") {
        cfg.group_id = group.as_bytes().to_vec();
    }
    // An unknown scope is refused, where stock fails on it (`AutoInterface.py` 190-201).
    if let Some(scope) = keys.parsed("discovery_scope", Scope::from_config)? {
        cfg.scope = scope;
    }
    if let Some(kind) = keys.str("multicast_address_type") {
        cfg.addr_type = AddrType::from_config(kind);
    }
    if let Some(port) = keys.int("discovery_port")? {
        cfg.discovery_port = port;
    }
    if let Some(port) = keys.int("data_port")? {
        cfg.data_port = port;
    }
    if let Some(devices) = keys.str("devices") {
        cfg.devices = super::syntax::list(devices);
    }
    if let Some(devices) = keys.str("ignored_devices") {
        cfg.ignored_devices = super::syntax::list(devices);
    }
    Ok(Carrier::Auto(cfg))
}

#[cfg(feature = "pipe")]
fn pipe(keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    use super::super::pipe::PipeConfig;
    let command = keys.str("command").ok_or_else(|| keys.missing("command"))?;
    let argv = super::syntax::shell_words(command)
        .filter(|words| !words.is_empty())
        .ok_or_else(|| keys.bad("command", command))?;
    let mut cfg = PipeConfig::new(argv.into_iter().map(Into::into).collect());
    if let Some(delay) = keys.duration("respawn_delay", 1.0)? {
        cfg.respawn_delay = delay;
    }
    Ok(Carrier::Pipe(cfg))
}

impl Carrier {
    /// Fill in what the carrier config itself carries: IFAC, a configured bitrate's
    /// autoconfigured MTU (`Reticulum.py` 1038-1039), and for a listener or Auto parent the
    /// mode and policy every spawned interface inherits.
    pub(super) fn with_settings(mut self, settings: &InterfaceSettings) -> Self {
        match &mut self {
            Self::TcpClient { client, .. } => {
                client.ifac = settings.ifac.clone();
                if let (HwMtu::Auto { .. }, Some(bitrate)) = (client.hw_mtu, settings.bitrate) {
                    client.hw_mtu = HwMtu::Auto { bitrate };
                }
            }
            Self::TcpServer { policy, .. } => {
                policy.mode = settings.mode;
                policy.ifac = settings.ifac.clone();
                policy.iface = settings.iface_policy();
                if let Some(bitrate) = settings.bitrate {
                    policy.hw_mtu = Some(HwMtu::Auto { bitrate });
                }
            }
            #[cfg(all(feature = "auto", unix))]
            Self::Auto(cfg) => {
                cfg.ifac = settings.ifac.clone();
                cfg.mode = settings.mode;
                cfg.policy = settings.iface_policy();
            }
            #[cfg(feature = "pipe")]
            Self::Pipe(cfg) => cfg.ifac = settings.ifac.clone(),
            _ => {}
        }
        self
    }
}

/// What [`Endpoint::attach_rns_interface`] attached.
#[derive(Debug)]
#[non_exhaustive]
pub enum RnsAttached {
    /// A dialed TCP interface.
    Interface(InterfaceId),
    /// A TCP listener; its connections inherit the section's settings.
    Listener(Listener),
    /// A UDP carrier.
    Udp(UdpHandle),
}

impl Endpoint {
    /// Attach a TCP client, TCP or Backbone listener, or UDP section and apply its settings.
    /// Other carriers are attached through their own APIs with [`RnsInterface::carrier`]'s
    /// config, then [`Self::apply_interface_settings`]. `enabled` is the caller's to check.
    pub async fn attach_rns_interface(&self, section: &RnsInterface) -> io::Result<RnsAttached> {
        let unsupported = |why: &str| io::Error::new(io::ErrorKind::Unsupported, why.to_string());
        let id = match &section.carrier {
            Carrier::TcpClient { client, .. } => self.attach_tcp(client.clone()).await?,
            Carrier::TcpServer { addr, policy, .. } => {
                return Ok(RnsAttached::Listener(
                    self.listen_on(addr.clone(), policy.clone()).await?,
                ));
            }
            Carrier::Udp(udp) => {
                let handle = self
                    .attach_udp(udp.config()?, section.settings.ifac.clone())
                    .await?;
                self.apply_interface_settings(handle.id, &section.settings);
                return Ok(RnsAttached::Udp(handle));
            }
            #[allow(unreachable_patterns)]
            _ => return Err(unsupported("attach this carrier with its own API")),
        };
        self.apply_interface_settings(id, &section.settings);
        Ok(RnsAttached::Interface(id))
    }
}
