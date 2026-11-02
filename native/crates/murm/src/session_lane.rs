// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Murm over a point-to-point session, gated by the owner's policy.
//!
//! Notochord N2, Murm half. Murm's ordinary peer runtime is topic-shaped:
//! posts arrive on a gossip overlay and gaps are reconciled by LogSync. Over a
//! radio bearer there is no overlay to ride — a Reticulum link is two peers and
//! a stream — so this is the lane that lets a cabal move at all out there, and
//! it is also where an owner's rule gets to refuse a peer before Murm sees a
//! byte of application traffic.
//!
//! It carries the same posts the gossip lane does, through the same
//! [`ConversationEngine::ingest_post`], which verifies the signature, the
//! self-describing cabal id, and the per-author log rule, and is idempotent.
//! A post arriving here and on gossip lands once.
//!
//! ## Setup
//!
//! Every session reserves from its [`SessionLane`]'s shared gate before
//! security runs and keeps that lease, with one deadline and fixed byte
//! budgets, through admission (consolidation T6). The owner picks the profile
//! by the method it calls: native iroh security on p2panda, Noise on any other
//! ordered carrier. A profile that fails is a refusal; nothing retries a
//! weaker one. Admission is Notochord's secure v2, so the subject signs this
//! exact channel and a hello lifted from another channel does not verify.
//!
//! ## What admission buys
//!
//! A refused peer gets a well-formed `DenyReason`, its stream is finished, and
//! `ingest_post` is never reached — the engine does not see a byte. An
//! admitted peer's [`AdmittedPrincipal`] is returned alongside the count of
//! what it sent, so a caller can attribute traffic to the subject the proof
//! established rather than to whatever the frames claim.

use std::sync::Arc;

use identity::{Ed25519Keypair, IdentityProvider};
use insigne::delegation::SignedDelegationCertificate;
use notochord::{
    AdmittedPrincipal, DenyReason, FrameError, LocalNetworkPolicy, NetworkId, ProfileRef,
    ProofBinding, RequestedAction, RevocationLedger, SecureIdentityPolicy, SessionFacts,
    SessionHello, SessionReply, TrafficClass, admit_session_secure, initiate_session,
    read_frame_or_eof, write_frame,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use transport::security::{
    PreparedStream, ServiceSecurityPolicy, SessionSecurityFacts, SetupError, SetupGate, SetupGuard,
    SetupLimits, SetupStats,
};
use transport::{AcceptedSession, Alpn, P2pandaStream, P2pandaTransport, PeerID, Transport};

use crate::conversation_engine::ConversationEngine;
use crate::error::MurmError;
use crate::post::Post;
use crate::post_wire::{decode_post, encode_post};

/// Largest post frame this lane will read.
///
/// Bounded before the body is read, so a hostile peer cannot make the reader
/// allocate. Generous next to a text post and far under a link MTU's worth of
/// round trips; a cabal that needs more than this wants the resource lane, not
/// a post frame.
pub const MAX_POST_FRAME: u32 = 262_144;

/// What one served session did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOutcome {
    /// Who the proof established, and under what action.
    pub principal: AdmittedPrincipal,
    /// Posts this peer sent that the engine accepted.
    pub posts_ingested: u64,
    /// Frames that decoded but the engine rejected: a bad signature, a foreign
    /// cabal id, a log-rule violation, or a duplicate. Counted rather than
    /// fatal, because one bad post from an admitted peer is not grounds to
    /// tear down a link that is otherwise behaving.
    pub posts_rejected: u64,
}

/// Frame errors reach Murm's error type here rather than through a bare `?`,
/// so a crossed bound still reports which one and by how much.
///
/// The framing itself now comes from `notochord`, which had the same
/// length-prefixed format private in its handshake module while this lane
/// carried a second copy. [`MAX_POST_FRAME`] stays here: it is Murm's payload
/// bound, not handshake attack surface, so it is passed in rather than read
/// out of `HandshakeLimits`.
fn framing(error: FrameError) -> MurmError {
    MurmError::Backend(error.to_string())
}

fn backend(error: impl std::fmt::Display) -> MurmError {
    MurmError::Backend(error.to_string())
}

/// Everything the owner's rule needs to decide one session.
///
/// Bundled because these always travel together, and a service that serves
/// many sessions builds it once per accept loop rather than threading five
/// arguments through every call. Carrier facts are not here: they come off
/// the accepted session, never from the caller.
pub struct Admission<'a> {
    /// The owner's local policy.
    pub policy: &'a LocalNetworkPolicy,
    /// Revocations this node has folded.
    pub ledger: &'a RevocationLedger,
    /// How the channel's proved key must relate to the hello's subject. The
    /// owner selects it; nothing a peer sends can.
    pub identity: SecureIdentityPolicy,
    /// The caller's clock, in milliseconds.
    pub now_ms: u64,
    /// Live sessions already admitted under this action's rule.
    pub active_sessions: u32,
}

/// Who is pushing, and under what invitation.
///
/// The lane issues the hello once the channel exists, because a v2 proof signs
/// that channel's binding; a hello made in advance would have nothing to sign.
pub struct Credentials<'a, I: IdentityProvider> {
    /// The subject's custody provider; no key is kept here.
    pub identity: &'a I,
    /// The invited network.
    pub network: NetworkId,
    /// The requested profile.
    pub profile: ProfileRef,
    /// The service action asked for.
    pub action: RequestedAction,
    /// The traffic class asked for.
    pub class: TrafficClass,
    /// A fresh admission nonce.
    pub nonce: [u8; 32],
    /// The invitation chain, sent only inside the secured channel.
    pub delegations: Vec<SignedDelegationCertificate>,
}

/// What the gate hands back: the bounded stream, the channel's verified facts,
/// and the lease that lasts through admission.
type Secured<S> = (PreparedStream<S>, SessionSecurityFacts, SetupGuard);

/// One lane service: its protocol, setup limits and shared setup gate.
///
/// Keep one per service and clone it into accept loops. Clones share the gate,
/// so sessions still in setup on every carrier meet one ceiling.
#[derive(Clone, Debug)]
pub struct SessionLane {
    protocol: Alpn,
    gate: SetupGate,
    limits: SetupLimits,
}

impl SessionLane {
    /// A lane for `protocol` with at most `max_pending` sessions in setup.
    pub fn new(protocol: Alpn, max_pending: usize, limits: SetupLimits) -> Self {
        Self {
            protocol,
            gate: SetupGate::new(max_pending),
            limits,
        }
    }

    /// Counter-only setup observations, shared by every clone.
    pub fn stats(&self) -> SetupStats {
        self.gate.stats()
    }

    /// Serve an accepted p2panda session through native iroh security.
    ///
    /// Returns `Ok(Err(reason))` when the owner's rule refuses the peer, having
    /// already written the refusal and finished the stream. The engine is not
    /// touched on that path.
    pub async fn serve_native(
        &self,
        accepted: AcceptedSession<P2pandaStream>,
        engine: &Arc<ConversationEngine>,
        cabal_id: [u8; 32],
        admission: Admission<'_>,
    ) -> Result<Result<SessionOutcome, DenyReason>, MurmError> {
        let (stream, facts) = accepted.into_session();
        let security =
            ServiceSecurityPolicy::native(self.protocol.clone(), None, self.limits.clone());
        let secured = self
            .gate
            .reserve(&security)
            .and_then(|guard| guard.native(stream))
            .map_err(backend)?;
        serve(secured, &facts, engine, cabal_id, admission).await
    }

    /// Serve an accepted session on any ordered carrier through Noise, proving
    /// `key` as this end.
    pub async fn serve_noise<S>(
        &self,
        key: &Ed25519Keypair,
        accepted: AcceptedSession<S>,
        engine: &Arc<ConversationEngine>,
        cabal_id: [u8; 32],
        admission: Admission<'_>,
    ) -> Result<Result<SessionOutcome, DenyReason>, MurmError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let (stream, facts) = accepted.into_session();
        let security =
            ServiceSecurityPolicy::noise(self.protocol.clone(), None, self.limits.clone());
        let secured = self
            .gate
            .noise_responder(key, stream, &security)
            .await
            .map_err(backend)?;
        serve(secured, &facts, engine, cabal_id, admission).await
    }

    /// Dial `server` over p2panda, require that endpoint, then push `posts`.
    ///
    /// The dial runs under the setup deadline. The stream is finished when the
    /// posts are away, so the peer sees a clean end.
    pub async fn push_native<I: IdentityProvider>(
        &self,
        carrier: &P2pandaTransport,
        server: PeerID,
        credentials: Credentials<'_, I>,
        posts: &[Post],
    ) -> Result<Result<usize, DenyReason>, MurmError> {
        let security =
            ServiceSecurityPolicy::native(self.protocol.clone(), Some(server), self.limits.clone());
        let guard = self.gate.reserve(&security).map_err(backend)?;
        let outbound = guard
            .run(carrier.connect_session(server, self.protocol.clone()))
            .await
            .map_err(backend)??;
        let binding = outbound.proof_binding();
        let secured = guard.native(outbound.stream).map_err(backend)?;
        push(secured, binding, credentials, posts).await
    }

    /// Dial `server` on any carrier, run Noise as `key` and require the
    /// responder to prove `expected`, then push `posts`.
    ///
    /// `expected` comes from the invitation, not the carrier: on Reticulum the
    /// destination identity and the responder's Noise key are different keys.
    pub async fn push_noise<T: Transport, I: IdentityProvider>(
        &self,
        carrier: &T,
        server: PeerID,
        key: &Ed25519Keypair,
        expected: PeerID,
        credentials: Credentials<'_, I>,
        posts: &[Post],
    ) -> Result<Result<usize, DenyReason>, MurmError> {
        let security = ServiceSecurityPolicy::noise(
            self.protocol.clone(),
            Some(expected),
            self.limits.clone(),
        );
        let guard = self.gate.reserve(&security).map_err(backend)?;
        let outbound = guard
            .run(carrier.connect_session(server, self.protocol.clone()))
            .await
            .map_err(backend)??;
        let binding = outbound.proof_binding();
        let secured = guard
            .noise_initiator(key, outbound.stream)
            .await
            .map_err(backend)?;
        push(secured, binding, credentials, posts).await
    }
}

/// Admit a secured session under its lease, then ingest the posts it sends.
async fn serve<S>(
    (stream, security, guard): Secured<S>,
    facts: &SessionFacts,
    engine: &Arc<ConversationEngine>,
    cabal_id: [u8; 32],
    admission: Admission<'_>,
) -> Result<Result<SessionOutcome, DenyReason>, MurmError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let channel = security
        .secure_channel_facts()
        .ok_or_else(|| backend(SetupError::Security))?;
    let admitted = guard
        .run(admit_session_secure(
            stream,
            admission.policy,
            admission.ledger,
            facts,
            &channel,
            admission.identity,
            admission.now_ms,
            admission.active_sessions,
        ))
        .await
        .map_err(backend)?
        .map_err(backend)?;
    let mut admitted = match admitted {
        Ok(session) => session.into_session(),
        Err(reason) => {
            guard.refused();
            return Ok(Err(reason));
        },
    };
    guard.admitted().map_err(backend)?;

    let mut outcome = SessionOutcome {
        principal: admitted.principal.clone(),
        posts_ingested: 0,
        posts_rejected: 0,
    };
    while let Some(frame) = read_frame_or_eof(&mut admitted.stream, MAX_POST_FRAME)
        .await
        .map_err(framing)?
    {
        match decode_post(&frame) {
            Ok(post) => {
                if engine.ingest_post(&cabal_id, post).await.is_ok() {
                    outcome.posts_ingested += 1;
                } else {
                    outcome.posts_rejected += 1;
                }
            },
            Err(_) => outcome.posts_rejected += 1,
        }
    }
    Ok(Ok(outcome))
}

/// Prove the subject on this exact channel under its lease, then send `posts`.
async fn push<S, I: IdentityProvider>(
    (mut stream, security, guard): Secured<S>,
    carrier: ProofBinding,
    credentials: Credentials<'_, I>,
    posts: &[Post],
) -> Result<Result<usize, DenyReason>, MurmError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let channel = security
        .secure_channel_facts()
        .ok_or_else(|| backend(SetupError::Security))?;
    let hello = SessionHello::issue_secure(
        credentials.identity,
        credentials.network,
        credentials.profile,
        credentials.action,
        credentials.class,
        credentials.nonce,
        &carrier.with_secure_channel(channel),
        credentials.delegations,
    )
    .map_err(backend)?;
    let reply = guard
        .run(initiate_session(&mut stream, &hello, &Default::default()))
        .await
        .map_err(backend)?
        .map_err(backend)?;
    if let SessionReply::Reject { reason } = reply {
        guard.refused();
        return Ok(Err(reason));
    }
    guard.admitted().map_err(backend)?;
    for post in posts {
        write_frame(&mut stream, &encode_post(post), MAX_POST_FRAME)
            .await
            .map_err(framing)?;
    }
    let _ = stream.shutdown().await;
    Ok(Ok(posts.len()))
}
