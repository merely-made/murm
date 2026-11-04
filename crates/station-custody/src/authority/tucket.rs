// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! One scoped Tucket release with a connection-lifetime custody watch.
use super::*;
use castellan_client::custody_client::CustodyClient;
use dramatis::view::VaultLockView;
use personae::{IdentityError, IdentityProvider, tucket::TucketStationMaterial};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Material for one connection. Its lease closes permanently on a lock,
/// missing resident, changed wallet or broken watch. Reconnect asks again.
pub struct ReleasedTucketStation {
    /// Transient, redacted device/realm material.
    pub material: TucketStationMaterial,
    /// Custody lifetime guard. Drop after the protocol runtime has stopped.
    pub lease: TucketCustodyLease,
}

/// Public authority state; contains no seed or released key.
pub struct StationCustodyLease {
    authority: DjinnStationAuthority,
    wallet: [u8; 32],
    closed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
/// The original Tucket spelling of the shared station custody watch.
pub type TucketCustodyLease = StationCustodyLease;

fn locked() -> CustodyClientError {
    CustodyRefusal::Locked.into()
}

impl DjinnStationAuthority {
    /// Public wallet root while custody is unlocked. No material is released.
    pub fn tucket_wallet(&self) -> Result<[u8; 32], CustodyClientError> {
        self.unlocked_station_wallet()
    }
    pub(super) fn unlocked_station_wallet(&self) -> Result<[u8; 32], CustodyClientError> {
        let status = self.status()?;
        if status.lock != VaultLockView::Unlocked {
            return Err(locked());
        }
        status
            .wallet_public_key
            .ok_or_else(|| CustodyRefusal::NoWallet.into())
    }
    /// Release exactly one Tucket scope, verify it, and begin custody watching
    /// before handing material to the application. No wallet is opened here.
    pub fn open_tucket(
        &self,
        device: DeviceId,
        realm: &str,
    ) -> Result<ReleasedTucketStation, CustodyClientError> {
        let wallet = self.tucket_wallet()?;
        let lease = TucketCustodyLease::watch(self.clone(), wallet)?;
        let released = self.release_tucket(device, realm)?;
        let salt = personae::tucket::station_salt(device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        if released.master_public_key().to_bytes() != wallet || !released.holds(&[salt]) {
            return Err(CustodyClientError::Release(
                IdentityError::DerivationFailed("Tucket release scope or wallet changed".into()),
            ));
        }
        let material = TucketStationMaterial::derive(&released, device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        lease.check()?;
        Ok(ReleasedTucketStation { material, lease })
    }
}

impl StationCustodyLease {
    /// Cheap live guard between I/O steps. Pair with `check` at the start of
    /// each driver turn; the independent lock watch closes it asynchronously.
    pub fn check_live(&self) -> Result<(), CustodyClientError> {
        if self.closed.load(Ordering::SeqCst) {
            Err(locked())
        } else {
            Ok(())
        }
    }
    pub(super) fn watch(
        authority: DjinnStationAuthority,
        wallet: [u8; 32],
    ) -> Result<Self, CustodyClientError> {
        let closed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let actor_closed = closed.clone();
        let actor_stop = stop.clone();
        let endpoint = authority.endpoint.clone();
        let app = authority.app.clone();
        let (ready, receive) = std::sync::mpsc::sync_channel(1);
        let join = std::thread::Builder::new().name("signalman-tucket-custody".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(_) => { actor_closed.store(true, Ordering::SeqCst); return; }
            };
            runtime.block_on(async {
                tokio::select! {
                    _ = async { while !actor_stop.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(25)).await; } } => {},
                    _ = async {
                        match CustodyClient::open_declared_at(&endpoint, app, super::declaration()).await {
                            Ok(mut client) => {
                                let _ = ready.send(Ok(()));
                                let _ = client.watch_lock(VaultLockView::Unlocked).await;
                            },
                            Err(error) => { let _ = ready.send(Err(error)); },
                        }
                    } => {},
                }
            });
            actor_closed.store(true, Ordering::SeqCst);
        }).map_err(|_| locked())?;
        let lease = Self {
            authority,
            wallet,
            closed,
            stop,
            join: Some(join),
        };
        receive
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| locked())??;
        Ok(lease)
    }

    /// Check immediately before serial/protocol work. Failure permanently
    /// closes this lease, even if Djinn later unlocks or returns.
    pub fn check(&self) -> Result<(), CustodyClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(locked());
        }
        let result = self.authority.status().and_then(|status| {
            if status.lock == VaultLockView::Unlocked
                && status.wallet_public_key == Some(self.wallet)
                && !self.closed.load(Ordering::SeqCst)
            {
                Ok(())
            } else {
                Err(locked())
            }
        });
        if result.is_err() {
            self.closed.store(true, Ordering::SeqCst);
        }
        result
    }
}

impl Drop for StationCustodyLease {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
