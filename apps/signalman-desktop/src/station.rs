// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The live station actor.
//!
//! One ordinary worker thread owns a small Tokio runtime and the lease-gated
//! `station_custody::SitedStation`. It polls the port's read-only management
//! snapshot getter and hands owned Postilion snapshots to the UI thread, which
//! projects them under the owner's stale policy. The actor never touches
//! Retinue routing state directly, renders nothing, and holds no policy: a
//! closed lease or failed station simply becomes a terminal event.
//!
//! Djinn owns the wallet and signs grants. The application retains only
//! station-scoped keys and a public device id under its local data root.
//! Reopening asks Djinn for the same scope; absent or Locked custody stops
//! before local state is created or a radio is opened.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use station_custody::{
    DjinnStationAuthority, SitedStation, SitedStationCredential, SitedStationError,
};
use pandect::DeviceId;
use postilion::management::ManagementSnapshot;

use crate::network::LayoutWake;

mod messages;
pub use messages::StationSender;

#[cfg(test)]
pub(crate) fn test_channel() -> (
    StationSender,
    tokio::sync::mpsc::Receiver<signalman::message::Message>,
) {
    messages::channel()
}

/// How often the actor captures a snapshot while the station is live.
const POLL: Duration = Duration::from_secs(2);
/// Each Djinn-issued host grant covers this window.
const GRANT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// A replacement grant is issued when less than this much window remains, so
/// renewal always lands before the lease deadline.
const RENEW_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Everything the actor needs to open one attended local station.
#[derive(Clone, Debug)]
pub struct StationSettings {
    /// Application-local public device-id record home.
    pub data_root: PathBuf,
    /// The serial port carrying a running Retinue board.
    pub port: String,
    /// The announced station name.
    pub name: String,
}

/// Read the live-station activation from the environment.
///
/// The live path stays off unless `SIGNALMAN_STATION_PORT` names a port. This
/// mirrors the other runtime-shaping variables (`SIGNALMAN_SERIAL_PORTS`,
/// `SIGNALMAN_MESSAGE_STORE`): bench activation without inventing UI policy
/// the management plan has not gated yet.
pub fn settings_from_env() -> Option<StationSettings> {
    let port = std::env::var("SIGNALMAN_STATION_PORT").ok()?;
    if port.trim().is_empty() {
        return None;
    }
    let data_root = std::env::var_os("SIGNALMAN_STATION_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let root = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            root.join("Merely").join("Signalman").join("station")
        });
    let name = std::env::var("SIGNALMAN_STATION_NAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Signalman station".to_owned());
    Some(StationSettings {
        data_root,
        port,
        name,
    })
}

/// What the actor observed, in order. Snapshots are facts for projection;
/// the other variants are presentation status only.
pub enum StationEvent {
    /// The lease-gated station opened.
    Connected {
        name: String,
        port: String,
        expires_at_ms: u64,
        local: signalman::message::MessagePeer,
    },
    /// One owned management capture and its wall-clock capture time.
    Snapshot {
        snapshot: Box<ManagementSnapshot>,
        captured_unix_ms: u64,
    },
    Message {
        event: signalman::message::MessageEvent,
        complete: bool,
    },
    Peer {
        destination: signalman::message::MessagePeer,
    },
    Notice {
        message: String,
    },
    /// The actor stopped and will not produce further snapshots.
    Failed {
        message: String,
    },
}

/// One live station owned by one ordinary worker thread.
pub struct StationWorker {
    events: Receiver<StationEvent>,
    sender: StationSender,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl StationWorker {
    pub fn spawn(settings: StationSettings, wake: LayoutWake) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (sender, commands) = messages::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let actor_stop = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("signalman-station".to_owned())
            .spawn(move || run_station(settings, events_tx, wake, actor_stop, commands))
            .expect("spawn Signalman station actor");
        Self {
            events,
            sender,
            stop,
            join: Some(join),
        }
    }

    pub fn sender(&self) -> StationSender {
        self.sender.clone()
    }

    /// Take every event observed since the last drain, oldest first.
    pub fn drain(&self) -> Vec<StationEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for StationWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_station(
    settings: StationSettings,
    events: Sender<StationEvent>,
    wake: LayoutWake,
    stop: Arc<AtomicBool>,
    mut commands: tokio::sync::mpsc::Receiver<signalman::message::Message>,
) {
    let fail = |message: String| {
        let _ = events.send(StationEvent::Failed { message });
        wake();
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return fail(format!("could not start the station runtime: {error}"));
        }
    };
    runtime.block_on(async {
        let opened = tokio::select! {
            opened = open_station(&settings) => opened,
            _ = messages::stopped(&stop) => return,
        };
        let station = match opened {
            Ok(station) => station,
            Err(message) => return fail(message),
        };
        let initial = match station.station.management_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return fail(format!("station identity capture failed: {error}")),
        };
        let local = signalman::message::MessagePeer::new(*initial.station.delivery_destination.as_bytes(), Some(*initial.station.identity.ed25519_bytes()));
        let _ = events.send(StationEvent::Connected {
            name: settings.name.clone(),
            port: settings.port.clone(),
            expires_at_ms: station.station.lease().expires_at_ms(),
            local,
        });
        wake();

        let mut last_generation = None;
        let mut poll = tokio::time::interval(POLL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = messages::stopped(&stop) => break,
                command = commands.recv() => match command {
                    Some(message) => {
                        if let Err(error) = station.renew_if_needed() {
                            fail(format!("station grant renewal failed: {error}")); break;
                        }
                        messages::send(&station.station, message, local, &events, &wake, &stop).await;
                    },
                    None => break,
                },
                _ = poll.tick() => {
                    if let Err(error) = station.renew_if_needed() {
                        fail(format!("station grant renewal failed: {error}")); break;
                    }
                    match station.station.management_snapshot().await {
                        Ok(snapshot) => {
                            if last_generation != Some(snapshot.generation) {
                                last_generation = Some(snapshot.generation);
                                let captured_unix_ms = match unix_time_ms() {
                                    Ok(now) => now, Err(error) => { fail(format!("station clock failed: {error}")); break; }
                                };
                                let _ = events.send(StationEvent::Snapshot { snapshot: Box::new(snapshot), captured_unix_ms }); wake();
                            }
                        },
                        Err(SitedStationError::OperationInterrupted) => {},
                        Err(error) => { fail(format!("station snapshot failed: {error}")); break; }
                    }
                },
                received = station.station.next_event() => match received {
                    Ok(event) => {
                        let observed = match unix_time_ms() { Ok(now) => now, Err(error) => { fail(format!("station clock failed: {error}")); break; } };
                        let update = match signalman::message::observe_station_event(&event, local, observed) {
                            Ok(signalman::message::MessageObservation::Incoming(event)) => StationEvent::Message { event, complete: false },
                            Ok(signalman::message::MessageObservation::PeerAppeared { destination, .. }) => StationEvent::Peer { destination },
                            Ok(signalman::message::MessageObservation::Dropped(message)) => StationEvent::Notice { message: format!("Radio receive refused: {message}") },
                            Err(error) => StationEvent::Notice { message: format!("Radio message profile refused: {error}") },
                        };
                        let _ = events.send(update); wake();
                    },
                    Err(SitedStationError::OperationInterrupted) => {},
                    Err(error) => { fail(format!("station receive failed: {error}")); break; }
                },
            }
        }
        station.station.stop().await;
    });
}

struct OpenedStation {
    station: SitedStation,
    credential: SitedStationCredential,
    device_id: DeviceId,
    authority: DjinnStationAuthority,
    wallet_public_key: [u8; 32],
    label: String,
}

impl OpenedStation {
    /// Recheck custody and ask Djinn to renew before the lease deadline.
    ///
    /// A Locked or absent custodian stops this actor before its next capture.
    /// The lease independently checks public grant records on every operation.
    fn renew_if_needed(&self) -> Result<(), String> {
        let status = self
            .authority
            .status()
            .map_err(|error| format!("station identity is pending: {error}"))?;
        if status.wallet_public_key != Some(self.wallet_public_key) {
            return Err("station identity is pending: the wallet is Locked or changed".into());
        }
        let now_ms = unix_time_ms().map_err(|error| format!("station clock failed: {error}"))?;
        let expires_at_ms = self.station.lease().expires_at_ms();
        if expires_at_ms.saturating_sub(now_ms) >= RENEW_MARGIN.as_millis() as u64 {
            return Ok(());
        }
        self.credential
            .issue_remote_auth_grant(
                &self.authority,
                self.device_id,
                self.label.clone(),
                now_ms,
                now_ms + GRANT_WINDOW.as_millis() as u64,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

async fn open_station(settings: &StationSettings) -> Result<OpenedStation, String> {
    open_station_with_authority(settings, DjinnStationAuthority::default()).await
}

async fn open_station_with_authority(
    settings: &StationSettings,
    authority: DjinnStationAuthority,
) -> Result<OpenedStation, String> {
    // Request custody before touching local state or the radio. The public
    // record root comes from Djinn; the desktop never opens a local wallet.
    let status = authority
        .status()
        .map_err(|error| format!("station identity is pending: {error}"))?;
    let wallet_public_key = status
        .wallet_public_key
        .ok_or("station identity is pending: the wallet is Locked or absent")?;
    let wallet_root = status
        .wallet_root
        .ok_or("station identity is pending: Djinn has no wallet record root")?;
    refuse_unadopted_wallet(&settings.data_root, &wallet_root)?;
    // Read an existing public scope first. A fresh scope is saved only after
    // Djinn releases keys and signs its grant.
    let device_id = read_device_id(&settings.data_root)?.unwrap_or_else(DeviceId::new);
    let released = authority
        .release_station(device_id)
        .map_err(|error| format!("station identity is pending: {error}"))?;
    let credential = SitedStationCredential::derive_for_device(&released, device_id)
        .map_err(|error| format!("station credential derivation failed: {error}"))?;
    let now_ms = unix_time_ms().map_err(|error| format!("station clock failed: {error}"))?;
    credential
        .issue_remote_auth_grant(
            &authority,
            device_id,
            settings.name.clone(),
            now_ms,
            now_ms + GRANT_WINDOW.as_millis() as u64,
        )
        .map_err(|error| format!("station grant was refused: {error}"))?;
    persist_device_id(&settings.data_root, device_id)?;
    let station = credential
        .open_station(
            &wallet_root,
            device_id,
            settings.port.clone(),
            settings.name.clone(),
            authority.clone().shared(),
        )
        .await
        .map_err(|error| format!("station did not open on {}: {error}", settings.port))?;
    Ok(OpenedStation {
        station,
        credential,
        device_id,
        authority,
        wallet_public_key,
        label: settings.name.clone(),
    })
}

/// A former application wallet must be adopted by the custodian first.
/// Inspect only the public manifest's existence, never its seed or sealed data.
fn refuse_unadopted_wallet(
    local_root: &std::path::Path,
    wallet_root: &std::path::Path,
) -> Result<(), String> {
    if local_root.join("identity/wallet.json").exists() && local_root != wallet_root {
        return Err(
            "the existing station wallet needs adoption by Djinn before this station can reopen"
                .into(),
        );
    }
    Ok(())
}

/// Read public host bookkeeping without minting identity or writing state.
fn read_device_id(data_root: &std::path::Path) -> Result<Option<DeviceId>, String> {
    let path = data_root.join("station-device-id.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| format!("station device-id record was not readable: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "station device-id record was not readable: {error}"
        )),
    }
}

fn persist_device_id(data_root: &std::path::Path, device_id: DeviceId) -> Result<(), String> {
    std::fs::create_dir_all(data_root)
        .map_err(|error| format!("station data root was not usable: {error}"))?;
    let text = serde_json::to_string(&device_id)
        .map_err(|error| format!("station device-id record was not writable: {error}"))?;
    std::fs::write(data_root.join("station-device-id.json"), text)
        .map_err(|error| format!("station device-id record was not writable: {error}"))
}

fn unix_time_ms() -> Result<u64, std::time::SystemTimeError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(u64::try_from(duration.as_millis()).expect("unix millisecond count fits u64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_wallet_is_not_silently_replaced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("identity")).unwrap();
        let manifest = root.path().join("identity/wallet.json");
        std::fs::write(&manifest, b"opaque legacy manifest").unwrap();
        assert!(refuse_unadopted_wallet(root.path(), &root.path().join("other")).is_err());
        assert!(refuse_unadopted_wallet(root.path(), root.path()).is_ok());
        assert_eq!(std::fs::read(manifest).unwrap(), b"opaque legacy manifest");
    }

    #[test]
    fn corrupt_public_scope_is_refused_instead_of_reminted() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("station-device-id.json"), b"broken").unwrap();
        assert!(read_device_id(root.path()).is_err());
    }

    #[test]
    fn a_durable_device_id_survives_reload() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(read_device_id(root.path()).unwrap(), None);
        let first = DeviceId::new();
        persist_device_id(root.path(), first).unwrap();
        assert_eq!(read_device_id(root.path()).unwrap(), Some(first));
    }
}

#[cfg(test)]
mod custody_receipts {
    use super::*;

    #[test]
    #[ignore = "run with the isolated DR-C absent/Locked keeper"]
    fn desktop_station_pending_leaves_local_state_untouched() {
        let mode = std::env::var("DR_C_RECEIPT_MODE").expect("receipt mode");
        assert!(matches!(mode.as_str(), "absent" | "locked"));
        let root = tempfile::tempdir().unwrap();
        let settings = StationSettings {
            data_root: root.path().join("uncreated"),
            port: "receipt-must-not-open".into(),
            name: "custody receipt".into(),
        };
        let authority =
            DjinnStationAuthority::at(std::env::var("GRAPHSHELL_APP_ENDPOINT").unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(open_station_with_authority(&settings, authority));
        let error = match result {
            Ok(_) => panic!("desktop must not open a fallback station"),
            Err(error) => error,
        };
        assert!(error.contains("identity is pending"), "{error}");
        assert!(
            !settings.data_root.exists(),
            "pending identity must not create local state"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
