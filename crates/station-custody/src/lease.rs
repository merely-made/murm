// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Live host-side enforcement for one sited-station grant.
//!
//! A station lease is deliberately local to the station-custody adapter. Retinue receives
//! only the typed station identity; it never reads a wallet, clock, or grant.
//! The port reloads the current host grant on every lease check. A replacement
//! may extend the deadline before it passes, while expiry, revocation, a stale
//! roster, or a bad signature permanently closes this running lease.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sporran::station_grant::{SitedStationGrant, SitedStationGrantError};
use personae::carry::{DeviceId, RemoteAuthRevocationOutcome};

use crate::authority::StationAuthority;

/// Host-side authorization state for one running station.
#[derive(Clone)]
pub struct SitedStationLease {
    data_root: PathBuf,
    device_id: DeviceId,
    station_ed25519_public_key: [u8; 32],
    state: Arc<Mutex<LeaseState>>,
    /// Who revokes: the wallet's custodian, djinn (D17).
    authority: Arc<dyn StationAuthority>,
}

impl fmt::Debug for SitedStationLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SitedStationLease")
            .field("data_root", &self.data_root)
            .field("device_id", &self.device_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct LeaseState {
    closed: bool,
    expires_at_ms: u64,
}

impl SitedStationLease {
    /// Acquire a live lease from the active host-side station grant.
    pub fn acquire(
        data_root: impl AsRef<Path>,
        device_id: DeviceId,
        station_ed25519_public_key: [u8; 32],
        now_ms: u64,
        authority: Arc<dyn StationAuthority>,
    ) -> Result<Self, SitedStationLeaseError> {
        let data_root = data_root.as_ref().to_path_buf();
        let grant = SitedStationGrant::load_active(
            &data_root,
            device_id,
            station_ed25519_public_key,
            now_ms,
        )?;
        Ok(Self {
            data_root,
            device_id,
            station_ed25519_public_key,
            state: Arc::new(Mutex::new(LeaseState {
                closed: false,
                expires_at_ms: grant.expires_at_ms(),
            })),
            authority,
        })
    }

    /// Recheck the host grant and return its active expiry deadline.
    ///
    /// A longer replacement grant may extend the deadline only when this check
    /// lands before the current deadline. Any failed check permanently closes
    /// this lease. Opening a new station requires a new lease after an expiry
    /// or revocation.
    pub fn authorize_at(&self, now_ms: u64) -> Result<u64, SitedStationLeaseError> {
        let current_expiry = self.accepted_expiry_at(now_ms)?;

        match SitedStationGrant::load_active(
            &self.data_root,
            self.device_id,
            self.station_ed25519_public_key,
            now_ms,
        ) {
            Ok(grant) => {
                let mut state = self.state.lock().expect("station lease mutex poisoned");
                if state.closed {
                    return Err(SitedStationLeaseError::Closed {
                        device_id: self.device_id,
                    });
                }
                if now_ms >= current_expiry {
                    state.closed = true;
                    return Err(SitedStationLeaseError::DeadlineElapsed {
                        device_id: self.device_id,
                        expires_at_ms: current_expiry,
                        now_ms,
                    });
                }
                state.expires_at_ms = grant.expires_at_ms();
                Ok(state.expires_at_ms)
            },
            Err(error) => {
                self.close();
                Err(error.into())
            },
        }
    }

    /// Check the last deadline accepted by this lease without reading the host
    /// store again.
    ///
    /// Call this after an I/O-backed authorization check and immediately before
    /// opening or using Retinue. It prevents a slow host-store read from
    /// admitting an operation after the accepted deadline.
    pub(crate) fn accepted_expiry_at(&self, now_ms: u64) -> Result<u64, SitedStationLeaseError> {
        let mut state = self.state.lock().expect("station lease mutex poisoned");
        if state.closed {
            return Err(SitedStationLeaseError::Closed {
                device_id: self.device_id,
            });
        }
        if now_ms >= state.expires_at_ms {
            state.closed = true;
            return Err(SitedStationLeaseError::DeadlineElapsed {
                device_id: self.device_id,
                expires_at_ms: state.expires_at_ms,
                now_ms,
            });
        }
        Ok(state.expires_at_ms)
    }

    /// Mark the lease closed and refuse every future authorization.
    pub fn close(&self) {
        self.state
            .lock()
            .expect("station lease mutex poisoned")
            .closed = true;
    }

    /// Revoke this device in the host wallet, through djinn, and close the
    /// lease immediately.
    ///
    /// This is the local host action. Propagating the revocation to a remote
    /// unattended station remains a carrier concern, but the host process does
    /// not wait for that carrier before dropping its own authority.
    pub fn revoke(&self) -> Result<RemoteAuthRevocationOutcome, SitedStationLeaseError> {
        let outcome = self
            .authority
            .revoke_device(self.device_id)
            .map_err(SitedStationLeaseError::Storage)?;
        self.close();
        Ok(outcome)
    }

    /// Whether this lease is permanently closed.
    pub fn is_closed(&self) -> bool {
        self.state
            .lock()
            .expect("station lease mutex poisoned")
            .closed
    }

    /// The latest active deadline accepted by this lease.
    pub fn expires_at_ms(&self) -> u64 {
        self.state
            .lock()
            .expect("station lease mutex poisoned")
            .expires_at_ms
    }

    /// The device this lease authorizes.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }
}

/// Why a running station can no longer be authorized.
#[derive(Debug)]
pub enum SitedStationLeaseError {
    /// The host wallet could not carry out a lease operation.
    Storage(std::io::Error),
    /// The host grant did not authorize the station.
    Grant(SitedStationGrantError),
    /// The lease had already failed closed.
    Closed {
        /// The station device that must be reopened under a new lease.
        device_id: DeviceId,
    },
    /// The previous lease deadline elapsed before a renewal was accepted.
    DeadlineElapsed {
        /// The station device that stopped.
        device_id: DeviceId,
        /// The last expiry deadline the lease accepted.
        expires_at_ms: u64,
        /// The instant where the lease was checked.
        now_ms: u64,
    },
}

impl From<SitedStationGrantError> for SitedStationLeaseError {
    fn from(error: SitedStationGrantError) -> Self {
        Self::Grant(error)
    }
}

impl fmt::Display for SitedStationLeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "sited station lease storage failed: {error}"),
            Self::Grant(error) => write!(f, "sited station lease refused: {error}"),
            Self::Closed { device_id } => write!(
                f,
                "sited station lease for {} is closed; reopen under a new active grant",
                device_id.as_uuid()
            ),
            Self::DeadlineElapsed {
                device_id,
                expires_at_ms,
                now_ms,
            } => write!(
                f,
                "sited station lease for {} expired at {expires_at_ms}; checked at {now_ms}",
                device_id.as_uuid()
            ),
        }
    }
}

impl std::error::Error for SitedStationLeaseError {}
