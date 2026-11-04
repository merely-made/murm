// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Shared admitted-stream ceremony for native and SAM relay channels.
use super::{RelayError, RelayRequest};
use crate::{Admission, Credentials};
use identity::IdentityProvider;
use notochord::{
    AdmittedPrincipal, ProofBinding, SessionFacts, SessionHello, SessionReply,
    admit_session_secure, initiate_session, read_frame_or_eof, write_frame,
};
use tokio::io::{AsyncRead, AsyncWrite};
use transport::security::{PreparedStream, SessionSecurityFacts, SetupGuard};

pub(super) type Secured<S> = (PreparedStream<S>, SessionSecurityFacts, SetupGuard);
pub(super) type Pending<S> = (PreparedStream<S>, SetupGuard);

pub(super) async fn initiate<S, I: IdentityProvider>(
    (mut stream, security, guard): Secured<S>,
    binding: ProofBinding,
    credentials: Credentials<'_, I>,
) -> Result<Pending<S>, RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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

pub(super) async fn admit<S>(
    (stream, security, guard): Secured<S>,
    facts: &SessionFacts,
    admission: Admission<'_>,
) -> Result<(PreparedStream<S>, AdmittedPrincipal, SetupGuard), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let secure = security.secure_channel_facts().ok_or(RelayError::Setup)?;
    let admitted = guard
        .run(admit_session_secure(
            stream,
            admission.policy,
            admission.ledger,
            facts,
            &secure,
            admission.identity,
            admission.now_ms,
            admission.active_sessions,
        ))
        .await
        .map_err(|_| RelayError::Setup)?
        .map_err(|_| RelayError::Setup)?;
    match admitted {
        Ok(session) => {
            let session = session.into_session();
            Ok((session.stream, session.principal, guard))
        },
        Err(_) => {
            guard.refused();
            Err(RelayError::Unauthorized)
        },
    }
}

pub(super) async fn request<S>(
    stream: &mut S,
    guard: &SetupGuard,
    request: RelayRequest,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    guard
        .run(async {
            let bytes = [request.pool.as_slice(), request.target.as_slice()].concat();
            write_frame(stream, &bytes, 64)
                .await
                .map_err(|_| RelayError::Setup)?;
            if read_frame_or_eof(stream, 1)
                .await
                .map_err(|_| RelayError::Setup)?
                .as_deref()
                != Some(&[1])
            {
                return Err(RelayError::Unauthorized);
            }
            Ok(())
        })
        .await
        .map_err(|_| RelayError::Setup)?
}

pub(super) async fn read_request<S>(
    stream: &mut S,
    guard: &SetupGuard,
) -> Result<RelayRequest, RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    guard
        .run(async {
            let bytes = read_frame_or_eof(stream, 64)
                .await
                .map_err(|_| RelayError::Setup)?
                .filter(|bytes| bytes.len() == 64)
                .ok_or(RelayError::Unauthorized)?;
            Ok(RelayRequest {
                pool: bytes[..32].try_into().unwrap(),
                target: bytes[32..].try_into().unwrap(),
            })
        })
        .await
        .map_err(|_| RelayError::Setup)?
}

pub(super) async fn ready<S: AsyncWrite + Unpin>(
    stream: &mut S,
    guard: &SetupGuard,
) -> Result<(), RelayError> {
    guard
        .run(write_frame(stream, &[1], 1))
        .await
        .map_err(|_| RelayError::Setup)?
        .map_err(|_| RelayError::Setup)
}
