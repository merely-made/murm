// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Scoped Sennet station identity/material, using the same sticky lock/loss
//! watch as Tucket. This route does not provision channel-encryption keys.
use super::*;
use personae::{
    IdentityError, IdentityProvider,
    sennet::{SennetStationIdentity, SennetStationMaterial},
};

/// One public node number and its connection-lifetime custody guard.
pub struct ReleasedSennetStation {
    /// Scoped public number; it does not authenticate RF senders.
    pub identity: SennetStationIdentity,
    /// Transient device/realm child for immediate protocol construction only.
    pub material: SennetStationMaterial,
    /// Retain until serial/protocol work has stopped.
    pub lease: StationCustodyLease,
}
impl DjinnStationAuthority {
    /// Public wallet root while unlocked, without releasing station material.
    pub fn sennet_wallet(&self) -> Result<[u8; 32], CustodyClientError> {
        self.unlocked_station_wallet()
    }
    /// Request exactly one device/realm child; the wallet root stays in Djinn.
    pub fn release_sennet(
        &self,
        device: DeviceId,
        realm: &str,
    ) -> Result<RetainedKeys, CustodyClientError> {
        let salt = personae::sennet::station_salt(device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        self.with_client(|client| client.release(KeySource::Wallet, vec![salt]))
    }
    /// Verify a scoped release and start watching custody before returning a
    /// public node number. A lock/loss permanently closes this connection.
    pub fn open_sennet(
        &self,
        device: DeviceId,
        realm: &str,
    ) -> Result<ReleasedSennetStation, CustodyClientError> {
        let wallet = self.sennet_wallet()?;
        let lease = StationCustodyLease::watch(self.clone(), wallet)?;
        let released = self.release_sennet(device, realm)?;
        let salt = personae::sennet::station_salt(device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        if released.master_public_key().to_bytes() != wallet || !released.holds(&[salt]) {
            return Err(CustodyClientError::Release(
                IdentityError::DerivationFailed("Sennet release scope or wallet changed".into()),
            ));
        }
        let identity = SennetStationIdentity::derive(&released, device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        let material = SennetStationMaterial::derive(&released, device.as_uuid().as_bytes(), realm)
            .map_err(CustodyClientError::Release)?;
        lease.check()?;
        Ok(ReleasedSennetStation {
            identity,
            material,
            lease,
        })
    }
}
