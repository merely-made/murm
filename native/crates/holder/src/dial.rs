// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Dialing a holder to ask for an acknowledgement, over any carrier.
//!
//! The holder is named by the Noise key it must prove, so a stranger cannot
//! answer for a configured holder. Which carrier and address reach it is a
//! separate matter ([`Reach`]).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use identity::Ed25519Keypair;
use murm_resolve::{Local, key_alpn, release_alpn};
use murm_sync::{StreamSession, StreamSyncError, StreamSyncLimits, StreamSyncOutcome};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use transport::security::SetupLimits;
use transport::sync_lane::{SyncLane, SyncLaneError};
use transport::{Alpn, PeerID, Transport};

use crate::ack::{ACK_LEN, Ack};
use crate::service::{ACK_REQUEST, bounded};

/// How to reach one holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// By peer id on a peer-addressed carrier (memory, Reticulum).
    Peer(PeerID),
    /// By socket address over Noise on TCP.
    NoiseTcp(SocketAddr),
}

/// One configured holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HolderTarget {
    /// The key the holder proves, and signs acknowledgements with.
    pub holder: PeerID,
    /// Where to dial it.
    pub reach: Reach,
}

/// Why a holder did not answer.
#[derive(Debug, thiserror::Error)]
pub enum DialError {
    /// No enabled dialer handles this reach.
    #[error("no enabled carrier reaches this holder")]
    NoCarrier,
    /// The lane, carrier or Noise proof failed.
    #[error(transparent)]
    Lane(#[from] SyncLaneError),
    /// The holder answered that it does not hold the log.
    #[error("holder does not hold the log")]
    NotHeld,
    /// The reply was not an acknowledgement.
    #[error("holder sent no usable acknowledgement")]
    NoAck,
}

type Slot = Arc<Mutex<Option<Option<Ack>>>>;

/// The publisher's side of one acknowledgement request.
pub struct AckSession {
    key: [u8; 32],
    challenge: [u8; 32],
    slot: Slot,
}

impl AckSession {
    fn new(key: [u8; 32], challenge: [u8; 32]) -> (Self, Slot) {
        let slot = Slot::default();
        (
            Self {
                key,
                challenge,
                slot: slot.clone(),
            },
            slot,
        )
    }
}

impl StreamSession for AckSession {
    async fn run<St>(self, mut stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
    where
        St: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = StreamSyncLimits::default().deadline();
        let mut request = vec![ACK_REQUEST, 1];
        request.extend_from_slice(&self.key);
        request.extend_from_slice(&self.challenge);
        bounded(deadline, async {
            stream.write_all(&request).await?;
            stream.flush().await
        })
        .await?;
        let mut status = [0u8; 1];
        bounded(deadline, stream.read_exact(&mut status)).await?;
        let reply = if status[0] == 1 {
            let mut bytes = [0u8; ACK_LEN];
            bounded(deadline, stream.read_exact(&mut bytes)).await?;
            Ack::decode(&bytes).ok()
        } else {
            None
        };
        *self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reply);
        Ok(StreamSyncOutcome::default())
    }
}

/// A carrier the publisher can dial a holder over.
pub trait Dial {
    /// Short name for traces.
    fn name(&self) -> &'static str;
    /// Whether this dialer handles `reach`.
    fn handles(&self, reach: &Reach) -> bool;
    /// Ask `target` to acknowledge `key` for `challenge`. The answer is
    /// unverified; the caller checks it.
    fn ask<'a>(
        &'a self,
        target: &'a HolderTarget,
        key: [u8; 32],
        challenge: [u8; 32],
    ) -> Local<'a, Result<Ack, DialError>>;
}

fn lane(alpn: Alpn) -> SyncLane {
    SyncLane::new(alpn, 4, SetupLimits::default())
}

fn answer(slot: &Slot) -> Result<Ack, DialError> {
    match slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        Some(Some(ack)) => Ok(ack),
        Some(None) => Err(DialError::NotHeld),
        None => Err(DialError::NoAck),
    }
}

/// Any [`Transport`] whose peers are named by id.
pub struct TransportDialer<T> {
    name: &'static str,
    transport: T,
    noise_key: Ed25519Keypair,
    per_key_alpn: bool,
}

impl<T: Transport> TransportDialer<T> {
    /// Dial over `transport` as `noise_key`, on the generic release ALPN.
    pub fn new(name: &'static str, transport: T, noise_key: Ed25519Keypair) -> Self {
        Self {
            name,
            transport,
            noise_key,
            per_key_alpn: false,
        }
    }

    /// Dial on the per-key ALPN, for carriers whose holders announce a name
    /// that carries the log key (Reticulum).
    #[must_use]
    pub fn per_key_alpn(mut self) -> Self {
        self.per_key_alpn = true;
        self
    }
}

impl<T: Transport> Dial for TransportDialer<T> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn handles(&self, reach: &Reach) -> bool {
        matches!(reach, Reach::Peer(_))
    }

    fn ask<'a>(
        &'a self,
        target: &'a HolderTarget,
        key: [u8; 32],
        challenge: [u8; 32],
    ) -> Local<'a, Result<Ack, DialError>> {
        Box::pin(async move {
            let Reach::Peer(peer) = target.reach else {
                return Err(DialError::NoCarrier);
            };
            let alpn = if self.per_key_alpn {
                key_alpn(&key)
            } else {
                release_alpn()
            };
            let (session, slot) = AckSession::new(key, challenge);
            lane(alpn)
                .dial_noise_any(
                    &self.transport,
                    peer,
                    &self.noise_key,
                    Some(target.holder),
                    session,
                )
                .await?;
            answer(&slot)
        })
    }
}

/// Noise over TCP, for holders named by address.
pub struct NoiseTcpDialer {
    noise_key: Ed25519Keypair,
}

impl NoiseTcpDialer {
    /// Dial as `noise_key`.
    pub fn new(noise_key: Ed25519Keypair) -> Self {
        Self { noise_key }
    }
}

impl Dial for NoiseTcpDialer {
    fn name(&self) -> &'static str {
        "noise+tcp"
    }

    fn handles(&self, reach: &Reach) -> bool {
        matches!(reach, Reach::NoiseTcp(_))
    }

    fn ask<'a>(
        &'a self,
        target: &'a HolderTarget,
        key: [u8; 32],
        challenge: [u8; 32],
    ) -> Local<'a, Result<Ack, DialError>> {
        Box::pin(async move {
            let Reach::NoiseTcp(addr) = target.reach else {
                return Err(DialError::NoCarrier);
            };
            let (session, slot) = AckSession::new(key, challenge);
            lane(release_alpn())
                .dial_noise_tcp(addr, &self.noise_key, target.holder, session)
                .await?;
            answer(&slot)
        })
    }
}
