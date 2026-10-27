//! I2PInterface: HDLC over SAM STREAM sessions through a local I2P router (`I2PInterface.py`
//! 76-906, read at RNS 1.5.6).
//!
//! Stock RNS reaches SAM through i2plib's client and server tunnels, which forward local TCP
//! ports; the peer on the far side sees only the stream. This speaks SAM directly with
//! [`truchman`], so the bytes across I2P are the same HDLC frames, keepalive included.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{Notify, mpsc};
use truchman::{
    Bridge, Destination, Key, PrivateKey, SIGNATURE_ED25519, SessionOptions, StreamSession,
};

use crate::hash::AddressHash;
use crate::ifac::Ifac;
use crate::node::InterfaceMode;

use super::dial::synthesize;
use super::iface_policy::IfacePolicy;
use super::interface::InterfaceId;
use super::mtu::HwMtu;
use super::pump::{Framing, Liveness, PumpEnd, carry_live};
use super::runtime::{Endpoint, endpoint_closed, track};
use super::shared::Shared;

/// The I2P interface's hardware MTU (`I2PInterface.py` 340, 737). It is fixed: links stay at
/// the protocol MTU, as neither AUTOCONFIGURE_MTU nor FIXED_MTU is set.
pub const I2P_HW_MTU: usize = 1064;
/// RNS's bitrate guess for I2P (`I2PInterface.py` 719).
pub const I2P_BITRATE_GUESS: u64 = 256_000;
/// An empty HDLC frame, RNS's I2P keepalive (`I2PInterface.py` 584).
const KEEPALIVE: &[u8] = &[0x7E, 0x7E];
/// Back-off after a failed accept on a live session, so a refusing bridge does not spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(250);

/// RNS's I2P timings (`I2PInterface.py` 323-331, 418, 507-533, 569-603, 805).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I2pTimers {
    /// Send a keepalive when nothing has been written for this long: 10 s.
    pub keepalive_after: Duration,
    /// End a stream nothing has been read from for this long: `(9 * 5 + 10) * 2` = 110 s.
    pub read_idle: Duration,
    /// Wait before redialing a peer whose stream ended: 15 s.
    pub reconnect_wait: Duration,
    /// Wait before retrying a peer's session or name lookup: 8 s.
    pub client_tunnel_retry: Duration,
    /// Wait before recreating a listener's session: 15 s.
    pub server_tunnel_retry: Duration,
}

impl Default for I2pTimers {
    fn default() -> Self {
        Self {
            keepalive_after: Duration::from_secs(10),
            read_idle: Duration::from_secs(110),
            reconnect_wait: Duration::from_secs(15),
            client_tunnel_retry: Duration::from_secs(8),
            server_tunnel_retry: Duration::from_secs(15),
        }
    }
}

impl I2pTimers {
    fn liveness(&self) -> Liveness {
        Liveness {
            keepalive_after: self.keepalive_after,
            keepalive: KEEPALIVE,
            read_idle: self.read_idle,
        }
    }
}

/// One I2P peer to dial: an entry of RNS's `peers` list.
#[derive(Clone, Debug)]
pub struct I2pPeer {
    /// The router's SAM bridge.
    pub bridge: Bridge,
    /// The peer: a `.b32.i2p` name, a host name, or a base64 destination.
    pub peer: String,
    /// RNS's peer interface name, `"<interface> to <peer>"`; its tunnel is synthesized
    /// under `I2PInterfacePeer[<name>]`.
    pub name: String,
    /// Access code; a stock I2P interface without `ifac_size` uses [`Ifac::for_stream`].
    pub ifac: Option<Ifac>,
    /// Options for the peer's transient session. Empty leaves tunnels at the router's
    /// defaults, as RNS does.
    pub session: SessionOptions,
    /// Liveness and retry timings.
    pub timers: I2pTimers,
    /// Drop the session and the resolved destination after a stream ends or a connect is
    /// refused, so the next redial starts from a fresh session and lookup. On by default:
    /// a kept session holds the router's stale state for a peer that restarted, so on
    /// emissary every connect is refused and on i2pd the stream opens but reaches nothing
    /// (I2P4 receipt, runs 7-9). Off keeps one session for the interface's life, as RNS's
    /// client tunnel does, saving a session build on each redial.
    pub fresh_session_per_redial: bool,
}

impl I2pPeer {
    /// A peer of interface `interface`, with RNS's defaults.
    pub fn new(bridge: Bridge, interface: &str, peer: impl Into<String>) -> Self {
        let peer = peer.into();
        Self {
            bridge,
            name: format!("{interface} to {peer}"),
            peer,
            ifac: None,
            session: SessionOptions::default(),
            timers: I2pTimers::default(),
            fresh_session_per_redial: true,
        }
    }

    fn label(&self) -> String {
        format!("I2PInterfacePeer[{}]", self.name)
    }
}

/// A connectable I2P interface: a stored destination that accepts streams, each becoming an
/// interface that inherits these settings (`I2PInterface.py` 822-870).
#[derive(Clone, Debug)]
pub struct I2pListen {
    /// The router's SAM bridge.
    pub bridge: Bridge,
    /// The destination to be reachable at; see [`i2p_keyfile`].
    pub key: PrivateKey,
    /// Mode of each spawned interface.
    pub mode: InterfaceMode,
    /// Access code each spawned interface requires.
    pub ifac: Option<Ifac>,
    /// Announce and transmit policy of each spawned interface. Its bitrate defaults to
    /// [`I2P_BITRATE_GUESS`].
    pub iface: IfacePolicy,
    /// Told each spawned interface's id; advisory, as [`super::ListenPolicy::spawned`] is.
    pub spawned: Option<mpsc::Sender<InterfaceId>>,
    /// Options for the listening session.
    pub session: SessionOptions,
    /// Liveness and retry timings.
    pub timers: I2pTimers,
}

impl I2pListen {
    /// Listen at `key` with RNS's defaults.
    pub fn new(bridge: Bridge, key: PrivateKey) -> Self {
        Self {
            bridge,
            key,
            mode: InterfaceMode::Full,
            ifac: None,
            iface: IfacePolicy::default(),
            spawned: None,
            session: SessionOptions::default(),
            timers: I2pTimers::default(),
        }
    }
}

/// A running I2P listener. Dropping the handle leaves it listening; [`Self::close`] stops it.
#[derive(Debug)]
pub struct I2pListener {
    destination: Destination,
    stop: Arc<Notify>,
}

impl I2pListener {
    /// The destination peers dial.
    pub fn destination(&self) -> &Destination {
        &self.destination
    }

    /// Stop accepting and close the session. Streams already accepted keep their interfaces.
    pub fn close(&self) {
        self.stop.notify_one();
    }
}

impl Endpoint {
    /// Dial an I2P peer. Returns at once, as RNS does: building tunnels can take minutes, so
    /// the interface comes online when its first stream connects. An ended stream or a
    /// refused connect is redialed every `reconnect_wait` under the same id, from a fresh
    /// session unless [`I2pPeer::fresh_session_per_redial`] is off; a dead session is
    /// recreated either way.
    pub fn attach_i2p_peer(&self, peer: I2pPeer) -> io::Result<InterfaceId> {
        if !self.shared.is_running() {
            return Err(endpoint_closed());
        }
        let attached = self.shared.add_interface(
            I2P_HW_MTU,
            peer.ifac.clone(),
            InterfaceMode::Full,
            false,
            HwMtu::Protocol,
        );
        if !attached.registered {
            return Err(endpoint_closed());
        }
        let (id, online, mut out) = (attached.id, attached.online, attached.outbound);
        self.set_interface_bitrate(id, Some(I2P_BITRATE_GUESS));
        let shared = Arc::clone(&self.shared);
        let started = track(&self.shared, async move {
            let (label, liveness) = (peer.label(), peer.timers.liveness());
            let mut session: Option<StreamSession> = None;
            let mut target: Option<Destination> = None;
            loop {
                if shared.with_iface(id, |_| ()).is_none() {
                    return;
                }
                if !session.as_ref().is_some_and(StreamSession::is_alive) {
                    session =
                        StreamSession::create(peer.bridge, Key::Transient, peer.session.clone())
                            .await
                            .ok();
                }
                // Resolved through the session, since some routers (emissary) answer
                // `.b32.i2p` lookups only inside one.
                if target.is_none()
                    && let Some(live) = &session
                {
                    target = live.resolve(&peer.peer).await.ok();
                }
                let (Some(live), Some(to)) = (&session, &target) else {
                    tokio::time::sleep(peer.timers.client_tunnel_retry).await;
                    continue;
                };
                if let Ok(stream) = live.connect(to).await {
                    let (reader, writer) = stream.into_split();
                    // Online and discarded first, so carry_live keeps what is queued here.
                    if !online.swap(true, Ordering::AcqRel) {
                        out.discard();
                    }
                    synthesize(&shared, id, &label);
                    let end = carry_live(
                        &shared,
                        id,
                        &online,
                        &mut out,
                        Framing::Hdlc,
                        Some(liveness),
                        reader,
                        writer,
                    )
                    .await;
                    if end == PumpEnd::Closed {
                        return;
                    }
                }
                if peer.fresh_session_per_redial {
                    session = None;
                    target = None;
                }
                tokio::time::sleep(peer.timers.reconnect_wait).await;
            }
        });
        if !started {
            self.shared.forget_interface(id);
            return Err(endpoint_closed());
        }
        Ok(id)
    }

    /// Accept I2P streams at `listen.key`; every accepted stream becomes an interface. A
    /// session the router drops is recreated every `server_tunnel_retry`.
    pub fn listen_i2p(&self, listen: I2pListen) -> io::Result<I2pListener> {
        if !self.shared.is_running() {
            return Err(endpoint_closed());
        }
        let destination = listen.key.destination().clone();
        let stop = Arc::new(Notify::new());
        let (shared, stopped) = (Arc::clone(&self.shared), Arc::clone(&stop));
        let started = track(&self.shared, async move {
            loop {
                let key = Key::Persistent(listen.key.clone());
                let created = tokio::select! {
                    biased;
                    () = stopped.notified() => return,
                    created = StreamSession::create(listen.bridge, key, listen.session.clone()) => created,
                };
                if let Ok(session) = created {
                    loop {
                        let accepted = tokio::select! {
                            biased;
                            () = stopped.notified() => return,
                            () = session.closed() => break,
                            accepted = session.accept() => accepted,
                        };
                        match accepted {
                            Ok((stream, _peer)) => {
                                if !shared.is_running() {
                                    return;
                                }
                                let (id, attached) = spawn(&shared, stream, &listen);
                                if let (true, Some(spawned)) = (attached, &listen.spawned) {
                                    let _ = spawned.try_send(id);
                                }
                            }
                            Err(_) if !session.is_alive() => break,
                            Err(_) => tokio::time::sleep(ACCEPT_RETRY).await,
                        }
                    }
                }
                tokio::select! {
                    biased;
                    () = stopped.notified() => return,
                    () = tokio::time::sleep(listen.timers.server_tunnel_retry) => {}
                }
            }
        });
        if !started {
            return Err(endpoint_closed());
        }
        Ok(I2pListener { destination, stop })
    }
}

/// Attach one accepted stream under `listen`'s settings, carried until it ends and then
/// forgotten. An accepted peer synthesizes no tunnel; only the initiator does (503, 533).
fn spawn(
    shared: &Arc<Shared>,
    stream: truchman::Stream,
    listen: &I2pListen,
) -> (InterfaceId, bool) {
    let attached = shared.add_interface(
        I2P_HW_MTU,
        listen.ifac.clone(),
        listen.mode,
        true,
        HwMtu::Protocol,
    );
    let id = attached.id;
    if !attached.registered {
        return (id, false);
    }
    let mut iface = listen.iface;
    iface.bitrate_bps.get_or_insert(I2P_BITRATE_GUESS);
    shared.iface_policies.lock().unwrap().insert(id, iface);
    let (online, mut outbound) = (attached.online, attached.outbound);
    let liveness = listen.timers.liveness();
    let owner = Arc::clone(shared);
    let started = track(shared, async move {
        let (reader, writer) = stream.into_split();
        carry_live(
            &owner,
            id,
            &online,
            &mut outbound,
            Framing::Hdlc,
            Some(liveness),
            reader,
            writer,
        )
        .await;
        owner.forget_interface(id);
    });
    if !started {
        shared.forget_interface(id);
    }
    (id, started)
}

/// Where RNS keeps interface `name`'s destination key under `storage`, its `storagepath`
/// (`I2PInterface.py` 229-239): `i2p/<hex(H(H(name) + H(transport)))>.i2p`, or the older
/// `i2p/<hex(H(H(name)))>.i2p` when that file exists. `H` is SHA-256 and `transport` the
/// node's transport identity hash, so a node keeps its b32 address across RNS and retinue.
pub fn i2p_keyfile(storage: &Path, name: &str, transport: AddressHash) -> PathBuf {
    let dir = storage.join("i2p");
    let name_hash = Sha256::digest(name.as_bytes());
    let old = dir.join(format!("{}.i2p", hex(&Sha256::digest(name_hash))));
    if old.is_file() {
        return old;
    }
    let mut both = Vec::with_capacity(64);
    both.extend_from_slice(&name_hash);
    both.extend_from_slice(&Sha256::digest(transport.as_slice()));
    dir.join(format!("{}.i2p", hex(&Sha256::digest(&both))))
}

/// The key stored at `path`, or a new one generated through `bridge` and stored there, as
/// RNS does on first start (`I2PInterface.py` 241-252). The file holds SAM's base64 private
/// key and nothing else; on unix it is created readable by its owner only.
pub async fn load_or_generate_i2p_key(bridge: Bridge, path: &Path) -> io::Result<PrivateKey> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(PrivateKey::from_base64(&text)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = bridge.generate(SIGNATURE_ED25519).await?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            io::Write::write_all(&mut options.open(path)?, key.expose_base64().as_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
