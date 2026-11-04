#![forbid(unsafe_code)]
// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Persona-backed custody adapter for Djinn-managed Retinue stations.
//!
//! This crate is the only layer here that knows both Castellan/Personae and
//! Retinue. It derives a station-scoped Reticulum identity, binds its signing
//! key to Castellan's narrow expiring RemoteAuth grant, and opens Retinue only
//! behind a lease that stops the running station at expiry. Retinue does not
//! know where the credential came from and never creates a local account file.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use outrider::LxmfPayload;
use sporran::station_grant::{SitedStationGrant, SitedStationGrantError, SitedStationGrantRequest};
use personae::carry::{DeviceId, RemoteAuthRevocationOutcome};
use personae::reticulum::{ReticulumControllerMaterial, ReticulumStationMaterial};
use personae::{IdentityError, IdentityProvider, SealedRecordStorage};
use postilion::control::first_owner::{
    ClaimOutcome, ClaimPlan, FirstOwnerController, FirstOwnerError, FirstOwnerExchange,
};
use postilion::control::verified::{
    ControlClient, ControlClientError, ControlExchange, VerifiedStatus,
};
use postilion::{Event, Sent, Station, StationConfig};
use radio_hand::control::NodeId;
use retinue::hash::AddressHash;
use retinue::identity::{Identity, PrivateIdentity};
use tokio::sync::Mutex;
use zeroize::Zeroize;

mod authority;
mod control;
mod head;
mod lease;

pub use authority::{
    DjinnStationAuthority, ReleasedSennetStation, ReleasedTucketStation, SIGNALMAN_APP, declaration,
    StationAuthority, StationCustodyLease, TucketCustodyLease, controller_salts, station_salts,
};
pub use control::{
    SITED_STATION_CONTROL_ACK_TITLE, SITED_STATION_CONTROL_TITLE, SitedStationControl,
    SitedStationControlAck, SitedStationControlError, SitedStationControlReceiver,
    SitedStationControlResult, SitedStationControlSigner,
};
pub use head::{RunningSitedStationHead, SitedStationHead, SitedStationHeadError};
pub use lease::{SitedStationLease, SitedStationLeaseError};

/// First-owner controller signer, deliberately separate from a station credential.
///
/// The type is private because a caller must be able to request the claim workflow without
/// serializing, replacing, or repurposing the controller identity.
struct FirstOwnerCredential {
    identity: PrivateIdentity,
}

impl FirstOwnerCredential {
    fn derive(
        provider: &dyn IdentityProvider,
        controller_scope: &[u8],
    ) -> Result<Self, IdentityError> {
        let material = ReticulumControllerMaterial::derive(provider, controller_scope)?;
        let mut secret = material.secret_bytes();
        let identity = PrivateIdentity::from_secret_bytes(&secret);
        secret.zeroize();
        Ok(Self { identity })
    }

    async fn claim<C>(
        &self,
        controller: &mut FirstOwnerController<C>,
        plan: ClaimPlan,
    ) -> Result<ClaimOutcome, FirstOwnerError<C::Error>>
    where
        C: FirstOwnerExchange,
    {
        controller.claim(&self.identity, plan).await
    }

    async fn status<C>(
        &self,
        carrier: C,
        node: NodeId,
        counter: u64,
    ) -> Result<VerifiedStatus, ControlClientError<C::Error>>
    where
        C: ControlExchange,
    {
        ControlClient::new(carrier, &self.identity, node)
            .status(counter)
            .await
    }
}

/// A controller credential could not be derived or its signed status exchange failed.
pub enum ControllerStatusError<E> {
    Identity(IdentityError),
    Status(ControlClientError<E>),
}

impl<E: fmt::Debug> fmt::Debug for ControllerStatusError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The identity error is never printed: it could describe key material.
            Self::Identity(_) => f
                .debug_tuple("ControllerStatusError")
                .field(&"identity")
                .finish(),
            Self::Status(error) => f.debug_tuple("ControllerStatusError").field(error).finish(),
        }
    }
}

impl<E: fmt::Debug> fmt::Display for ControllerStatusError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(_) => {
                f.write_str("first-owner controller credential could not be derived")
            },
            Self::Status(error) => write!(f, "signed status exchange failed: {error}"),
        }
    }
}

impl<E> std::error::Error for ControllerStatusError<E> where E: fmt::Debug + 'static {}

/// Ask one claimed board for its control status as the first-owner controller.
///
/// The scope selects the same controller identity that claimed the board, so the board's
/// durable grant verifies the signature. `counter` is this controller's next unused outer
/// counter; the caller persists it before the exchange, because the board may accept and
/// journal a command whose reply is then lost.
pub async fn first_owner_status<C>(
    provider: &dyn IdentityProvider,
    controller_scope: &[u8],
    carrier: C,
    node: NodeId,
    counter: u64,
) -> Result<VerifiedStatus, ControllerStatusError<C::Error>>
where
    C: ControlExchange,
{
    FirstOwnerCredential::derive(provider, controller_scope)
        .map_err(ControllerStatusError::Identity)?
        .status(carrier, node, counter)
        .await
        .map_err(ControllerStatusError::Status)
}

/// A controller credential could not be derived or its bounded claim workflow failed.
pub enum FirstOwnerCredentialError<E> {
    Identity(IdentityError),
    Claim(FirstOwnerError<E>),
}

impl<E> fmt::Debug for FirstOwnerCredentialError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Identity(_) => "identity",
            Self::Claim(_) => "claim",
        };
        f.debug_tuple("FirstOwnerCredentialError")
            .field(&kind)
            .finish()
    }
}

impl<E> fmt::Display for FirstOwnerCredentialError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(_) => {
                f.write_str("first-owner controller credential could not be derived")
            },
            Self::Claim(_) => f.write_str("first-owner claim workflow failed"),
        }
    }
}

impl<E> std::error::Error for FirstOwnerCredentialError<E> where E: fmt::Debug + 'static {}

/// Return the public Reticulum address of a first-owner controller scope.
///
/// The scope's private credential remains inside this crate. This is safe to
/// print as an initialization receipt before any board carrier is opened.
pub fn first_owner_controller_fingerprint(
    provider: &dyn IdentityProvider,
    controller_scope: &[u8],
) -> Result<AddressHash, IdentityError> {
    Ok(FirstOwnerCredential::derive(provider, controller_scope)?
        .identity
        .public()
        .hash())
}

/// Claim one physically witnessed board with an application-owned controller scope.
///
/// The scope selects the controller identity but is not a station id. The private signer stays
/// inside this crate; Postilion receives only the portable public claim and its proof.
pub async fn claim_first_owner<C>(
    provider: &dyn IdentityProvider,
    controller_scope: &[u8],
    controller: &mut FirstOwnerController<C>,
    plan: ClaimPlan,
) -> Result<ClaimOutcome, FirstOwnerCredentialError<C::Error>>
where
    C: FirstOwnerExchange,
{
    FirstOwnerCredential::derive(provider, controller_scope)
        .map_err(FirstOwnerCredentialError::Identity)?
        .claim(controller, plan)
        .await
        .map_err(FirstOwnerCredentialError::Claim)
}

/// A Reticulum station credential derived for one sited device.
///
/// The underlying identity remains private. Callers can inspect its public
/// form and construct a station configuration, but cannot ask this type to
/// serialize or persist the secret.
#[derive(Clone, Debug)]
pub struct SitedStationCredential {
    identity: PrivateIdentity,
    issuer_public_key: [u8; 32],
    control_signer: SitedStationControlSigner,
}

impl SitedStationCredential {
    fn derive(provider: &dyn IdentityProvider, device_id: DeviceId) -> Result<Self, IdentityError> {
        let material = ReticulumStationMaterial::derive(provider, device_id.as_uuid().as_bytes())?;
        let mut secret = material.secret_bytes();
        let identity = PrivateIdentity::from_secret_bytes(&secret);
        secret.zeroize();
        Ok(Self {
            identity,
            issuer_public_key: provider.master_public_key().to_bytes(),
            control_signer: SitedStationControlSigner::derive(provider, device_id)?,
        })
    }

    /// Derive a station credential from its durable host-side device id.
    ///
    /// The id is persisted only in the host's wallet grant and roster. It is
    /// not a radio location or a device-local account file.
    pub fn derive_for_device(
        provider: &dyn IdentityProvider,
        device_id: DeviceId,
    ) -> Result<Self, IdentityError> {
        Self::derive(provider, device_id)
    }

    /// The public Reticulum identity the station will announce.
    pub fn public_identity(&self) -> &Identity {
        self.identity.public()
    }

    /// The host-only signer for this station's grant and revoke controls.
    ///
    /// The signer is a device-specific Personae child. Its public attestation
    /// travels in each control frame, while this credential keeps the signing
    /// key inside the operator boundary.
    pub fn control_signer(&self) -> &SitedStationControlSigner {
        &self.control_signer
    }

    /// The durable host-side id this credential was derived for.
    pub fn device_id(&self) -> DeviceId {
        self.control_signer.device_id()
    }

    /// Provision a sealed unattended head without exporting its private identity.
    ///
    /// The returned head begins without authority. Deliver a signed grant control
    /// and verify its acknowledgement before opening the radio port.
    pub fn provision_head(
        &self,
        storage: SealedRecordStorage,
        record_path: impl AsRef<Path>,
    ) -> Result<SitedStationHead, SitedStationHeadError> {
        SitedStationHead::provision(
            storage,
            record_path,
            self.device_id(),
            self.identity.clone(),
        )
    }

    fn station_config(&self, port: impl Into<String>, name: impl Into<String>) -> StationConfig {
        StationConfig::new(port, name, self.identity.clone())
    }

    /// Issue the only grant shape an unattended station can receive.
    ///
    /// `authority` (djinn) signs it with the wallet, which must belong to the
    /// same Persona that derived this credential. The resulting grant carries
    /// exactly `transport.egress`, no personas, no private epochs, and a
    /// mandatory hard expiry; it is checked here before it is returned.
    pub fn issue_remote_auth_grant(
        &self,
        authority: &dyn StationAuthority,
        device_id: DeviceId,
        label: impl Into<String>,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<SitedStationGrant, SitedStationGrantError> {
        let request = SitedStationGrantRequest::new(
            device_id,
            *self.public_identity().ed25519_bytes(),
            label,
            issued_at_ms,
            expires_at_ms,
        )?;
        SitedStationGrant::from_signed(
            authority.issue_station_grant(&request, self.issuer_public_key)?,
        )
    }

    /// Acquire a host-side lease for this station's current active grant.
    ///
    /// `data_root` is where the wallet's public records live (djinn reports
    /// it); `authority` is who revokes.
    pub fn acquire_lease(
        &self,
        data_root: &Path,
        device_id: DeviceId,
        now_ms: u64,
        authority: Arc<dyn StationAuthority>,
    ) -> Result<SitedStationLease, SitedStationLeaseError> {
        SitedStationLease::acquire(
            data_root,
            device_id,
            *self.public_identity().ed25519_bytes(),
            now_ms,
            authority,
        )
    }

    /// Open a Retinue station under a fail-closed host-side lease.
    ///
    /// The running port checks the grant before every public station action and
    /// owns a deadline watchdog. A replacement grant can extend the live
    /// station only when it is written before the current deadline; a failed
    /// check drops the Retinue station and aborts its radio tasks.
    pub async fn open_station(
        &self,
        data_root: &Path,
        device_id: DeviceId,
        port: impl Into<String>,
        name: impl Into<String>,
        authority: Arc<dyn StationAuthority>,
    ) -> Result<SitedStation, SitedStationError> {
        let now_ms = unix_time_ms()?;
        let lease = self.acquire_lease(data_root, device_id, now_ms, authority)?;
        let station = Station::open(self.station_config(port, name))
            .await
            .map_err(SitedStationError::Station)?;
        let after_open_ms = unix_time_ms()?;
        lease.authorize_at(after_open_ms)?;
        let usable_at_ms = unix_time_ms()?;
        lease.accepted_expiry_at(usable_at_ms)?;
        Ok(SitedStation::new(station, lease))
    }
}

/// A running Retinue station whose public operations are lease-gated.
///
/// Its underlying `postilion::Station` stays private so callers cannot retain
/// a bypass around expiry or revocation checks.
pub struct SitedStation {
    station: Arc<Mutex<Option<Station>>>,
    lease: SitedStationLease,
    watchdog: tokio::task::JoinHandle<()>,
}

impl SitedStation {
    fn new(station: Station, lease: SitedStationLease) -> Self {
        let station = Arc::new(Mutex::new(Some(station)));
        let watchdog = tokio::spawn(watch_station_lease(lease.clone(), Arc::clone(&station)));
        Self {
            station,
            lease,
            watchdog,
        }
    }

    /// The active authorization lease for status rendering.
    pub fn lease(&self) -> &SitedStationLease {
        &self.lease
    }

    /// The delivery destination owned by this running station.
    ///
    /// Reading a public address still rechecks the lease so a caller cannot
    /// keep presenting a revoked station as live through this handle.
    pub async fn address(&self) -> Result<AddressHash, SitedStationError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        Ok(station
            .as_ref()
            .ok_or(SitedStationError::Stopped)?
            .address())
    }

    /// Recheck the host grant before announcing the station.
    pub async fn announce(&self) -> Result<(), SitedStationError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        station
            .as_ref()
            .ok_or(SitedStationError::Stopped)?
            .announce();
        Ok(())
    }

    /// Send one message, refusing to begin after expiry and interrupting the
    /// operation at the current lease deadline.
    pub async fn send_text(
        &self,
        prefix: &str,
        body: &str,
        patience: Duration,
    ) -> Result<Sent, SitedStationError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let window = remaining_window(now_ms, expires_at_ms);
        let result = {
            let station = self.station.lock().await;
            let station = station.as_ref().ok_or(SitedStationError::Stopped)?;
            tokio::time::timeout(window, station.send_text(prefix, body, patience)).await
        };
        match result {
            Ok(result) => result.map_err(SitedStationError::Station),
            Err(_) => self.stop_after_deadline().await,
        }
    }

    /// Carry one complete LXMF payload while the station lease remains live.
    ///
    /// Application-owned fields remain in their authenticated LXMF positions;
    /// this boundary does not copy them into a text body.
    pub async fn send_payload(
        &self,
        prefix: &str,
        payload: &LxmfPayload,
        patience: Duration,
    ) -> Result<Sent, SitedStationError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let window = remaining_window(now_ms, expires_at_ms);
        let result = {
            let station = self.station.lock().await;
            let station = station.as_ref().ok_or(SitedStationError::Stopped)?;
            tokio::time::timeout(window, station.send_payload(prefix, payload, patience)).await
        };
        match result {
            Ok(result) => result.map_err(SitedStationError::Station),
            Err(_) => self.stop_after_deadline().await,
        }
    }

    /// Carry one signed grant renewal or revoke frame to an announced station.
    ///
    /// The destination is the full address or a unique prefix from a prior
    /// [`Event::Message`]. The caller retains retry policy and records the
    /// returned station-signed acknowledgement separately.
    pub async fn send_control(
        &self,
        prefix: &str,
        control: &SitedStationControl,
        patience: Duration,
    ) -> Result<Sent, SitedStationError> {
        self.send_control_bytes(
            prefix,
            SITED_STATION_CONTROL_TITLE,
            &control.to_bytes()?,
            patience,
        )
        .await
    }

    /// Carry a station-signed acknowledgement back to the command sender.
    pub async fn send_control_ack(
        &self,
        prefix: &str,
        acknowledgement: &SitedStationControlAck,
        patience: Duration,
    ) -> Result<Sent, SitedStationError> {
        self.send_control_bytes(
            prefix,
            SITED_STATION_CONTROL_ACK_TITLE,
            &acknowledgement.to_bytes()?,
            patience,
        )
        .await
    }

    /// Capture Postilion's bounded management read model while the lease
    /// remains live.
    ///
    /// The snapshot is a read-only fact capture: it mutates no station,
    /// endpoint, or lease state beyond the ordinary fail-closed authorization
    /// recheck every public station operation performs, and the caller
    /// receives owned data rather than any handle onto the private station.
    pub async fn management_snapshot(
        &self,
    ) -> Result<postilion::management::ManagementSnapshot, SitedStationError> {
        self.authorize_now()?;
        let station = self.station.lock().await;
        Ok(station
            .as_ref()
            .ok_or(SitedStationError::Stopped)?
            .management_snapshot())
    }

    /// Wait for the next event, returning instead when the active deadline
    /// arrives so the caller cannot hold the station past expiry.
    pub async fn next_event(&self) -> Result<Event, SitedStationError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let window = remaining_window(now_ms, expires_at_ms);
        let result = {
            let mut station = self.station.lock().await;
            let station = station.as_mut().ok_or(SitedStationError::Stopped)?;
            tokio::time::timeout(window, station.next_event()).await
        };
        match result {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(SitedStationError::Stopped),
            Err(_) => self.stop_after_deadline().await,
        }
    }

    /// Deliberately stop the station, for operator-initiated revocation or
    /// shutdown. A new grant still needs a new station lease to restart it.
    pub async fn stop(&self) {
        self.lease.close();
        self.watchdog.abort();
        self.station.lock().await.take();
    }

    /// Revoke this station at the host, then drop the running Retinue station.
    ///
    /// The host stop is immediate. Sending that revocation across a remote
    /// station's radio link is deliberately not implied by this local action.
    pub async fn revoke(&self) -> Result<RemoteAuthRevocationOutcome, SitedStationError> {
        let outcome = self.lease.revoke()?;
        self.watchdog.abort();
        self.station.lock().await.take();
        Ok(outcome)
    }

    fn authorize_now(&self) -> Result<(u64, u64), SitedStationError> {
        let checked_at_ms = unix_time_ms()?;
        self.lease.authorize_at(checked_at_ms)?;
        let usable_at_ms = unix_time_ms()?;
        let expires_at_ms = self.lease.accepted_expiry_at(usable_at_ms)?;
        Ok((usable_at_ms, expires_at_ms))
    }

    async fn stop_after_deadline<T>(&self) -> Result<T, SitedStationError> {
        let now_ms = unix_time_ms()?;
        match self.lease.authorize_at(now_ms) {
            Ok(_) => Err(SitedStationError::OperationInterrupted),
            Err(error) => {
                self.station.lock().await.take();
                Err(error.into())
            },
        }
    }

    async fn send_control_bytes(
        &self,
        prefix: &str,
        title: &[u8],
        body: &[u8],
        patience: Duration,
    ) -> Result<Sent, SitedStationError> {
        let (now_ms, expires_at_ms) = self.authorize_now()?;
        let window = remaining_window(now_ms, expires_at_ms);
        let result = {
            let station = self.station.lock().await;
            let station = station.as_ref().ok_or(SitedStationError::Stopped)?;
            tokio::time::timeout(window, station.send_bytes(prefix, title, body, patience)).await
        };
        match result {
            Ok(result) => result.map_err(SitedStationError::Station),
            Err(_) => self.stop_after_deadline().await,
        }
    }
}

impl Drop for SitedStation {
    fn drop(&mut self) {
        self.watchdog.abort();
    }
}

/// Why a lease-gated station could not run an operation.
#[derive(Debug)]
pub enum SitedStationError {
    /// The host clock could not be read.
    Clock(std::time::SystemTimeError),
    /// The current station lease denied the action.
    Lease(SitedStationLeaseError),
    /// A station-control frame could not be encoded.
    Control(SitedStationControlError),
    /// Retinue could not open or use its radio station.
    Station(postilion::Error),
    /// The station had already been stopped.
    Stopped,
    /// The station was renewed while a timed operation was being cancelled.
    OperationInterrupted,
}

impl From<std::time::SystemTimeError> for SitedStationError {
    fn from(error: std::time::SystemTimeError) -> Self {
        Self::Clock(error)
    }
}

impl From<SitedStationLeaseError> for SitedStationError {
    fn from(error: SitedStationLeaseError) -> Self {
        Self::Lease(error)
    }
}

impl From<SitedStationControlError> for SitedStationError {
    fn from(error: SitedStationControlError) -> Self {
        Self::Control(error)
    }
}

impl fmt::Display for SitedStationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clock(error) => write!(f, "could not read the station clock: {error}"),
            Self::Lease(error) => write!(f, "station lease denied the operation: {error}"),
            Self::Control(error) => write!(f, "station control frame failed: {error}"),
            Self::Station(error) => write!(f, "Retinue station operation failed: {error}"),
            Self::Stopped => f.write_str("sited station is stopped"),
            Self::OperationInterrupted => f.write_str(
                "station operation was interrupted at its old deadline after a grant renewal; retry it",
            ),
        }
    }
}

impl std::error::Error for SitedStationError {}

fn unix_time_ms() -> Result<u64, std::time::SystemTimeError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(u64::try_from(duration.as_millis()).expect("unix millisecond count fits u64"))
}

fn remaining_window(now_ms: u64, expires_at_ms: u64) -> Duration {
    Duration::from_millis(expires_at_ms.saturating_sub(now_ms))
}

async fn watch_station_lease(lease: SitedStationLease, station: Arc<Mutex<Option<Station>>>) {
    loop {
        let now_ms = match unix_time_ms() {
            Ok(now_ms) => now_ms,
            Err(_) => {
                lease.close();
                station.lock().await.take();
                return;
            },
        };
        let expires_at_ms = lease.expires_at_ms();
        let window = remaining_window(now_ms, expires_at_ms);
        tokio::time::sleep(window).await;

        let now_ms = match unix_time_ms() {
            Ok(now_ms) => now_ms,
            Err(_) => {
                lease.close();
                station.lock().await.take();
                return;
            },
        };
        if lease.authorize_at(now_ms).is_err() {
            station.lock().await.take();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use personae::carry::DeviceId;
    use personae::InMemoryProvider;

    use crate::authority::test_support::TestWallet;
    use postilion::control::first_owner::v4_usb_claim_plan;
    use radio_hand::control::{
        ClaimResponse, FirstOwnerRequest, FirstOwnerResponse, FirstWriteStatus, NodeId,
        PairEvidence, ResumeResponse,
    };
    use radio_hand::region::Region;
    use tempfile::tempdir;

    use super::*;

    struct FirstOwnerScript {
        replies: VecDeque<Result<FirstOwnerResponse, &'static str>>,
        requests: Vec<FirstOwnerRequest>,
    }

    impl FirstOwnerScript {
        fn new(
            replies: impl IntoIterator<Item = Result<FirstOwnerResponse, &'static str>>,
        ) -> Self {
            Self {
                replies: replies.into_iter().collect(),
                requests: Vec::new(),
            }
        }
    }

    impl FirstOwnerExchange for FirstOwnerScript {
        type Error = &'static str;

        fn exchange(
            &mut self,
            request: FirstOwnerRequest,
        ) -> impl std::future::Future<Output = Result<FirstOwnerResponse, Self::Error>> {
            self.requests.push(request);
            std::future::ready(self.replies.pop_front().expect("script reply"))
        }
    }

    #[test]
    fn controller_claims_through_a_carrier_without_exposing_or_reusing_station_identity() {
        let provider = InMemoryProvider::from_seed([0x44; 32]);
        let scope = b"first-owner-controller:west-wall";
        let credential = FirstOwnerCredential::derive(&provider, scope).unwrap();
        let station =
            SitedStationCredential::derive_for_device(&provider, DeviceId::new()).unwrap();
        let mut phy = postilion::profile(250_000);
        phy.frequency_hz = 906_875_000;
        phy.tx_power_dbm = 17;
        let plan = v4_usb_claim_plan(Region::Us915, phy).unwrap();
        let mut controller = FirstOwnerController::new(FirstOwnerScript::new([
            Ok(FirstOwnerResponse::Inspect {
                status: FirstWriteStatus {
                    control: PairEvidence::Blank,
                    pending: PairEvidence::Blank,
                },
                node: NodeId([0x22; 16]),
                nonce: [0x51; 32],
            }),
            Ok(FirstOwnerResponse::Claim(ClaimResponse::Staged)),
            Ok(FirstOwnerResponse::Resume(ResumeResponse::Committed)),
        ]));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        assert!(matches!(
            runtime.block_on(claim_first_owner(&provider, scope, &mut controller, plan)),
            Ok(ClaimOutcome::Committed)
        ));

        let script = controller.into_carrier();
        assert!(matches!(script.requests[0], FirstOwnerRequest::Inspect));
        let FirstOwnerRequest::Claim(claim) = &script.requests[1] else {
            panic!("the credential must claim after a fresh inspection");
        };
        assert_eq!(
            claim.claim().owner_public_identity(),
            &credential.identity.public().to_public_bytes()
        );
        assert_ne!(
            claim.claim().owner_public_identity(),
            &station.public_identity().to_public_bytes()
        );
        assert!(matches!(script.requests[2], FirstOwnerRequest::Resume));
    }

    #[test]
    fn one_persona_and_station_scope_keep_the_same_reticulum_address() {
        let provider = InMemoryProvider::from_seed([0x44; 32]);
        let device_id = DeviceId::new();
        let first = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        let second = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();

        assert_eq!(
            first.public_identity().hash(),
            second.public_identity().hash()
        );
    }

    #[test]
    fn station_scopes_cannot_share_a_reticulum_address_accidentally() {
        let provider = InMemoryProvider::from_seed([0x44; 32]);
        let first = SitedStationCredential::derive_for_device(&provider, DeviceId::new()).unwrap();
        let second = SitedStationCredential::derive_for_device(&provider, DeviceId::new()).unwrap();

        assert_ne!(
            first.public_identity().hash(),
            second.public_identity().hash()
        );
    }

    #[test]
    fn credential_provisions_a_sealed_head_without_exporting_its_identity() {
        let storage_root = tempdir().unwrap();
        let storage = SealedRecordStorage::open_with_key(storage_root.path(), [0x43; 32]);
        let provider = InMemoryProvider::from_seed([0x44; 32]);
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();

        let head = credential
            .provision_head(storage.clone(), "station/ridge-north.json")
            .unwrap();
        let restored = SitedStationHead::restore(storage, "station/ridge-north.json").unwrap();

        assert_eq!(head.device_id(), device_id);
        assert_eq!(restored.device_id(), device_id);
        assert_eq!(restored.public_identity(), credential.public_identity());
    }

    #[test]
    fn station_config_is_a_typed_injection_not_an_identity_path() {
        let provider = InMemoryProvider::from_seed([0x44; 32]);
        let credential =
            SitedStationCredential::derive_for_device(&provider, DeviceId::new()).unwrap();
        let config = credential.station_config("COM7", "Ridge north");

        assert_eq!(config.port, "COM7");
        assert_eq!(config.name, "Ridge north");
        assert_eq!(
            config.identity.public().hash(),
            credential.public_identity().hash()
        );
    }

    #[test]
    fn station_lease_is_egress_only_renews_before_expiry_and_then_fails_closed() {
        let root = tempdir().unwrap();
        let wallet = TestWallet::new(root.path(), 0x5a);
        let provider = wallet.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();

        let grant = credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let set = grant.signed();
        assert!(
            set.personas.is_empty(),
            "a station gets no persona authority"
        );
        let certificate = set.device.as_ref().expect("a device certificate");
        assert_eq!(
            certificate
                .certificate
                .scope
                .actions
                .iter()
                .collect::<Vec<_>>(),
            ["transport.egress"]
        );
        assert_eq!(certificate.certificate.remaining_delegation_depth, 0);

        let lease = credential
            .acquire_lease(root.path(), device_id, 199, wallet.clone())
            .unwrap();
        assert_eq!(lease.expires_at_ms(), 200);
        credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 199, 300)
            .unwrap();
        assert_eq!(lease.authorize_at(199).unwrap(), 300);
        assert_eq!(lease.authorize_at(200).unwrap(), 300);

        let expiry = lease.authorize_at(300).unwrap_err();
        assert!(matches!(
            expiry,
            SitedStationLeaseError::DeadlineElapsed {
                expires_at_ms: 300,
                ..
            }
        ));
        assert!(lease.is_closed());

        credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 300, 400)
            .unwrap();
        let closed = lease.authorize_at(301).unwrap_err();
        assert!(matches!(closed, SitedStationLeaseError::Closed { .. }));
    }

    #[test]
    fn station_lease_fails_closed_when_the_host_revokes_the_device() {
        let root = tempdir().unwrap();
        let wallet = TestWallet::new(root.path(), 0x5a);
        let provider = wallet.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let lease = credential
            .acquire_lease(root.path(), device_id, 199, wallet.clone())
            .unwrap();

        let outcome = lease.revoke().unwrap();
        assert_eq!(outcome.device_id, device_id);
        let revoked = lease.authorize_at(199).unwrap_err();
        assert!(matches!(revoked, SitedStationLeaseError::Closed { .. }));
        assert!(lease.is_closed());
    }

    #[test]
    fn a_replacement_written_after_the_last_accepted_deadline_cannot_revive_a_lease() {
        let root = tempdir().unwrap();
        let wallet = TestWallet::new(root.path(), 0x5a);
        let provider = wallet.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        let lease = credential
            .acquire_lease(root.path(), device_id, 199, wallet.clone())
            .unwrap();

        credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 200, 300)
            .unwrap();
        let expired = lease.authorize_at(200).unwrap_err();

        assert!(matches!(
            expired,
            SitedStationLeaseError::DeadlineElapsed {
                expires_at_ms: 200,
                ..
            }
        ));
        assert!(lease.is_closed());
    }

    #[test]
    fn commissioning_refuses_a_credential_from_another_persona() {
        let root = tempdir().unwrap();
        let wallet = TestWallet::new(root.path(), 0x5a);
        let foreign_provider = InMemoryProvider::from_seed([0x77; 32]);
        let credential =
            SitedStationCredential::derive_for_device(&foreign_provider, DeviceId::new()).unwrap();

        let error = credential
            .issue_remote_auth_grant(wallet.as_ref(), DeviceId::new(), "Ridge north", 100, 200)
            .unwrap_err();
        assert!(matches!(error, SitedStationGrantError::IssuerMismatch));
    }
}
