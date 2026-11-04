// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The unattended sited-station host.
//!
//! This is the place where a remote station's unavoidable delegated secret is
//! held. It seals that Reticulum identity and the control receiver snapshot at
//! a caller-selected relative record path. Retinue sees only the transient
//! typed identity supplied to `Station::open`; it never reads this state.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use outrider::LxmfPayload;
use personae::carry::DeviceId;
use personae::{IdentityError, SealedRecordStorage};
use postilion::management::ManagementSnapshot;
use postilion::{Event, Sent, Station, StationConfig};
use retinue::hash::AddressHash;
use retinue::identity::{Identity, PrivateIdentity};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, watch};
use zeroize::{Zeroize, Zeroizing};

use crate::{SitedStationControlAck, SitedStationControlError, SitedStationControlReceiver};

const HEAD_RECORD_SCHEMA_VERSION: u16 = 1;

/// Sealed, non-exporting owner of an unattended station's delegated identity.
///
/// Provision it only after the device's secure-storage root is available. The
/// record path is caller-selected but must be relative, as enforced by
/// [`SealedRecordStorage`]. A reboot restores the same station address and its
/// last accepted control state.
#[derive(Clone)]
pub struct SitedStationHead {
    station_identity: PrivateIdentity,
    core: Arc<SitedStationHeadCore>,
}

impl fmt::Debug for SitedStationHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SitedStationHead")
            .field("device_id", &self.device_id())
            .field("station", self.public_identity())
            .finish_non_exhaustive()
    }
}

struct SitedStationHeadCore {
    receiver: Mutex<SitedStationControlReceiver>,
    persistence: HeadPersistence,
    changed: watch::Sender<()>,
}

#[derive(Clone)]
struct HeadPersistence {
    storage: SealedRecordStorage,
    record_path: PathBuf,
    device_id: DeviceId,
    station_secret: Zeroizing<Vec<u8>>,
}

impl SitedStationHead {
    /// Seal a freshly provisioned delegated station identity and an empty
    /// control receiver before opening any radio port.
    pub(crate) fn provision(
        storage: SealedRecordStorage,
        record_path: impl AsRef<Path>,
        device_id: DeviceId,
        station_identity: PrivateIdentity,
    ) -> Result<Self, SitedStationHeadError> {
        let receiver =
            SitedStationControlReceiver::new(device_id, *station_identity.public().ed25519_bytes());
        let head = Self::from_parts(storage, record_path.as_ref(), station_identity, receiver)?;
        head.persist()?;
        Ok(head)
    }

    /// Restore an existing unattended station from its sealed record.
    pub fn restore(
        storage: SealedRecordStorage,
        record_path: impl AsRef<Path>,
    ) -> Result<Self, SitedStationHeadError> {
        let record_path = record_path.as_ref();
        let mut record: SitedStationHeadRecord =
            storage.load_record(record_path)?.ok_or_else(|| {
                SitedStationHeadError::MissingState {
                    record_path: record_path.to_path_buf(),
                }
            })?;
        if record.schema_version != HEAD_RECORD_SCHEMA_VERSION {
            return Err(SitedStationHeadError::UnsupportedStateSchema {
                actual: record.schema_version,
            });
        }
        let mut secret: [u8; 64] = record.station_secret.as_slice().try_into().map_err(|_| {
            SitedStationHeadError::InvalidStationSecretLength {
                actual: record.station_secret.len(),
            }
        })?;
        let station_identity = PrivateIdentity::from_secret_bytes(&secret);
        secret.zeroize();
        let receiver = SitedStationControlReceiver::from_snapshot(
            &record.control_snapshot,
            record.device_id,
            *station_identity.public().ed25519_bytes(),
        )?;
        record.station_secret.zeroize();
        record.control_snapshot.zeroize();
        Self::from_parts(storage, record_path, station_identity, receiver)
    }

    fn from_parts(
        storage: SealedRecordStorage,
        record_path: &Path,
        station_identity: PrivateIdentity,
        receiver: SitedStationControlReceiver,
    ) -> Result<Self, SitedStationHeadError> {
        let device_id = receiver.device_id();
        let mut secret = station_identity.to_secret_bytes();
        let persistence = HeadPersistence {
            storage,
            record_path: record_path.to_path_buf(),
            device_id,
            station_secret: Zeroizing::new(secret.to_vec()),
        };
        secret.zeroize();
        let (changed, _) = watch::channel(());
        Ok(Self {
            station_identity,
            core: Arc::new(SitedStationHeadCore {
                receiver: Mutex::new(receiver),
                persistence,
                changed,
            }),
        })
    }

    /// The durable device id this head controls.
    pub fn device_id(&self) -> DeviceId {
        self.core.persistence.device_id
    }

    /// The public Reticulum station identity that signs its acknowledgements.
    pub fn public_identity(&self) -> &Identity {
        self.station_identity.public()
    }

    /// Accept a titled LXMF delivery, persist any accepted transition, then
    /// return the station-signed acknowledgement.
    ///
    /// `Ok(None)` means an ordinary application message. An accepted grant,
    /// renewal, or revoke is written to sealed storage before its
    /// acknowledgement returns. A late replacement that closes the receiver is
    /// likewise written before its error returns. The running station's
    /// watchdog is then woken so revocation and expiry stop it without waiting
    /// for its old deadline.
    pub fn receive_delivery(
        &self,
        title: &[u8],
        body: &[u8],
        now_ms: u64,
    ) -> Result<Option<SitedStationControlAck>, SitedStationHeadError> {
        let acknowledgement = self.transition(|receiver| {
            receiver.receive_delivery(title, body, now_ms, &self.station_identity)
        });
        if acknowledgement.as_ref().is_ok_and(Option::is_some) || self.is_closed() {
            self.core.changed.send_replace(());
        }
        acknowledgement
    }

    /// Recheck the stored delegated authority at one instant.
    ///
    /// Exact expiry fails closed. The closure itself is durably recorded before
    /// returning, so a reboot cannot revive the station.
    pub fn authorize_at(&self, now_ms: u64) -> Result<u64, SitedStationHeadError> {
        let result = self.transition(|receiver| receiver.authorize_at(now_ms));
        if result.is_err() {
            self.core.changed.send_replace(());
        }
        result
    }

    /// Whether the retained receiver has permanently stopped this head.
    pub fn is_closed(&self) -> bool {
        self.core
            .receiver
            .lock()
            .expect("sited station head mutex poisoned")
            .is_closed()
    }

    /// Open the serial radio using the sealed delegated identity.
    ///
    /// The returned running station keeps that identity private and applies the
    /// persisted receiver before every public radio operation.
    pub async fn open_station(
        &self,
        port: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<RunningSitedStationHead, SitedStationHeadError> {
        let now_ms = unix_time_ms()?;
        self.authorize_at(now_ms)?;
        let station = Station::open(StationConfig::new(
            port,
            name,
            self.station_identity.clone(),
        ))
        .await
        .map_err(SitedStationHeadError::Station)?;
        self.authorize_at(unix_time_ms()?)?;
        Ok(RunningSitedStationHead::new(self.clone(), station))
    }

    fn transition<T>(
        &self,
        transition: impl FnOnce(&mut SitedStationControlReceiver) -> Result<T, SitedStationControlError>,
    ) -> Result<T, SitedStationHeadError> {
        let mut receiver = self
            .core
            .receiver
            .lock()
            .expect("sited station head mutex poisoned");
        let previous = receiver.clone();
        let result = transition(&mut receiver);
        if *receiver != previous
            && let Err(error) = self.core.persistence.save(&receiver)
        {
            *receiver = previous;
            return Err(error);
        }
        result.map_err(Into::into)
    }

    fn persist(&self) -> Result<(), SitedStationHeadError> {
        let receiver = self
            .core
            .receiver
            .lock()
            .expect("sited station head mutex poisoned");
        self.core.persistence.save(&receiver)
    }
}

impl HeadPersistence {
    fn save(&self, receiver: &SitedStationControlReceiver) -> Result<(), SitedStationHeadError> {
        self.storage.save_record(
            &self.record_path,
            &SitedStationHeadRecord {
                schema_version: HEAD_RECORD_SCHEMA_VERSION,
                device_id: self.device_id,
                station_secret: self.station_secret.to_vec(),
                control_snapshot: receiver.snapshot()?,
            },
        )?;
        Ok(())
    }
}

/// A running radio whose identity and authorization are owned by a
/// [`SitedStationHead`].
pub struct RunningSitedStationHead {
    head: SitedStationHead,
    station: Arc<AsyncMutex<Option<Station>>>,
    watchdog: tokio::task::JoinHandle<()>,
}

impl RunningSitedStationHead {
    fn new(head: SitedStationHead, station: Station) -> Self {
        let station = Arc::new(AsyncMutex::new(Some(station)));
        let watchdog = tokio::spawn(watch_head(
            head.clone(),
            Arc::clone(&station),
            head.core.changed.subscribe(),
        ));
        Self {
            head,
            station,
            watchdog,
        }
    }

    /// The durable authority owner for status rendering and control delivery.
    pub fn head(&self) -> &SitedStationHead {
        &self.head
    }

    /// The delivery destination owned by this running station.
    ///
    /// The sealed receiver is rechecked before the address is exposed so an
    /// expired or revoked head cannot keep presenting a live station handle.
    pub async fn address(&self) -> Result<AddressHash, SitedStationHeadError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        Ok(station
            .as_ref()
            .ok_or(SitedStationHeadError::Stopped)?
            .address())
    }

    /// Capture the station's read-only management facts under its sealed
    /// authority. Retinue's mutable routing state stays private.
    pub async fn management_snapshot(&self) -> Result<ManagementSnapshot, SitedStationHeadError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        Ok(station
            .as_ref()
            .ok_or(SitedStationHeadError::Stopped)?
            .management_snapshot())
    }

    /// Announce while the sealed receiver remains live.
    pub async fn announce(&self) -> Result<(), SitedStationHeadError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        station
            .as_ref()
            .ok_or(SitedStationHeadError::Stopped)?
            .announce();
        Ok(())
    }

    /// Send text while the sealed receiver remains live.
    ///
    /// A control transition interrupts an in-flight send. A caller may retry
    /// after a valid renewal; an expiry or revoke drops the radio instead.
    pub async fn send_text(
        &self,
        prefix: &str,
        body: &str,
        patience: Duration,
    ) -> Result<Sent, SitedStationHeadError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let mut changed = self.head.core.changed.subscribe();
        let result = {
            let station = self.station.lock().await;
            let station = station.as_ref().ok_or(SitedStationHeadError::Stopped)?;
            tokio::select! {
                result = station.send_text(prefix, body, patience) => Some(result),
                _ = tokio::time::sleep(remaining_window(now_ms, expires_at_ms)) => None,
                _ = changed.changed() => None,
            }
        };
        match result {
            Some(result) => result.map_err(SitedStationHeadError::Station),
            None => self.stop_after_interruption().await,
        }
    }

    /// Carry one complete LXMF payload while the sealed receiver remains live.
    ///
    /// A renewal, expiry, or revoke interrupts the in-flight send just as it
    /// does for text, while application fields retain their LXMF positions.
    pub async fn send_payload(
        &self,
        prefix: &str,
        payload: &LxmfPayload,
        patience: Duration,
    ) -> Result<Sent, SitedStationHeadError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let mut changed = self.head.core.changed.subscribe();
        let result = {
            let station = self.station.lock().await;
            let station = station.as_ref().ok_or(SitedStationHeadError::Stopped)?;
            tokio::select! {
                result = station.send_payload(prefix, payload, patience) => Some(result),
                _ = tokio::time::sleep(remaining_window(now_ms, expires_at_ms)) => None,
                _ = changed.changed() => None,
            }
        };
        match result {
            Some(result) => result.map_err(SitedStationHeadError::Station),
            None => self.stop_after_interruption().await,
        }
    }

    /// Wait for one Retinue event while the sealed receiver remains live.
    ///
    /// A control transition interrupts the wait so the caller cannot retain an
    /// old receiver state through a renewal, expiry, or revoke.
    pub async fn next_event(&self) -> Result<Event, SitedStationHeadError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let mut changed = self.head.core.changed.subscribe();
        let result = {
            let mut station = self.station.lock().await;
            let station = station.as_mut().ok_or(SitedStationHeadError::Stopped)?;
            tokio::select! {
                result = station.next_event() => Some(result),
                _ = tokio::time::sleep(remaining_window(now_ms, expires_at_ms)) => None,
                _ = changed.changed() => None,
            }
        };
        match result {
            Some(Some(event)) => Ok(event),
            Some(None) => Err(SitedStationHeadError::Stopped),
            None => self.stop_after_interruption().await,
        }
    }

    /// Stop the radio and refuse future activity in this running instance.
    pub async fn stop(&self) {
        self.watchdog.abort();
        self.station.lock().await.take();
    }

    fn authorize_now(&self) -> Result<(u64, u64), SitedStationHeadError> {
        let checked_at_ms = unix_time_ms()?;
        self.head.authorize_at(checked_at_ms)?;
        let usable_at_ms = unix_time_ms()?;
        let expires_at_ms = self.head.authorize_at(usable_at_ms)?;
        Ok((usable_at_ms, expires_at_ms))
    }

    async fn stop_after_interruption<T>(&self) -> Result<T, SitedStationHeadError> {
        match self.head.authorize_at(unix_time_ms()?) {
            Ok(_) => Err(SitedStationHeadError::OperationInterrupted),
            Err(error) => {
                self.station.lock().await.take();
                Err(error)
            },
        }
    }
}

impl Drop for RunningSitedStationHead {
    fn drop(&mut self) {
        self.watchdog.abort();
    }
}

async fn watch_head(
    head: SitedStationHead,
    station: Arc<AsyncMutex<Option<Station>>>,
    mut changed: watch::Receiver<()>,
) {
    loop {
        let now_ms = match unix_time_ms() {
            Ok(now_ms) => now_ms,
            Err(_) => {
                station.lock().await.take();
                return;
            },
        };
        let expires_at_ms = match head.authorize_at(now_ms) {
            Ok(expires_at_ms) => expires_at_ms,
            Err(_) => {
                station.lock().await.take();
                return;
            },
        };
        tokio::select! {
            _ = tokio::time::sleep(remaining_window(now_ms, expires_at_ms)) => {}
            changed_result = changed.changed() => {
                if changed_result.is_err() {
                    station.lock().await.take();
                    return;
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SitedStationHeadRecord {
    schema_version: u16,
    device_id: DeviceId,
    station_secret: Vec<u8>,
    control_snapshot: Vec<u8>,
}

impl Drop for SitedStationHeadRecord {
    fn drop(&mut self) {
        self.station_secret.zeroize();
        self.control_snapshot.zeroize();
    }
}

/// Why an unattended station host could not load, persist, or run.
#[derive(Debug)]
pub enum SitedStationHeadError {
    /// The record encryption or storage adapter failed.
    Storage(IdentityError),
    /// The control receiver refused a frame or authorization check.
    Control(SitedStationControlError),
    /// The sealed record was not provisioned yet.
    MissingState { record_path: PathBuf },
    /// The sealed record uses a newer or incompatible schema.
    UnsupportedStateSchema { actual: u16 },
    /// The sealed station secret did not have Reticulum's exact 64-byte size.
    InvalidStationSecretLength { actual: usize },
    /// The host clock could not be read.
    Clock(std::time::SystemTimeError),
    /// Postilion could not open or use the station.
    Station(postilion::Error),
    /// The running station was already stopped.
    Stopped,
    /// Authority changed while a radio operation was running but remains live.
    OperationInterrupted,
}

impl From<IdentityError> for SitedStationHeadError {
    fn from(error: IdentityError) -> Self {
        Self::Storage(error)
    }
}

impl From<SitedStationControlError> for SitedStationHeadError {
    fn from(error: SitedStationControlError) -> Self {
        Self::Control(error)
    }
}

impl From<std::time::SystemTimeError> for SitedStationHeadError {
    fn from(error: std::time::SystemTimeError) -> Self {
        Self::Clock(error)
    }
}

impl fmt::Display for SitedStationHeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "sited station sealed storage failed: {error}"),
            Self::Control(error) => write!(f, "sited station control failed: {error}"),
            Self::MissingState { record_path } => write!(
                f,
                "sited station state is not provisioned at {}",
                record_path.display()
            ),
            Self::UnsupportedStateSchema { actual } => {
                write!(f, "unsupported sited station state schema {actual}")
            },
            Self::InvalidStationSecretLength { actual } => write!(
                f,
                "sealed sited station secret has {actual} bytes, not Reticulum's 64"
            ),
            Self::Clock(error) => write!(f, "could not read the station clock: {error}"),
            Self::Station(error) => write!(f, "Retinue station failed: {error}"),
            Self::Stopped => f.write_str("sited station is stopped"),
            Self::OperationInterrupted => {
                f.write_str("sited station operation was interrupted by a grant renewal; retry it")
            },
        }
    }
}

impl std::error::Error for SitedStationHeadError {}

fn unix_time_ms() -> Result<u64, std::time::SystemTimeError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(u64::try_from(duration.as_millis()).expect("unix millisecond count fits u64"))
}

fn remaining_window(now_ms: u64, expires_at_ms: u64) -> Duration {
    Duration::from_millis(expires_at_ms.saturating_sub(now_ms))
}

#[cfg(test)]
mod tests {
    use crate::authority::test_support::TestWallet;
    use tempfile::tempdir;

    use super::*;
    use crate::{SITED_STATION_CONTROL_TITLE, SitedStationCredential};

    #[test]
    fn accepted_control_is_sealed_before_its_ack_and_survives_restart() {
        let wallet = tempdir().unwrap();
        let storage_root = tempdir().unwrap();
        let custodian = TestWallet::new(wallet.path(), 0x5a);
        let provider = custodian.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        let grant = credential
            .issue_remote_auth_grant(custodian.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let storage = SealedRecordStorage::open_with_key(storage_root.path(), [0x55; 32]);
        let head = SitedStationHead::provision(
            storage.clone(),
            "station/ridge-north.json",
            device_id,
            credential.identity.clone(),
        )
        .unwrap();

        let control = credential.control_signer().grant(&grant).unwrap();
        let acknowledgement = head
            .receive_delivery(
                SITED_STATION_CONTROL_TITLE,
                &control.to_bytes().unwrap(),
                101,
            )
            .unwrap()
            .expect("control title should produce an acknowledgement");
        assert_eq!(
            acknowledgement
                .verify(credential.public_identity(), &control)
                .unwrap(),
            &crate::SitedStationControlResult::GrantInstalled { expires_at_ms: 200 }
        );

        let restored = SitedStationHead::restore(storage, "station/ridge-north.json").unwrap();
        assert_eq!(restored.public_identity(), credential.public_identity());
        assert_eq!(restored.authorize_at(199).unwrap(), 200);
    }

    #[test]
    fn expiry_is_persisted_so_restart_cannot_restore_authority() {
        let wallet = tempdir().unwrap();
        let storage_root = tempdir().unwrap();
        let custodian = TestWallet::new(wallet.path(), 0x5a);
        let provider = custodian.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        let grant = credential
            .issue_remote_auth_grant(custodian.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let storage = SealedRecordStorage::open_with_key(storage_root.path(), [0x56; 32]);
        let head = SitedStationHead::provision(
            storage.clone(),
            "station/ridge-north.json",
            device_id,
            credential.identity.clone(),
        )
        .unwrap();
        let control = credential.control_signer().grant(&grant).unwrap();
        head.receive_delivery(
            SITED_STATION_CONTROL_TITLE,
            &control.to_bytes().unwrap(),
            101,
        )
        .unwrap();

        assert!(head.authorize_at(200).is_err());
        let restored = SitedStationHead::restore(storage, "station/ridge-north.json").unwrap();
        assert!(restored.is_closed());
        assert!(restored.authorize_at(201).is_err());
    }

    #[test]
    fn revoke_is_persisted_before_its_ack_so_restart_cannot_restore_authority() {
        let wallet = tempdir().unwrap();
        let storage_root = tempdir().unwrap();
        let custodian = TestWallet::new(wallet.path(), 0x5a);
        let provider = custodian.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        let grant = credential
            .issue_remote_auth_grant(custodian.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let storage = SealedRecordStorage::open_with_key(storage_root.path(), [0x58; 32]);
        let head = SitedStationHead::provision(
            storage.clone(),
            "station/ridge-north.json",
            device_id,
            credential.identity.clone(),
        )
        .unwrap();
        let grant_control = credential.control_signer().grant(&grant).unwrap();
        head.receive_delivery(
            SITED_STATION_CONTROL_TITLE,
            &grant_control.to_bytes().unwrap(),
            101,
        )
        .unwrap();

        let revoke = credential.control_signer().revoke(150).unwrap();
        let acknowledgement = head
            .receive_delivery(
                SITED_STATION_CONTROL_TITLE,
                &revoke.to_bytes().unwrap(),
                150,
            )
            .unwrap()
            .expect("revoke should produce an acknowledgement");
        assert_eq!(
            acknowledgement
                .verify(credential.public_identity(), &revoke)
                .unwrap(),
            &crate::SitedStationControlResult::Revoked
        );

        let restored = SitedStationHead::restore(storage, "station/ridge-north.json").unwrap();
        assert!(restored.is_closed());
        assert!(restored.authorize_at(151).is_err());
    }

    #[test]
    fn sealed_record_requires_its_storage_key() {
        let storage_root = tempdir().unwrap();
        let storage = SealedRecordStorage::open_with_key(storage_root.path(), [0x57; 32]);
        let device_id = DeviceId::new();
        let identity = PrivateIdentity::from_secret_bytes(&[0x7c; 64]);
        SitedStationHead::provision(storage, "station/ridge-north.json", device_id, identity)
            .unwrap();

        let text =
            std::fs::read_to_string(storage_root.path().join("station/ridge-north.json")).unwrap();
        assert!(!text.contains("station_secret"));

        let wrong_key = SealedRecordStorage::open_with_key(storage_root.path(), [0x58; 32]);
        assert!(matches!(
            SitedStationHead::restore(wrong_key, "station/ridge-north.json"),
            Err(SitedStationHeadError::Storage(_))
        ));
    }
}
