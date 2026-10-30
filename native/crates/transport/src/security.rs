// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Exact owner-selected security profiles and bounded setup leases.
//!
//! Reserve once from a service's shared gate, then retain the guard through
//! connection establishment, security and application admission. Nothing here
//! negotiates or retries a weaker profile. Only `admitted` lifts setup limits;
//! dropping the guard poisons its stream and returns its capacity.

mod io;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use identity::Ed25519Keypair;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, timeout_at};

use self::io::{BudgetIo, FrameFormat, Usage};

/// Hard service gate ceiling, independent of caller configuration.
pub const MAX_PENDING_SETUPS: usize = 256;
/// Hard cumulative encoded setup bytes per direction.
pub const MAX_SETUP_BYTES: usize = 1024 * 1024;
/// Hard cumulative encoded setup frame count.
pub const MAX_SETUP_FRAMES: usize = 4096;
/// Hard cumulative setup I/O poll count.
pub const MAX_SETUP_IO_OPERATIONS: usize = 16384;
use crate::noise::{
    NoiseSessionFacts, NoiseStream, handshake_limited, read_len_prefixed, write_len_prefixed,
};
use crate::{Alpn, PeerID};

/// A mode chosen by the service owner before any untrusted bytes are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityMode {
    /// The internal Noise XX/Ed25519 profile.
    Noise,
    /// Backend-proved native confidentiality, authentication and channel binding.
    Native,
    /// Explicitly public, unauthenticated plaintext; unsuitable for bearer secrets.
    PublicPlaintext,
}

/// Maximum work and storage exposed by one setup, including admission.
///
/// Frame size bounds each encoded body before allocation; byte/frame and I/O
/// operation budgets are cumulative across security and admission. Noise has
/// exactly three handshake steps and one remote Ed25519 proof verification.
#[derive(Clone, Debug)]
pub struct SetupLimits {
    /// One absolute deadline, starting at reservation, including establishment.
    pub timeout: Duration,
    /// Maximum encoded bytes read, including prefixes.
    pub max_read_bytes: usize,
    /// Maximum encoded bytes written, including prefixes.
    pub max_write_bytes: usize,
    /// Maximum encoded frame body, before allocating it.
    pub max_frame_bytes: usize,
    /// Maximum read and written frames combined.
    pub max_frames: usize,
    /// Maximum I/O polls combined, including pending polls.
    pub max_io_operations: usize,
}

impl Default for SetupLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_read_bytes: 16 * 1024,
            max_write_bytes: 16 * 1024,
            max_frame_bytes: 4096,
            max_frames: 24,
            max_io_operations: 512,
        }
    }
}

/// One service's immutable selection and peer expectations.
#[derive(Clone)]
pub struct ServiceSecurityPolicy {
    mode: SecurityMode,
    protocol: Alpn,
    expected_peer: Option<PeerID>,
    limits: SetupLimits,
}

impl ServiceSecurityPolicy {
    /// Require Noise; its failure can never select native security or plaintext.
    pub fn noise(protocol: Alpn, expected_peer: Option<PeerID>, limits: SetupLimits) -> Self {
        Self {
            mode: SecurityMode::Noise,
            protocol,
            expected_peer,
            limits,
        }
    }
    /// Permit the native backend's exact authenticated channel profile.
    pub fn native(protocol: Alpn, expected_peer: Option<PeerID>, limits: SetupLimits) -> Self {
        Self {
            mode: SecurityMode::Native,
            protocol,
            expected_peer,
            limits,
        }
    }
    /// Explicitly allow public plaintext. No identity or confidentiality is claimed.
    pub fn public_plaintext(protocol: Alpn, limits: SetupLimits) -> Self {
        Self {
            mode: SecurityMode::PublicPlaintext,
            protocol,
            expected_peer: None,
            limits,
        }
    }
    /// Selected mode; there is no opportunistic downgrade.
    pub fn mode(&self) -> SecurityMode {
        self.mode
    }
    /// Independently expected inner application protocol.
    pub fn protocol(&self) -> &Alpn {
        &self.protocol
    }
    /// Fixed setup limits.
    pub fn limits(&self) -> &SetupLimits {
        &self.limits
    }

    fn valid(&self) -> bool {
        !self.protocol.is_empty()
            && self.protocol.len() <= 255
            && !self.limits.timeout.is_zero()
            && self.limits.timeout <= Duration::from_secs(300)
            && (128..=65535).contains(&self.limits.max_frame_bytes)
            && self.limits.max_read_bytes >= 2
            && self.limits.max_read_bytes <= MAX_SETUP_BYTES
            && self.limits.max_write_bytes >= 2
            && self.limits.max_write_bytes <= MAX_SETUP_BYTES
            && self.limits.max_frames > 0
            && self.limits.max_frames <= MAX_SETUP_FRAMES
            && self.limits.max_io_operations > 0
            && self.limits.max_io_operations <= MAX_SETUP_IO_OPERATIONS
    }
}

/// Stage of the setup lifecycle; diagnostics contain no identities or secrets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStage {
    /// Opening the carrier.
    Establishment,
    /// Authenticating the selected channel.
    Security,
    /// Waiting for the application's admission decision.
    Admission,
}

/// A bounded setup refusal. Backend detail is intentionally not logged here.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// Invalid owner configuration.
    #[error("invalid setup policy or limits")]
    InvalidPolicy,
    /// The caller invoked a mode the owner did not select.
    #[error("setup security mode mismatch")]
    ModeMismatch,
    /// No capacity is available on the shared service gate.
    #[error("setup capacity exhausted")]
    Capacity,
    /// The single setup deadline expired.
    #[error("setup deadline at {stage:?}")]
    Deadline {
        /// Stage that expired.
        stage: SetupStage,
    },
    /// A peer, protocol or backend channel was not the configured one.
    #[error("setup peer or protocol mismatch")]
    PeerOrProtocolMismatch,
    /// Malformed/tampered/truncated/budget-exceeding channel setup.
    #[error("secure setup failed")]
    Security,
    /// A guard already finished or was cancelled.
    #[error("setup is closed")]
    Closed,
}

/// Counter-only observation of a service gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetupStats {
    /// Established carrier streams entering security.
    pub established: usize,
    /// Authenticated confidential channels that passed the exact security policy.
    pub secured: usize,
    /// Application admissions explicitly completed by their owner.
    pub admitted: usize,
    /// Explicit refusals in any stage.
    pub refused: usize,
    /// Explicit refusals before carrier establishment completed.
    pub establishment_refused: usize,
    /// Explicit refusals while authenticating the selected channel.
    pub security_refused: usize,
    /// Explicit refusals by the application admission owner.
    pub admission_refused: usize,
    /// Expired absolute deadlines.
    pub timed_out: usize,
    /// Guards dropped before a terminal decision.
    pub cancelled: usize,
    /// Attempts refused before reserving capacity.
    pub capacity_refused: usize,
    /// Currently retained setup/admission leases.
    pub pending: usize,
    /// Maximum simultaneously retained leases.
    pub peak_pending: usize,
}

#[derive(Default)]
struct Counters {
    established: AtomicUsize,
    secured: AtomicUsize,
    admitted: AtomicUsize,
    refused: AtomicUsize,
    establishment_refused: AtomicUsize,
    security_refused: AtomicUsize,
    admission_refused: AtomicUsize,
    timed_out: AtomicUsize,
    cancelled: AtomicUsize,
    capacity_refused: AtomicUsize,
    peak_pending: AtomicUsize,
}

/// Shared service capacity. Store this with the service; do not recreate per call.
#[derive(Clone)]
pub struct SetupGate {
    permits: Arc<Semaphore>,
    limit: usize,
    counters: Arc<Counters>,
}

impl SetupGate {
    /// Create a service gate with a fixed pending-setup/admission ceiling.
    pub fn new(max_pending: usize) -> Self {
        let max_pending = max_pending.min(MAX_PENDING_SETUPS);
        Self {
            permits: Arc::new(Semaphore::new(max_pending)),
            limit: max_pending,
            counters: Arc::new(Counters::default()),
        }
    }
    /// Currently retained setup/admission leases.
    pub fn pending(&self) -> usize {
        self.limit - self.permits.available_permits()
    }
    /// Read counter-only diagnostics.
    pub fn stats(&self) -> SetupStats {
        let c = &self.counters;
        SetupStats {
            established: c.established.load(Ordering::Relaxed),
            secured: c.secured.load(Ordering::Relaxed),
            admitted: c.admitted.load(Ordering::Relaxed),
            refused: c.refused.load(Ordering::Relaxed),
            establishment_refused: c.establishment_refused.load(Ordering::Relaxed),
            security_refused: c.security_refused.load(Ordering::Relaxed),
            admission_refused: c.admission_refused.load(Ordering::Relaxed),
            timed_out: c.timed_out.load(Ordering::Relaxed),
            cancelled: c.cancelled.load(Ordering::Relaxed),
            capacity_refused: c.capacity_refused.load(Ordering::Relaxed),
            pending: self.pending(),
            peak_pending: c.peak_pending.load(Ordering::Relaxed),
        }
    }
    /// Reserve before connection establishment, retaining this guard through admission.
    pub fn reserve(&self, policy: &ServiceSecurityPolicy) -> Result<SetupGuard, SetupError> {
        if !policy.valid() {
            return Err(SetupError::InvalidPolicy);
        }
        let permit = self.permits.clone().try_acquire_owned().map_err(|_| {
            self.counters
                .capacity_refused
                .fetch_add(1, Ordering::Relaxed);
            SetupError::Capacity
        })?;
        self.counters
            .peak_pending
            .fetch_max(self.pending(), Ordering::Relaxed);
        Ok(SetupGuard {
            policy: policy.clone(),
            state: Arc::new(Lease {
                permit: Mutex::new(Some(permit)),
                status: AtomicU8::new(0),
                stage: AtomicU8::new(0),
                deadline: Instant::now() + policy.limits.timeout,
                limits: policy.limits.clone(),
                usage: Mutex::new(Usage::default()),
                counters: self.counters.clone(),
            }),
        })
    }
    /// Secure an already-established stream. Reserve first to include connect time.
    pub async fn noise_initiator<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        identity: &Ed25519Keypair,
        inner: S,
        policy: &ServiceSecurityPolicy,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, SetupGuard), SetupError> {
        self.reserve(policy)?.noise_initiator(identity, inner).await
    }
    /// Responder setup; caller retains the returned guard until admission completes.
    pub async fn noise_responder<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        identity: &Ed25519Keypair,
        inner: S,
        policy: &ServiceSecurityPolicy,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, SetupGuard), SetupError> {
        self.reserve(policy)?.noise_responder(identity, inner).await
    }
}

struct Lease {
    permit: Mutex<Option<OwnedSemaphorePermit>>,
    status: AtomicU8,
    stage: AtomicU8,
    deadline: Instant,
    limits: SetupLimits,
    usage: Mutex<Usage>,
    counters: Arc<Counters>,
}

impl Lease {
    fn stage(&self) -> SetupStage {
        match self.stage.load(Ordering::Relaxed) {
            0 => SetupStage::Establishment,
            1 => SetupStage::Security,
            _ => SetupStage::Admission,
        }
    }
    fn close(&self, outcome: u8) {
        if self
            .status
            .compare_exchange(0, outcome, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let counter = match outcome {
                1 => &self.counters.admitted,
                2 => &self.counters.refused,
                3 => &self.counters.timed_out,
                _ => &self.counters.cancelled,
            };
            counter.fetch_add(1, Ordering::Relaxed);
            if outcome == 2 {
                let stage_counter = match self.stage() {
                    SetupStage::Establishment => &self.counters.establishment_refused,
                    SetupStage::Security => &self.counters.security_refused,
                    SetupStage::Admission => &self.counters.admission_refused,
                };
                stage_counter.fetch_add(1, Ordering::Relaxed);
            }
            // Terminal decisions release capacity even if the caller retains
            // its poisoned guard or stream for diagnostics.
            self.permit.lock().unwrap_or_else(|p| p.into_inner()).take();
        }
    }
}

/// RAII setup capacity/deadline, held through the application's admission decision.
pub struct SetupGuard {
    policy: ServiceSecurityPolicy,
    state: Arc<Lease>,
}

/// Armed only while an admission/establishment future is being polled. Dropping
/// that future must poison partial framing even when its guard remains alive.
struct RunCancellation {
    lease: Arc<Lease>,
    armed: bool,
}
impl Drop for RunCancellation {
    fn drop(&mut self) {
        if self.armed {
            self.lease.close(4);
        }
    }
}

impl SetupGuard {
    /// Run establishment or admission under the same absolute deadline.
    pub async fn run<T>(&self, future: impl Future<Output = T>) -> Result<T, SetupError> {
        if self.state.status.load(Ordering::Acquire) != 0 {
            return Err(SetupError::Closed);
        }
        let mut cancellation = RunCancellation {
            lease: self.state.clone(),
            armed: true,
        };
        let result = match timeout_at(self.state.deadline, future).await {
            Ok(value) => match self.state.status.load(Ordering::Acquire) {
                0 if Instant::now() < self.state.deadline => Ok(value),
                0 | 3 => {
                    self.state.close(3);
                    Err(SetupError::Deadline {
                        stage: self.state.stage(),
                    })
                },
                2 => Err(SetupError::Security),
                _ => Err(SetupError::Closed),
            },
            Err(_) => {
                self.state.close(3);
                Err(SetupError::Deadline {
                    stage: self.state.stage(),
                })
            },
        };
        cancellation.armed = false;
        result
    }
    /// Complete admission, lift setup I/O limits and release the lease.
    pub fn admitted(self) -> Result<(), SetupError> {
        if self.state.status.load(Ordering::Acquire) != 0 {
            return Err(SetupError::Closed);
        }
        if self.state.stage.load(Ordering::Acquire) != 2 {
            self.state.close(2);
            return Err(SetupError::Closed);
        }
        if Instant::now() >= self.state.deadline {
            self.state.close(3);
            return Err(SetupError::Deadline {
                stage: self.state.stage(),
            });
        }
        self.state.close(1);
        Ok(())
    }
    /// Record an explicit application admission refusal and poison setup I/O.
    pub fn refused(self) {
        self.state.close(2);
    }

    fn begin(&self, mode: SecurityMode) -> Result<(), SetupError> {
        if self.state.status.load(Ordering::Acquire) != 0 {
            return Err(SetupError::Closed);
        }
        if self.state.stage.load(Ordering::Acquire) != 0 {
            self.state.close(2);
            return Err(SetupError::Closed);
        }
        if Instant::now() >= self.state.deadline {
            self.state.close(3);
            return Err(SetupError::Deadline {
                stage: self.state.stage(),
            });
        }
        self.state
            .counters
            .established
            .fetch_add(1, Ordering::Relaxed);
        self.state.stage.store(1, Ordering::Relaxed);
        if self.policy.mode != mode {
            self.state.close(2);
            return Err(SetupError::ModeMismatch);
        }
        Ok(())
    }
    fn check(&self, peer: Option<PeerID>, protocol: &Alpn) -> Result<(), SetupError> {
        if protocol != &self.policy.protocol
            || self
                .policy
                .expected_peer
                .is_some_and(|expected| Some(expected) != peer)
        {
            self.state.close(2);
            return Err(SetupError::PeerOrProtocolMismatch);
        }
        if self.policy.mode != SecurityMode::PublicPlaintext {
            self.state.counters.secured.fetch_add(1, Ordering::Relaxed);
        }
        self.state.stage.store(2, Ordering::Relaxed);
        Ok(())
    }
    /// Secure a reserved outbound stream without replacing its setup lease.
    pub async fn noise_initiator<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        identity: &Ed25519Keypair,
        inner: S,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, Self), SetupError> {
        self.noise(identity, inner, true).await
    }
    /// Secure a reserved inbound stream without replacing its setup lease.
    pub async fn noise_responder<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        identity: &Ed25519Keypair,
        inner: S,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, Self), SetupError> {
        self.noise(identity, inner, false).await
    }
    async fn noise<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        identity: &Ed25519Keypair,
        inner: S,
        initiator: bool,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, Self), SetupError> {
        self.begin(SecurityMode::Noise)?;
        let inner = BudgetIo::new(inner, self.state.clone(), FrameFormat::Noise);
        let setup = async {
            let (mut stream, handshake) = handshake_limited(
                identity,
                inner,
                initiator,
                self.policy.limits.max_frame_bytes,
            )
            .await?;
            let protocol = if initiator {
                write_len_prefixed(&mut stream, self.policy.protocol.as_bytes())
                    .await
                    .map_err(crate::TransportError::from)?;
                self.policy.protocol.clone()
            } else {
                Alpn::from_bytes(
                    read_len_prefixed(&mut stream, 255)
                        .await
                        .map_err(crate::TransportError::from)?,
                )
            };
            Ok::<_, crate::TransportError>((
                stream,
                NoiseSessionFacts::from_verified(handshake, protocol),
            ))
        };
        let (stream, facts) = match self.run(setup).await? {
            Ok(value) => value,
            Err(_) => {
                self.state.close(2);
                return Err(SetupError::Security);
            },
        };
        self.check(Some(facts.peer()), facts.protocol())?;
        Ok((
            PreparedStream::noise(stream, self.state.clone()),
            SessionSecurityFacts::Noise(facts),
            self,
        ))
    }
    /// Apply native policy using evidence produced by the actual backend stream.
    pub fn native(
        self,
        inner: crate::P2pandaStream,
    ) -> Result<
        (
            PreparedStream<crate::P2pandaStream>,
            SessionSecurityFacts,
            Self,
        ),
        SetupError,
    > {
        self.begin(SecurityMode::Native)?;
        let evidence = inner.native_channel_facts().map_err(|_| {
            self.state.close(2);
            SetupError::Security
        })?;
        self.check(Some(evidence.peer()), &evidence.protocol)?;
        let stream = BudgetIo::new(inner, self.state.clone(), FrameFormat::Application);
        Ok((
            PreparedStream::plain(stream, self.state.clone()),
            SessionSecurityFacts::Native(evidence),
            self,
        ))
    }
    /// Apply explicit public-plaintext policy; no authenticated facts are produced.
    pub fn public_plaintext<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        inner: S,
    ) -> Result<(PreparedStream<S>, SessionSecurityFacts, Self), SetupError> {
        self.begin(SecurityMode::PublicPlaintext)?;
        self.check(None, &self.policy.protocol)?;
        let stream = BudgetIo::new(inner, self.state.clone(), FrameFormat::Application);
        let protocol = self.policy.protocol.clone();
        Ok((
            PreparedStream::plain(stream, self.state.clone()),
            SessionSecurityFacts::PublicPlaintext(protocol),
            self,
        ))
    }
}

impl Drop for SetupGuard {
    fn drop(&mut self) {
        self.state.close(4);
    }
}

/// Exact iroh TLS exporter profile, scoped to one bidirectional stream.
pub const NATIVE_PROFILE: &[u8] = b"mere/iroh-tls-exporter-bistream";

/// Backend-produced native security evidence. Not constructible from wire claims.
#[derive(Clone)]
pub struct NativeChannelFacts {
    pub(crate) initiator: PeerID,
    pub(crate) responder: PeerID,
    pub(crate) protocol: Alpn,
    pub(crate) binding: [u8; 32],
    pub(crate) local_is_initiator: bool,
}
impl NativeChannelFacts {
    /// Remote backend-authenticated identity.
    pub fn peer(&self) -> PeerID {
        if self.local_is_initiator {
            self.responder
        } else {
            self.initiator
        }
    }
    /// Authenticated native protocol.
    pub fn protocol(&self) -> &Alpn {
        &self.protocol
    }
    /// Public digest of the per-stream exporter binding.
    pub fn binding(&self) -> &[u8; 32] {
        &self.binding
    }
}

/// The selected channel's verified observations; plaintext confers no peer identity.
pub enum SessionSecurityFacts {
    /// Verified internal Noise profile.
    Noise(NoiseSessionFacts),
    /// Verified backend-native security.
    Native(NativeChannelFacts),
    /// Owner-selected public plaintext protocol, unauthenticated.
    PublicPlaintext(Alpn),
}
impl SessionSecurityFacts {
    /// Remote authenticated peer, absent for plaintext.
    pub fn peer(&self) -> Option<PeerID> {
        match self {
            Self::Noise(f) => Some(f.peer()),
            Self::Native(f) => Some(f.peer()),
            Self::PublicPlaintext(_) => None,
        }
    }
    /// Application protocol; plaintext's value is owner configuration only.
    pub fn protocol(&self) -> &Alpn {
        match self {
            Self::Noise(f) => f.protocol(),
            Self::Native(f) => f.protocol(),
            Self::PublicPlaintext(p) => p,
        }
    }
    /// Shared secure evidence for v2 admission; absent for public plaintext.
    #[cfg(feature = "notochord")]
    pub fn secure_channel_facts(&self) -> Option<notochord::SecureChannelFacts> {
        match self {
            Self::Noise(f) => Some(f.secure_channel_facts()),
            Self::Native(f) => Some(notochord::SecureChannelFacts::new(
                NATIVE_PROFILE,
                1,
                f.binding.to_vec(),
                f.initiator.to_bytes(),
                f.responder.to_bytes(),
                f.protocol.as_bytes(),
            )),
            Self::PublicPlaintext(_) => None,
        }
    }
}

/// Opaque stream retaining setup budgets until its guard completes admission.
pub struct PreparedStream<S> {
    inner: StreamKind<S>,
    lease: Arc<Lease>,
}
enum StreamKind<S> {
    Noise(Box<NoiseStream<BudgetIo<S>>>),
    Plain(BudgetIo<S>),
}

impl std::fmt::Debug for SetupGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupGate")
            .field("limit", &self.limit)
            .field("pending", &self.pending())
            .finish()
    }
}
impl std::fmt::Debug for ServiceSecurityPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceSecurityPolicy")
            .field("mode", &self.mode)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for SetupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupGuard")
            .field("stage", &self.state.stage())
            .finish_non_exhaustive()
    }
}
impl<S> std::fmt::Debug for PreparedStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedStream").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for SessionSecurityFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Noise(_) => "Noise(..)",
            Self::Native(_) => "Native(..)",
            Self::PublicPlaintext(_) => "PublicPlaintext(..)",
        })
    }
}
impl std::fmt::Debug for NativeChannelFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeChannelFacts").finish_non_exhaustive()
    }
}
impl<S> PreparedStream<S> {
    fn noise(stream: NoiseStream<BudgetIo<S>>, lease: Arc<Lease>) -> Self {
        Self {
            inner: StreamKind::Noise(Box::new(stream)),
            lease,
        }
    }
    fn plain(stream: BudgetIo<S>, lease: Arc<Lease>) -> Self {
        Self {
            inner: StreamKind::Plain(stream),
            lease,
        }
    }
}
