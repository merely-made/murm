// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::{DeviceBudget, FlowLimits, RECIPIENT_PROTOCOL, RELAY_PROTOCOL, RelayError};
use super::{budget::PoolBudget, channel};
use crate::{Admission, Credentials};
use identity::IdentityProvider;
use notochord::AdmittedPrincipal;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::time::Instant;
use transport::security::{
    PreparedStream, ServiceSecurityPolicy, SetupGate, SetupGuard, SetupLimits,
};
use transport::{AcceptedSession, Alpn, P2pandaStream, P2pandaTransport, PeerID, Transport};

type Stream = PreparedStream<P2pandaStream>;

/// A registered logical target; never a client-supplied arbitrary address.
#[derive(Clone, Copy, Debug)]
pub struct RelayTarget {
    /// Target ID in this pool's policy.
    pub id: [u8; 32],
    /// Host-authorized carrier route for that target.
    pub peer: PeerID,
}

/// A client's fixed-size routing request, carried inside the secure session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelayRequest {
    /// Exact community pool.
    pub pool: [u8; 32],
    /// Registered target ID.
    pub target: [u8; 32],
}

/// Host authorization snapshot, refreshed while the flow is active.
#[derive(Clone, Copy, Debug)]
pub struct RelayGrant {
    /// Changes whenever any authority/route/limit affecting this grant changes.
    pub revision: [u8; 32],
    /// Current pool contribution's concurrent-flow ceiling.
    pub max_sessions: usize,
    /// Combined byte ceiling, intersected with the worker's own flow limit.
    pub max_bytes: u64,
    /// Combined forwarding rate allowed by the current contribution/use policy.
    pub bytes_per_second: u64,
    /// Monotonic authority/contribution/availability deadline.
    pub expires: Instant,
}

/// A reusable secure native channel and its bounded Notochord admission gate.
#[derive(Clone)]
pub struct RelayChannel {
    protocol: Alpn,
    gate: SetupGate,
    limits: SetupLimits,
}

impl RelayChannel {
    /// Create one channel per service; clones share pending setup capacity.
    pub fn new(protocol: &[u8], max_pending: usize, limits: SetupLimits) -> Self {
        Self {
            protocol: Alpn::from_bytes(protocol.to_vec()),
            gate: SetupGate::new(max_pending),
            limits,
        }
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
        carrier: &P2pandaTransport,
        peer: PeerID,
        credentials: Credentials<'_, I>,
    ) -> Result<(Stream, SetupGuard), RelayError> {
        let policy =
            ServiceSecurityPolicy::native(self.protocol.clone(), Some(peer), self.limits.clone());
        let guard = self.gate.reserve(&policy).map_err(|_| RelayError::Setup)?;
        let outbound = guard
            .run(carrier.connect_session(peer, self.protocol.clone()))
            .await
            .map_err(|_| RelayError::Setup)?
            .map_err(|_| RelayError::Setup)?;
        let binding = outbound.proof_binding();
        let (stream, security, guard) = guard
            .native(outbound.stream)
            .map_err(|_| RelayError::Setup)?;
        channel::initiate((stream, security, guard), binding, credentials).await
    }

    /// Connect and complete secure admission to a registered recipient host.
    pub async fn connect<I: IdentityProvider>(
        &self,
        carrier: &P2pandaTransport,
        peer: PeerID,
        credentials: Credentials<'_, I>,
    ) -> Result<Stream, RelayError> {
        let (stream, guard) = self.connect_pending(carrier, peer, credentials).await?;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        Ok(stream)
    }

    async fn accept_pending(
        &self,
        accepted: AcceptedSession<P2pandaStream>,
        admission: Admission<'_>,
    ) -> Result<(Stream, AdmittedPrincipal, SetupGuard), RelayError> {
        let (stream, facts) = accepted.into_session();
        let policy =
            ServiceSecurityPolicy::native(self.protocol.clone(), None, self.limits.clone());
        let (stream, security, guard) = self
            .gate
            .reserve(&policy)
            .and_then(|guard| guard.native(stream))
            .map_err(|_| RelayError::Setup)?;
        channel::admit((stream, security, guard), &facts, admission).await
    }

    /// Accept an independently admitted recipient channel; the target still
    /// authenticates the final requester through the inner encrypted session.
    pub async fn accept(
        &self,
        accepted: AcceptedSession<P2pandaStream>,
        admission: Admission<'_>,
    ) -> Result<(Stream, AdmittedPrincipal), RelayError> {
        let (stream, principal, guard) = self.accept_pending(accepted, admission).await?;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        Ok((stream, principal))
    }

    /// Open a relay stream. No application bytes are sent until its route is accepted.
    pub async fn open<I: IdentityProvider>(
        &self,
        carrier: &P2pandaTransport,
        peer: PeerID,
        credentials: Credentials<'_, I>,
        request: RelayRequest,
    ) -> Result<Stream, RelayError> {
        let (mut stream, guard) = self.connect_pending(carrier, peer, credentials).await?;
        channel::request(&mut stream, &guard, request).await?;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        Ok(stream)
    }
}

/// A native worker for one pool. It opens no endpoint and spawns no tasks.
pub struct RelayService {
    pool: [u8; 32],
    targets: BTreeMap<[u8; 32], PeerID>,
    carrier: Arc<P2pandaTransport>,
    budget: PoolBudget,
    entry: RelayChannel,
    target_channel: RelayChannel,
}

impl RelayService {
    /// Register the complete allow-list and inject the host's shared resources.
    pub fn new(
        pool: [u8; 32],
        targets: Vec<RelayTarget>,
        carrier: Arc<P2pandaTransport>,
        device: DeviceBudget,
        flow: FlowLimits,
        setup: SetupLimits,
    ) -> Result<Self, RelayError> {
        flow.validate()?;
        let count = targets.len();
        let targets: BTreeMap<_, _> = targets.into_iter().map(|t| (t.id, t.peer)).collect();
        if targets.len() != count || count == 0 || count > 256 {
            return Err(RelayError::InvalidLimits);
        }
        Ok(Self {
            pool,
            targets,
            carrier,
            entry: RelayChannel::shared(RELAY_PROTOCOL, device.setup(), setup.clone()),
            target_channel: RelayChannel::shared(RECIPIENT_PROTOCOL, device.setup(), setup),
            budget: PoolBudget::new(device, flow)?,
        })
    }

    /// Admit, resolve a registered target and forward opaque bytes. The owner
    /// must refresh the authorization view; failure/expiry cancels active work.
    pub async fn serve<I, F>(
        &self,
        accepted: AcceptedSession<P2pandaStream>,
        admission: Admission<'_>,
        target_credentials: Credentials<'_, I>,
        authorize: F,
    ) -> Result<(), RelayError>
    where
        I: IdentityProvider,
        F: Fn(&AdmittedPrincipal, &RelayRequest) -> Option<RelayGrant>,
    {
        let (mut inbound, principal, guard) =
            self.entry.accept_pending(accepted, admission).await?;
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
        let flow_deadline = granted
            .expires
            .min(reservation.started + reservation.limits.lifetime);
        let outbound = tokio::time::timeout_at(
            flow_deadline,
            guard.run(
                self.target_channel
                    .connect(&self.carrier, *target, target_credentials),
            ),
        )
        .await
        .map_err(|_| RelayError::Unauthorized)?
        .map_err(|_| RelayError::Setup)??;
        let current = authorize(&principal, &request).ok_or(RelayError::Unauthorized)?;
        if current.revision != granted.revision
            || current.max_sessions < granted.max_sessions
            || current.expires <= Instant::now()
            || Instant::now() >= flow_deadline
            || current.max_bytes < granted.max_bytes
            || current.bytes_per_second < granted.bytes_per_second
        {
            return Err(RelayError::Unauthorized);
        }
        channel::ready(&mut inbound, &guard).await?;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        super::pump::forward(inbound, outbound, reservation, granted, || {
            authorize(&principal, &request)
        })
        .await
    }
}
