// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! An I2P-only relay profile. Both legs use injected SAM sessions, with no
//! native endpoint, IP route, ticket parser or weaker security fallback.

use super::{
    DeviceBudget, FlowLimits, RelayError, RelayGrant, RelayRequest, budget::PoolBudget, channel,
};
use crate::{Admission, Credentials};
use identity::{Ed25519Keypair, IdentityProvider};
use notochord::{AdmittedPrincipal, CarrierKind, SessionFacts};
use std::collections::BTreeMap;
use tokio::time::Instant;
use transport::Alpn;
use transport::PeerID;
pub use transport::i2p::{I2pAddress, I2pSession, I2pStream, TunnelProfile};
use transport::security::{
    PreparedStream, ServiceSecurityPolicy, SetupGate, SetupGuard, SetupLimits,
};

/// Exact application profile, negotiated inside Noise over SAM.
pub const I2P_RELAY_PROTOCOL: &[u8] = b"murm/relay/i2p/stream/v1";
/// Independently admitted relay-to-recipient leg, also exclusively I2P.
pub const I2P_RECIPIENT_PROTOCOL: &[u8] = b"murm/relay/i2p/recipient/v1";
/// An admitted encrypted SAM stream; applications still add their inner E2E session.
pub type I2pRelayStream = PreparedStream<I2pStream>;

/// An authorized I2P-only route and independently expected service key.
#[derive(Clone, Debug)]
pub struct I2pRoute {
    /// Canonical service alias, deliberately separate from owner identity.
    pub address: I2pAddress,
    /// Expected application Noise identity, verified independently of SAM.
    pub service_key: [u8; 32],
}

/// Noise and Notochord admission over an injected I2P session.
#[derive(Clone)]
pub struct I2pRelayChannel {
    protocol: Alpn,
    gate: SetupGate,
    limits: SetupLimits,
}
impl I2pRelayChannel {
    /// One channel per service; clones share pending setup capacity.
    pub fn new(protocol: &[u8], max_pending: usize, limits: SetupLimits) -> Self {
        Self::shared(protocol, SetupGate::new(max_pending), limits)
    }
    fn shared(protocol: &[u8], gate: SetupGate, limits: SetupLimits) -> Self {
        Self {
            protocol: Alpn::from_bytes(protocol.to_vec()),
            gate,
            limits,
        }
    }
    async fn connect_pending<I: IdentityProvider>(
        &self,
        session: &I2pSession,
        route: &I2pRoute,
        key: &Ed25519Keypair,
        credentials: Credentials<'_, I>,
    ) -> Result<channel::Pending<I2pStream>, RelayError> {
        let expected = PeerID::from_bytes(&route.service_key).map_err(|_| RelayError::Setup)?;
        let policy = ServiceSecurityPolicy::noise(
            self.protocol.clone(),
            Some(expected),
            self.limits.clone(),
        );
        let guard = self
            .gate
            .reserve(&policy)
            .map_err(RelayError::SecureSetup)?;
        let stream = guard
            .run(session.connect(&route.address))
            .await
            .map_err(RelayError::SecureSetup)?
            .map_err(|_| RelayError::Setup)?;
        let secured = guard
            .noise_initiator(key, stream)
            .await
            .map_err(RelayError::SecureSetup)?;
        // SAM did not authenticate a Personae identity. Noise supplies the
        // separate channel proof; no destination is forged into a root key.
        let facts = SessionFacts::new(self.protocol.as_bytes().to_vec(), CarrierKind::Other);
        channel::initiate(secured, facts.proof_binding(), credentials).await
    }
    /// Connect to a registered recipient service and complete outer admission.
    ///
    /// # Errors
    /// Refuses carrier, security, setup-budget and admission failures.
    pub async fn connect<I: IdentityProvider>(
        &self,
        session: &I2pSession,
        route: &I2pRoute,
        key: &Ed25519Keypair,
        credentials: Credentials<'_, I>,
    ) -> Result<I2pRelayStream, RelayError> {
        let (stream, guard) = self
            .connect_pending(session, route, key, credentials)
            .await?;
        guard.admitted().map_err(RelayError::SecureSetup)?;
        Ok(stream)
    }
    async fn accept_pending(
        &self,
        stream: I2pStream,
        key: &Ed25519Keypair,
        admission: Admission<'_>,
    ) -> Result<(I2pRelayStream, AdmittedPrincipal, SetupGuard), RelayError> {
        let policy = ServiceSecurityPolicy::noise(self.protocol.clone(), None, self.limits.clone());
        let guard = self
            .gate
            .reserve(&policy)
            .map_err(RelayError::SecureSetup)?;
        let secured = guard
            .noise_responder(key, stream)
            .await
            .map_err(RelayError::SecureSetup)?;
        let facts = SessionFacts::new(self.protocol.as_bytes().to_vec(), CarrierKind::Other);
        channel::admit(secured, &facts, admission).await
    }
    /// Accept an independent outer channel; recipient application policy still applies.
    ///
    /// # Errors
    /// Refuses failed Noise, admission or exhausted setup limits.
    pub async fn accept(
        &self,
        stream: I2pStream,
        key: &Ed25519Keypair,
        admission: Admission<'_>,
    ) -> Result<(I2pRelayStream, AdmittedPrincipal), RelayError> {
        let (stream, principal, guard) = self.accept_pending(stream, key, admission).await?;
        guard.admitted().map_err(RelayError::SecureSetup)?;
        Ok((stream, principal))
    }
    /// Open a routed stream only after service authentication and live authorization.
    ///
    /// # Errors
    /// Refuses carrier/key/profile mismatches, expired setup or refused routing.
    pub async fn open<I: IdentityProvider>(
        &self,
        session: &I2pSession,
        route: &I2pRoute,
        key: &Ed25519Keypair,
        credentials: Credentials<'_, I>,
        request: RelayRequest,
    ) -> Result<I2pRelayStream, RelayError> {
        let (mut stream, guard) = self
            .connect_pending(session, route, key, credentials)
            .await?;
        channel::request(&mut stream, &guard, request).await?;
        guard.admitted().map_err(RelayError::SecureSetup)?;
        Ok(stream)
    }
}

/// A worker whose entry and every registered target leg are strictly I2P.
/// It owns no router, accept loop, identity custody or group state.
pub struct I2pRelayService {
    pool: [u8; 32],
    session: I2pSession,
    targets: BTreeMap<[u8; 32], I2pRoute>,
    budget: PoolBudget,
    entry: I2pRelayChannel,
    target_channel: I2pRelayChannel,
}
impl I2pRelayService {
    /// Register approved I2P targets and inject the host's shared device allowance.
    ///
    /// # Errors
    /// Refuses invalid limits, duplicate/empty/excessive targets or unpublished ingress.
    pub fn new(
        pool: [u8; 32],
        session: I2pSession,
        targets: Vec<([u8; 32], I2pRoute)>,
        device: DeviceBudget,
        flow: FlowLimits,
        setup: SetupLimits,
    ) -> Result<Self, RelayError> {
        let count = targets.len();
        let targets: BTreeMap<_, _> = targets.into_iter().collect();
        if count == 0 || count > 256 || targets.len() != count || !session.profile().publishes() {
            return Err(RelayError::InvalidLimits);
        }
        Ok(Self {
            pool,
            session,
            targets,
            entry: I2pRelayChannel::shared(I2P_RELAY_PROTOCOL, device.setup(), setup.clone()),
            target_channel: I2pRelayChannel::shared(I2P_RECIPIENT_PROTOCOL, device.setup(), setup),
            budget: PoolBudget::new(device, flow)?,
        })
    }
    /// Serve an accepted SAM stream, enforcing the same limits/rechecks as native
    /// forwarding. `service_key` is a scoped public-service key, not an owner root.
    ///
    /// # Errors
    /// Refuses failed setup, unavailable authority/target, exhausted limits,
    /// provider termination and interrupted forwarding.
    pub async fn serve<I, F>(
        &self,
        stream: I2pStream,
        service_key: &Ed25519Keypair,
        admission: Admission<'_>,
        target_key: &Ed25519Keypair,
        credentials: Credentials<'_, I>,
        authorize: F,
    ) -> Result<(), RelayError>
    where
        I: IdentityProvider,
        F: Fn(&AdmittedPrincipal, &RelayRequest) -> Option<RelayGrant>,
    {
        let (mut inbound, principal, guard) = self
            .entry
            .accept_pending(stream, service_key, admission)
            .await?;
        let request = channel::read_request(&mut inbound, &guard).await?;
        if request.pool != self.pool {
            return Err(RelayError::Unauthorized);
        }
        let target = self
            .targets
            .get(&request.target)
            .ok_or(RelayError::Unauthorized)?;
        let granted = authorize(&principal, &request)
            .filter(|g| g.expires > Instant::now())
            .ok_or(RelayError::Unauthorized)?;
        let active = self.budget.reserve(granted)?;
        let reservation = &active.forwarding;
        let deadline = granted
            .expires
            .min(reservation.started + reservation.limits.lifetime);
        let outbound = tokio::time::timeout_at(
            deadline,
            guard.run(
                self.target_channel
                    .connect(&self.session, target, target_key, credentials),
            ),
        )
        .await
        .map_err(|_| RelayError::Unauthorized)?
        .map_err(RelayError::SecureSetup)??;
        let current = authorize(&principal, &request).ok_or(RelayError::Unauthorized)?;
        if current.revision != granted.revision
            || current.expires <= Instant::now()
            || Instant::now() >= deadline
            || current.max_sessions < granted.max_sessions
            || current.max_bytes < granted.max_bytes
            || current.bytes_per_second < granted.bytes_per_second
        {
            return Err(RelayError::Unauthorized);
        }
        channel::ready(&mut inbound, &guard).await?;
        guard.admitted().map_err(RelayError::SecureSetup)?;
        tokio::select! {
            result = super::pump::forward(inbound, outbound, reservation, granted, || authorize(&principal, &request)) => result,
            _ = self.session.closed() => Err(RelayError::Setup),
        }
    }
}
