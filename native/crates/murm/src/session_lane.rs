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
//! ## What admission buys
//!
//! [`serve_session`] runs the bounded handshake before anything else. A
//! refused peer gets a well-formed `DenyReason`, its stream is finished, and
//! `ingest_post` is never reached — the engine does not see a byte. An
//! admitted peer's [`AdmittedPrincipal`] is returned alongside the count of
//! what it sent, so a caller can attribute traffic to the subject the proof
//! established rather than to whatever the frames claim.

use std::sync::Arc;

use notochord::{
    AdmittedPrincipal, DenyReason, FrameError, LocalNetworkPolicy, ProofBinding, RevocationLedger,
    SessionFacts, SessionHello, admit_session, initiate_session, read_frame_or_eof, write_frame,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use transport::AcceptedSession;

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

/// Everything the owner's rule needs to decide one session.
///
/// Bundled because these five always travel together, and a service that
/// serves many sessions builds it once per accept loop rather than threading
/// five arguments through every call.
pub struct Admission<'a> {
    /// The owner's local policy.
    pub policy: &'a LocalNetworkPolicy,
    /// Revocations this node has folded.
    pub ledger: &'a RevocationLedger,
    /// What the carrier observed about this session.
    pub facts: &'a SessionFacts,
    /// The caller's clock, in milliseconds.
    pub now_ms: u64,
    /// Live sessions already admitted under this action's rule.
    pub active_sessions: u32,
}

/// Serve one inbound session: admit it, then ingest the posts it sends.
///
/// Returns `Err(reason)` when the owner's rule refuses the peer, having
/// already written the refusal and finished the stream. The engine is not
/// touched on that path.
pub async fn serve_session<S>(
    stream: S,
    engine: &Arc<ConversationEngine>,
    cabal_id: [u8; 32],
    admission: Admission<'_>,
) -> Result<Result<SessionOutcome, DenyReason>, MurmError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let admitted = admit_session(
        stream,
        admission.policy,
        admission.ledger,
        admission.facts,
        admission.now_ms,
        admission.active_sessions,
    )
    .await
    .map_err(|e| MurmError::Backend(e.to_string()))?;
    let mut admitted = match admitted {
        Ok(session) => session,
        Err(reason) => return Ok(Err(reason)),
    };

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

/// Serve an inbound transport session using only facts recorded at acceptance.
///
/// This is the application-facing listener seam. The transport's
/// [`AcceptedSession`] is consumed at the boundary, and its protocol,
/// authenticated peer, and ingress context are converted through the one
/// audited `mere-transport` adapter before [`serve_session`] reads the
/// Notochord hello. A caller cannot accidentally substitute claims decoded
/// from that hello for carrier facts.
pub async fn serve_accepted_session<S>(
    accepted: AcceptedSession<S>,
    engine: &Arc<ConversationEngine>,
    cabal_id: [u8; 32],
    policy: &LocalNetworkPolicy,
    ledger: &RevocationLedger,
    now_ms: u64,
    active_sessions: u32,
) -> Result<Result<SessionOutcome, DenyReason>, MurmError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (stream, facts) = accepted.into_session();
    serve_session(
        stream,
        engine,
        cabal_id,
        Admission {
            policy,
            ledger,
            facts: &facts,
            now_ms,
            active_sessions,
        },
    )
    .await
}

/// Open a session as the initiator: prove the subject, then push `posts`.
///
/// The stream is finished when the posts are away, so the peer sees a clean
/// end rather than waiting on a sender that has nothing more to say.
pub async fn push_posts<S>(
    mut stream: S,
    hello: &SessionHello,
    policy: &LocalNetworkPolicy,
    posts: &[Post],
) -> Result<Result<usize, DenyReason>, MurmError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let reply = initiate_session(&mut stream, hello, &policy.limits)
        .await
        .map_err(|e| MurmError::Backend(e.to_string()))?;
    match reply {
        notochord::SessionReply::Reject { reason } => return Ok(Err(reason)),
        notochord::SessionReply::Accept { .. } => {},
    }
    for post in posts {
        write_frame(&mut stream, &encode_post(post), MAX_POST_FRAME)
            .await
            .map_err(framing)?;
    }
    let _ = stream.shutdown().await;
    Ok(Ok(posts.len()))
}

/// The binding an initiator signs for a session lane on an authenticating
/// carrier. Re-exported shape so callers do not hand-build one.
pub fn lane_binding(protocol: &[u8], local_identity: [u8; 32]) -> ProofBinding {
    ProofBinding::initiator(protocol.to_vec(), Some(local_identity), None)
}
