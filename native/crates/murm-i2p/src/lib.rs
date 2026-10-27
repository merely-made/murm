//! An embedded emissary I2P router for murm hosts.
//!
//! The router runs in-process and is reached only over its SAMv3 bridge, which it binds to
//! loopback on an OS-assigned port. A host takes [`EmbeddedRouter::bridge`] and uses it
//! wherever an external router's bridge would go, so retinue's I2P interface and anything
//! else speaking SAM through [`truchman`] work the same against either.
//!
//! Embedding makes the host an I2P participant, so the choices that cost bandwidth, ports
//! and battery are the host's: [`RouterConfig`] has no defaults for transit or publishing.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use emissary_core::router::Router;
use emissary_core::{Config, Ntcp2Config, SamConfig, Ssu2Config, TransitConfig};
use emissary_util::reseeder::Reseeder;
use emissary_util::runtime::tokio::Runtime;
use emissary_util::storage::{Storage, StorageBundle};
use tokio::sync::{Notify, watch};
use tokio::task::{AbortHandle, JoinHandle};
use truchman::Bridge;

/// Reseed when fewer routers than this are stored, as emissary-cli 0.4.0 does
/// (`emissary-cli/src/config.rs` 329-330).
pub const RESEED_THRESHOLD: usize = 25;

/// How long a graceful shutdown may take before the router task is dropped.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Whether the router carries other people's tunnels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transit {
    /// No transit tunnels: the router builds only its own.
    Off,
    /// Accept up to `max_tunnels` transit tunnels.
    On {
        /// The cap on concurrent transit tunnels.
        max_tunnels: usize,
    },
}

/// Where a first start, or a start with too few stored routers, gets router infos from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reseed {
    /// emissary-util's built-in HTTPS reseed servers.
    Builtin,
    /// These HTTPS reseed servers instead.
    Hosts(Vec<String>),
    /// Never reseed; start only from what storage holds.
    Never,
}

/// How the embedded router runs. Transit and publishing are required arguments: they are
/// owner decisions, not inherited defaults.
#[derive(Clone, Debug)]
pub struct RouterConfig {
    /// Where the router keeps its identity, transport keys, discovered routers and peer
    /// profiles. Keeping it between runs keeps the router ID stable and skips reseeding.
    pub storage: PathBuf,
    /// Transit participation.
    pub transit: Transit,
    /// Publish NTCP2 and SSU2 addresses so other routers can connect in. Off, the router
    /// still listens (emissary binds its transports), but advertises no address.
    pub publish: bool,
    /// Reseed source.
    pub reseed: Reseed,
    /// Reseed over IPv4 only, emissary-cli's default.
    pub reseed_ipv4_only: bool,
}

impl RouterConfig {
    /// A router at `storage` with the host's transit and publish choices, reseeding from the
    /// built-in servers over IPv4 when it knows fewer than [`RESEED_THRESHOLD`] routers.
    pub fn new(storage: impl Into<PathBuf>, transit: Transit, publish: bool) -> Self {
        Self {
            storage: storage.into(),
            transit,
            publish,
            reseed: Reseed::Builtin,
            reseed_ipv4_only: true,
        }
    }
}

/// What starting the router found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartReport {
    /// Routers storage held before any reseed.
    pub stored_routers: usize,
    /// Routers a reseed added, if one ran.
    pub reseeded: Option<usize>,
    /// The SAMv3 TCP bridge.
    pub sam: SocketAddr,
}

/// A running embedded router. [`Self::shutdown`] stops it gracefully; dropping it stops the
/// router task at once.
#[derive(Debug)]
pub struct EmbeddedRouter {
    report: StartReport,
    stop: Arc<Notify>,
    router: AbortHandle,
    end: watch::Receiver<Option<RouterEnd>>,
}

/// How the router task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouterEnd {
    /// The router future finished, on its own or after [`EmbeddedRouter::shutdown`].
    Exited,
    /// The router task panicked, with the panic's message when it had one.
    Panicked(String),
    /// The task was aborted, as dropping the handle does.
    Aborted,
}

impl EmbeddedRouter {
    /// Load storage, reseed if needed, and start the router on the current tokio runtime.
    pub async fn start(config: RouterConfig) -> io::Result<Self> {
        let storage = Storage::new::<Runtime>(Some(config.storage.clone()))
            .await
            .map_err(other)?;
        let StorageBundle {
            ntcp2_iv,
            ntcp2_key,
            profiles,
            router_info,
            mut routers,
            signing_key,
            static_key,
            ssu2_intro_key,
            ssu2_static_key,
        } = storage.load().await;
        let stored_routers = routers.len();
        let hosts = match &config.reseed {
            _ if stored_routers >= RESEED_THRESHOLD => None,
            Reseed::Never => None,
            Reseed::Builtin => Some(None),
            Reseed::Hosts(hosts) => Some(Some(hosts.clone())),
        };
        let mut reseeded = None;
        if let Some(hosts) = hosts {
            match Reseeder::reseed::<Runtime>(hosts, config.reseed_ipv4_only).await {
                Ok(infos) => {
                    reseeded = Some(infos.len());
                    for info in infos {
                        let _ = storage
                            .store_router_info(info.name, info.router_info.clone())
                            .await;
                        routers.push(info.router_info);
                    }
                },
                Err(e) if routers.is_empty() => return Err(other(e)),
                Err(_) => {},
            }
        }

        let core = Config {
            ntcp2: Some(Ntcp2Config {
                ipv4: true,
                ipv4_host: None,
                ipv6: false,
                ipv6_host: None,
                iv: ntcp2_iv,
                key: ntcp2_key,
                port: 0,
                publish: config.publish,
                ml_kem: None,
                disable_pq: false,
            }),
            ssu2: Some(Ssu2Config {
                disable_pq: false,
                intro_key: ssu2_intro_key,
                ipv4: true,
                ipv4_host: None,
                ipv4_mtu: None,
                ipv6: false,
                ipv6_host: None,
                ipv6_mtu: None,
                port: 0,
                publish: config.publish,
                static_key: ssu2_static_key,
                ml_kem: None,
            }),
            samv3_config: Some(SamConfig {
                tcp_port: 0,
                udp_port: 0,
                host: "127.0.0.1".into(),
            }),
            transit: match config.transit {
                Transit::Off => None,
                Transit::On { max_tunnels } => Some(TransitConfig {
                    max_tunnels: Some(max_tunnels),
                }),
            },
            routers,
            profiles,
            router_info,
            signing_key: Some(signing_key),
            static_key: Some(static_key),
            ..Config::default()
        };
        let (mut router, _events, local_info) =
            Router::<Runtime>::new(core, None, Some(Arc::new(storage.clone())))
                .await
                .map_err(other)?;
        storage
            .store_local_router_info(local_info)
            .await
            .map_err(other)?;
        let sam = router
            .protocol_address_info()
            .sam_tcp
            .ok_or_else(|| io::Error::other("router started without a SAMv3 listener"))?;

        let stop = Arc::new(Notify::new());
        let stopped = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            tokio::select! {
                () = &mut router => return,
                () = stopped.notified() => router.shutdown(),
            }
            let _ = tokio::time::timeout(SHUTDOWN_GRACE, router).await;
        });
        let router = task.abort_handle();
        let end = watch_end(task);
        Ok(Self {
            report: StartReport {
                stored_routers,
                reseeded,
                sam,
            },
            stop,
            router,
            end,
        })
    }

    /// Whether the router is still running.
    pub fn is_running(&self) -> bool {
        self.end.borrow().is_none()
    }

    /// Wait until the router ends, and say how.
    pub async fn stopped(&self) -> RouterEnd {
        let mut end = self.end.clone();
        match end.wait_for(Option::is_some).await {
            Ok(end) => end.clone().unwrap_or(RouterEnd::Aborted),
            Err(_) => RouterEnd::Aborted,
        }
    }

    /// The SAMv3 bridge to hand to [`truchman`] and retinue.
    pub fn bridge(&self) -> Bridge {
        Bridge::new(self.report.sam)
    }

    /// What starting found.
    pub fn report(&self) -> &StartReport {
        &self.report
    }

    /// Shut the router down gracefully, waiting up to ten seconds, and say how it ended.
    pub async fn shutdown(self) -> RouterEnd {
        self.stop.notify_one();
        self.stopped().await
    }
}

impl Drop for EmbeddedRouter {
    fn drop(&mut self) {
        self.router.abort();
    }
}

/// Watch `task` from a second task, so a panic in it is reported rather than leaving callers
/// to wait out their own timeouts.
fn watch_end(task: JoinHandle<()>) -> watch::Receiver<Option<RouterEnd>> {
    let (tx, end) = watch::channel(None);
    tokio::spawn(async move {
        let ended = match task.await {
            Ok(()) => RouterEnd::Exited,
            Err(e) if e.is_panic() => RouterEnd::Panicked(panic_message(e.into_panic())),
            Err(_) => RouterEnd::Aborted,
        };
        let _ = tx.send(Some(ended));
    });
    end
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(s) => *s,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map_or_else(String::new, |s| (*s).to_string()),
    }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ended(task: JoinHandle<()>) -> RouterEnd {
        let mut end = watch_end(task);
        end.wait_for(Option::is_some)
            .await
            .unwrap()
            .clone()
            .unwrap()
    }

    #[tokio::test]
    async fn the_watcher_reports_a_panic_an_exit_and_an_abort() {
        assert_eq!(
            ended(tokio::spawn(async { panic!("garlic tag") })).await,
            RouterEnd::Panicked("garlic tag".into())
        );
        let formatted = tokio::spawn(async { panic!("tag {}", 915) });
        assert_eq!(
            ended(formatted).await,
            RouterEnd::Panicked("tag 915".into())
        );
        assert_eq!(ended(tokio::spawn(async {})).await, RouterEnd::Exited);
        let pending = tokio::spawn(std::future::pending::<()>());
        pending.abort();
        assert_eq!(ended(pending).await, RouterEnd::Aborted);
    }
}
