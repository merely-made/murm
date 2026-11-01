// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Native secure admission for an optional OCapN service on an existing endpoint.
//!
//! The host registers a bounded service route on its existing `P2pandaTransport`.
//! This crate borrows that transport; it owns no endpoint, discovery or keys.
//! Native iroh TLS supplies confidentiality and a stream-specific exporter
//! binding. Notochord proves the application principal before CapTP starts.

use identity::IdentityProvider;
use insigne::delegation::SignedDelegationCertificate;
use notochord::{
    AdmittedSecureSession, DenyReason, IoHandshakeError, LocalNetworkPolicy, NetworkId, ProfileRef,
    RequestedAction, RevocationLedger, SecureChannelFacts, SecureIdentityPolicy, SessionHello,
    SessionReply, TrafficClass, admit_session_secure, initiate_session,
};
use std::time::Duration;
use transport::security::{
    PreparedStream, ServiceSecurityPolicy, SetupError, SetupGate, SetupLimits, SetupStats,
};
use transport::{
    Alpn, BoundedAcceptHandle, BoundedAcceptStats, P2pandaStream, P2pandaTransport, PeerID,
    Transport, TransportError,
};

/// The initial bounded Gemot service's exact, versioned protocol.
pub const GEMOT_ALPN: &[u8] = b"mere/gemot-coop/ocapn-suite-31f0b806/v1";
/// Application admission domain; method authority remains with Gemot.
pub const GEMOT_DOMAIN: &str = "mere.gemot";
/// Application admission service path.
pub const GEMOT_SERVICE: &str = "/services/gemot-coop";
/// Permission to connect to the adapter. This grants no collection mutation.
pub const CONNECT: &str = "connect";

/// One native confidential stream whose application principal was admitted.
pub type NativeAdmitted = AdmittedSecureSession<PreparedStream<P2pandaStream>>;

/// Failure before any CapTP runtime or object is exposed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Carrier establishment or acceptance failed.
    #[error(transparent)]
    Carrier(#[from] TransportError),
    /// Security setup, budget or lifetime failed.
    #[error(transparent)]
    Setup(#[from] SetupError),
    /// Admission framing failed.
    #[error(transparent)]
    HandshakeIo(#[from] IoHandshakeError),
    /// The local identity could not issue its secure hello.
    #[error(transparent)]
    Hello(#[from] notochord::HandshakeError),
    /// The admission owner refused the principal.
    #[error("OCapN service admission refused: {0:?}")]
    Refused(DenyReason),
    /// The admitted action is not the action this service implements.
    #[error("OCapN service action mismatch")]
    ActionMismatch,
    /// The responder sent a refusal.
    #[error("OCapN responder refused admission")]
    RemoteRefused,
}

/// Caller-owned identity and invitation material for one new connection.
pub struct Credentials<'a, I: IdentityProvider> {
    /// Existing Personae custody provider; no key is persisted here.
    pub identity: &'a I,
    /// Invited network.
    pub network: NetworkId,
    /// Requested profile.
    pub profile: ProfileRef,
    /// Fresh host-generated admission nonce.
    pub nonce: [u8; 32],
    /// Explicit invitation chain, used only inside the encrypted channel.
    pub delegations: Vec<SignedDelegationCertificate>,
}

/// Outbound channel after the trusted server key and admission reply verified.
pub struct Connected {
    /// Admitted stream, ready for the selected CapTP handshake.
    pub stream: PreparedStream<P2pandaStream>,
    /// Native stream binding retained for diagnostics/owner policy.
    pub secure_channel: SecureChannelFacts,
    /// The responder's admission decision; it is not a new identity credential.
    pub reply: SessionReply,
}

/// One service's shared setup capacity and fixed native-security policy.
#[derive(Clone, Debug)]
pub struct NativeService {
    protocol: Alpn,
    action: RequestedAction,
    gate: SetupGate,
    limits: SetupLimits,
}

/// A bounded route registered by this service on the host's existing endpoint.
/// Constructed only by `NativeService::register`; acceptance cannot select an
/// unrelated ordinary/unbounded ALPN queue.
pub struct NativeListener<'a> {
    service: NativeService,
    carrier: &'a P2pandaTransport,
    ingress: BoundedAcceptHandle,
}

impl NativeListener<'_> {
    /// Pending first-stream and queued connection observations.
    pub fn ingress_stats(&self) -> BoundedAcceptStats {
        self.ingress.stats()
    }

    /// Admit one bounded incoming channel before CapTP begins.
    /// The trusted clock is read after dequeue. The host supplies its honest
    /// active-session count and must enforce serving capacity atomically.
    pub async fn accept(
        &self,
        policy: &LocalNetworkPolicy,
        ledger: &RevocationLedger,
        clock: impl Fn() -> u64,
        active_sessions: u32,
    ) -> Result<NativeAdmitted, Error> {
        self.service
            .accept_bound(self.carrier, policy, ledger, clock, active_sessions)
            .await
    }
}

impl NativeService {
    /// Configure a service before registering its bounded route on the existing endpoint.
    pub fn new(
        protocol: Alpn,
        action: RequestedAction,
        max_pending: usize,
        limits: SetupLimits,
    ) -> Self {
        Self {
            protocol,
            action,
            gate: SetupGate::new(max_pending),
            limits,
        }
    }

    /// The initial Gemot adapter's connect action.
    pub fn gemot(max_pending: usize, limits: SetupLimits) -> Self {
        Self::new(
            Alpn::from_bytes(GEMOT_ALPN),
            RequestedAction {
                domain: GEMOT_DOMAIN.into(),
                path: GEMOT_SERVICE.into(),
                action: CONNECT.into(),
            },
            max_pending,
            limits,
        )
    }

    /// Protocol of this service; use `register` for its incoming route.
    pub fn protocol(&self) -> &Alpn {
        &self.protocol
    }
    /// Counter-only setup observations shared by cloned service handles.
    pub fn stats(&self) -> SetupStats {
        self.gate.stats()
    }

    /// Register a bounded route on an existing endpoint. Do not also put this
    /// service in the builder's ordinary `alpns` list. Other routes are unchanged.
    pub async fn register<'a>(
        &self,
        carrier: &'a P2pandaTransport,
        capacity: usize,
        first_stream_timeout: Duration,
    ) -> Result<NativeListener<'a>, Error> {
        let ingress = carrier
            .register_bounded_alpn(
                self.protocol.clone(),
                capacity,
                first_stream_timeout.min(self.limits.timeout),
            )
            .await?;
        Ok(NativeListener {
            service: self.clone(),
            carrier,
            ingress,
        })
    }

    /// Accept and admit before handing a stream to the CapTP runtime.
    ///
    /// Waiting for an incoming stream is ordinary passive service listening.
    /// The registered ingress bound covers first-stream waiting and queueing.
    /// The setup deadline is reduced by that actual carrier-observed wait.
    async fn accept_bound(
        &self,
        carrier: &P2pandaTransport,
        policy: &LocalNetworkPolicy,
        ledger: &RevocationLedger,
        clock: impl Fn() -> u64,
        active_sessions: u32,
    ) -> Result<NativeAdmitted, Error> {
        let (accepted, arrived_at) = carrier.accept_observed(self.protocol.clone()).await?;
        let (stream, facts) = accepted.into_session();
        let mut limits = self.limits.clone();
        limits.timeout = limits
            .timeout
            .checked_sub(arrived_at.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(SetupError::Deadline {
                stage: transport::security::SetupStage::Establishment,
            })?;
        let security = ServiceSecurityPolicy::native(self.protocol.clone(), None, limits);
        let (stream, evidence, guard) = self.gate.reserve(&security)?.native(stream)?;
        let channel = evidence
            .secure_channel_facts()
            .ok_or(SetupError::Security)?;
        let session = match guard
            .run(admit_session_secure(
                stream,
                policy,
                ledger,
                &facts,
                &channel,
                SecureIdentityPolicy::BoundChannelApplicationProof,
                clock(),
                active_sessions,
            ))
            .await??
        {
            Ok(session) => session,
            Err(reason) => {
                guard.refused();
                return Err(Error::Refused(reason));
            },
        };
        if session.session.principal.action != self.action {
            guard.refused();
            return Err(Error::ActionMismatch);
        }
        guard.admitted()?;
        Ok(session)
    }

    /// Connect to the trusted endpoint and issue a fresh stream-bound v2 hello.
    /// No plaintext or Noise fallback is attempted.
    pub async fn connect<I: IdentityProvider>(
        &self,
        carrier: &P2pandaTransport,
        expected_server: PeerID,
        credentials: Credentials<'_, I>,
    ) -> Result<Connected, Error> {
        let security = ServiceSecurityPolicy::native(
            self.protocol.clone(),
            Some(expected_server),
            self.limits.clone(),
        );
        let guard = self.gate.reserve(&security)?;
        let outbound = guard
            .run(carrier.connect_session(expected_server, self.protocol.clone()))
            .await??;
        let binding = outbound.proof_binding();
        let (mut stream, evidence, guard) = guard.native(outbound.stream)?;
        let channel = evidence
            .secure_channel_facts()
            .ok_or(SetupError::Security)?;
        let hello = SessionHello::issue_secure(
            credentials.identity,
            credentials.network,
            credentials.profile,
            self.action.clone(),
            TrafficClass::Interactive,
            credentials.nonce,
            &binding.with_secure_channel(channel.clone()),
            credentials.delegations,
        )?;
        let reply = guard
            .run(initiate_session(&mut stream, &hello, &Default::default()))
            .await??;
        if !reply.is_accept() {
            guard.refused();
            return Err(Error::RemoteRefused);
        }
        guard.admitted()?;
        Ok(Connected {
            stream,
            secure_channel: channel,
            reply,
        })
    }
}
