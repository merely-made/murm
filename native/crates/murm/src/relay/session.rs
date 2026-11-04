// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::{DeviceBudget, FlowLimits, RECIPIENT_PROTOCOL, RELAY_PROTOCOL, RelayError};
use crate::{Admission, Credentials};
use identity::IdentityProvider;
use notochord::{
    AdmittedPrincipal, SessionHello, SessionReply, admit_session_secure, initiate_session,
    read_frame_or_eof, write_frame,
};
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
        let (mut stream, security, guard) = guard
            .native(outbound.stream)
            .map_err(|_| RelayError::Setup)?;
        let secure = security.secure_channel_facts().ok_or(RelayError::Setup)?;
        let hello = SessionHello::issue_secure(
            credentials.identity,
            credentials.network,
            credentials.profile,
            credentials.action,
            credentials.class,
            credentials.nonce,
            &binding.with_secure_channel(secure),
            credentials.delegations,
        )
        .map_err(|_| RelayError::Setup)?;
        let reply = guard
            .run(initiate_session(&mut stream, &hello, &Default::default()))
            .await
            .map_err(|_| RelayError::Setup)?
            .map_err(|_| RelayError::Setup)?;
        if !matches!(reply, SessionReply::Accept { .. }) {
            guard.refused();
            return Err(RelayError::Unauthorized);
        }
        Ok((stream, guard))
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
        let secure = security.secure_channel_facts().ok_or(RelayError::Setup)?;
        let admitted = guard
            .run(admit_session_secure(
                stream,
                admission.policy,
                admission.ledger,
                &facts,
                &secure,
                admission.identity,
                admission.now_ms,
                admission.active_sessions,
            ))
            .await
            .map_err(|_| RelayError::Setup)?
            .map_err(|_| RelayError::Setup)?;
        let session = match admitted {
            Ok(session) => session.into_session(),
            Err(_) => {
                guard.refused();
                return Err(RelayError::Unauthorized);
            },
        };
        Ok((session.stream, session.principal, guard))
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
        let exchange = async {
            let bytes = [request.pool.as_slice(), request.target.as_slice()].concat();
            write_frame(&mut stream, &bytes, 64)
                .await
                .map_err(|_| RelayError::Setup)?;
            if read_frame_or_eof(&mut stream, 1)
                .await
                .map_err(|_| RelayError::Setup)?
                .as_deref()
                != Some(&[1])
            {
                return Err(RelayError::Unauthorized);
            }
            Ok::<_, RelayError>(())
        };
        guard.run(exchange).await.map_err(|_| RelayError::Setup)??;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        Ok(stream)
    }
}

/// A native worker for one pool. It opens no endpoint and spawns no tasks.
pub struct RelayService {
    pool: [u8; 32],
    targets: BTreeMap<[u8; 32], PeerID>,
    carrier: Arc<P2pandaTransport>,
    device: DeviceBudget,
    flow: FlowLimits,
    entry: RelayChannel,
    target_channel: RelayChannel,
    pool_slots: Arc<tokio::sync::Semaphore>,
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
            pool_slots: Arc::new(tokio::sync::Semaphore::new(flow.max_sessions)),
            entry: RelayChannel::shared(RELAY_PROTOCOL, device.setup(), setup.clone()),
            target_channel: RelayChannel::shared(RECIPIENT_PROTOCOL, device.setup(), setup),
            device,
            flow,
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
        let request = guard
            .run(async {
                let bytes = read_frame_or_eof(&mut inbound, 64)
                    .await
                    .map_err(|_| RelayError::Setup)?
                    .filter(|bytes| bytes.len() == 64)
                    .ok_or(RelayError::Unauthorized)?;
                Ok::<_, RelayError>(RelayRequest {
                    pool: bytes[..32].try_into().unwrap(),
                    target: bytes[32..].try_into().unwrap(),
                })
            })
            .await
            .map_err(|_| RelayError::Setup)??;
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
        let mut flow = self.flow;
        flow.max_bytes = flow.max_bytes.min(granted.max_bytes);
        flow.bytes_per_second = flow.bytes_per_second.min(granted.bytes_per_second);
        let _pool_slot = self
            .pool_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::Capacity)?;
        if self.flow.max_sessions - self.pool_slots.available_permits() > granted.max_sessions {
            return Err(RelayError::Capacity);
        }
        let reservation = self.device.reserve(flow)?;
        let flow_deadline = granted.expires.min(reservation.started + flow.lifetime);
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
        guard
            .run(write_frame(&mut inbound, &[1], 1))
            .await
            .map_err(|_| RelayError::Setup)?
            .map_err(|_| RelayError::Setup)?;
        guard.admitted().map_err(|_| RelayError::Setup)?;
        super::pump::forward(inbound, outbound, &reservation, granted, || {
            authorize(&principal, &request)
        })
        .await
    }
}
