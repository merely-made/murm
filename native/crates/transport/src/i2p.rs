// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Host-owned I2P STREAM carriage through the existing SAM client. This module
//! makes no Personae-authentication claim and contains no direct-route fallback.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
mod anonymous;
pub use anonymous::anonymous_i2p_key;
pub use truchman::{Bridge, Key};
use truchman::{SessionOptions, StreamSession};

/// A canonical, unblinded I2P service locator. No IP, URL, port or native ticket.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct I2pAddress(String);
impl I2pAddress {
    /// Parse exactly a 52-character lowercase base32 name plus `.b32.i2p`.
    ///
    /// # Errors
    /// Refuses noncanonical names and every non-I2P locator.
    pub fn parse(value: &str) -> Result<Self, I2pError> {
        let name = value.strip_suffix(".b32.i2p").ok_or(I2pError::Profile)?;
        if name.len() != 52
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        {
            return Err(I2pError::Profile);
        }
        // A 256-bit hash has only one significant bit in the last base32 digit.
        if !matches!(name.as_bytes()[51], b'a' | b'q') {
            return Err(I2pError::Profile);
        }
        Ok(Self(value.into()))
    }
    /// Deliberate disclosure of this service's public locator.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for I2pAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("I2pAddress { private }")
    }
}

/// Explicit nonzero-hop settings. No ambient router defaults or hop variance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TunnelProfile {
    inbound: u8,
    outbound: u8,
    quantity: u8,
    publish: bool,
}
impl TunnelProfile {
    /// Select hops, tunnel count per direction and LeaseSet publication.
    ///
    /// # Errors
    /// Refuses zero/excessive hops or an invalid tunnel count.
    pub fn new(inbound: u8, outbound: u8, quantity: u8, publish: bool) -> Result<Self, I2pError> {
        if !(1..=8).contains(&inbound)
            || !(1..=8).contains(&outbound)
            || !(1..=16).contains(&quantity)
        {
            return Err(I2pError::Profile);
        }
        Ok(Self {
            inbound,
            outbound,
            quantity,
            publish,
        })
    }
    /// Exact options passed to SAM, useful for inspecting the selected profile.
    pub fn options(self) -> SessionOptions {
        SessionOptions {
            options: vec![
                ("inbound.length".into(), self.inbound.to_string()),
                ("outbound.length".into(), self.outbound.to_string()),
                ("inbound.lengthVariance".into(), "0".into()),
                ("outbound.lengthVariance".into(), "0".into()),
                ("inbound.quantity".into(), self.quantity.to_string()),
                ("outbound.quantity".into(), self.quantity.to_string()),
                (
                    "i2cp.dontPublishLeaseSet".into(),
                    (!self.publish).to_string(),
                ),
                ("i2cp.leaseSetEncType".into(), "4".into()),
            ],
            ..SessionOptions::default()
        }
    }
    /// Whether the host selected a published, reachable service.
    pub fn publishes(self) -> bool {
        self.publish
    }
}

/// Public refusal facts; SAM messages, aliases, bridge addresses and keys stay private.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum I2pError {
    /// Invalid privacy settings or locator.
    #[error("I2P profile or locator refused")]
    Profile,
    /// Provider/session unavailable, lookup refused or stream establishment failed.
    #[error("I2P provider unavailable")]
    Unavailable,
}

/// A host-owned session. Clones share one existing SAM session and its lifetime.
#[derive(Clone)]
pub struct I2pSession {
    session: Arc<StreamSession>,
    profile: TunnelProfile,
}
impl std::fmt::Debug for I2pSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I2pSession")
            .field("profile", &self.profile)
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}
impl I2pSession {
    /// Create through a trusted loopback bridge, with caller-selected identity.
    /// Persistent/transient key policy stays with the host, outside Retinue defaults.
    ///
    /// # Errors
    /// Refuses non-loopback bridges, invalid time limits or failed SAM creation.
    pub async fn create(
        bridge: Bridge,
        key: Key,
        profile: TunnelProfile,
        timeout: Duration,
    ) -> Result<Self, I2pError> {
        if !bridge.addr().ip().is_loopback()
            || timeout.is_zero()
            || timeout > Duration::from_secs(300)
        {
            return Err(I2pError::Profile);
        }
        let session = tokio::time::timeout(
            timeout,
            StreamSession::create(bridge, key, profile.options()),
        )
        .await
        .map_err(|_| I2pError::Unavailable)?
        .map_err(|_| I2pError::Unavailable)?;
        Ok(Self {
            session: Arc::new(session),
            profile,
        })
    }
    /// Current service alias; only published sessions may offer inbound service.
    ///
    /// # Errors
    /// Refuses unpublished or ended sessions.
    pub fn address(&self) -> Result<I2pAddress, I2pError> {
        if !self.profile.publish || !self.is_alive() {
            return Err(I2pError::Unavailable);
        }
        I2pAddress::parse(&self.session.destination().b32())
    }
    /// Configured tunnel settings, not a claim of independently observed router behavior.
    pub fn profile(&self) -> TunnelProfile {
        self.profile
    }
    /// Whether the provider still holds this session's control connection.
    pub fn is_alive(&self) -> bool {
        self.session.is_alive()
    }
    /// Await provider/session termination.
    pub async fn closed(&self) {
        self.session.closed().await
    }
    /// Resolve only an I2P locator and connect through this SAM session.
    /// The caller's shared setup gate bounds this entire future.
    ///
    /// # Errors
    /// Refuses failed/mismatched lookup, connect or ended parent sessions.
    pub async fn connect(&self, address: &I2pAddress) -> Result<I2pStream, I2pError> {
        let work = async {
            let destination = self
                .session
                .lookup(address.as_str())
                .await
                .map_err(|_| I2pError::Unavailable)?;
            // A faulty provider cannot substitute another destination for this hash.
            if destination.b32() != address.as_str() {
                return Err(I2pError::Unavailable);
            }
            let stream = self
                .session
                .connect(&destination)
                .await
                .map_err(|_| I2pError::Unavailable)?;
            Ok(self.wrap(stream))
        };
        tokio::select! { result = work => result, _ = self.closed() => Err(I2pError::Unavailable) }
    }
    /// Accept a SAM stream. Its reported Destination never becomes a Personae subject.
    /// Keep this command future pinned while reaping unrelated tasks. Cancelling
    /// it closes the local socket, but a provider may retain a pending accept
    /// briefly; do not repeatedly cancel/reissue it as a listener polling trick.
    ///
    /// # Errors
    /// Refuses disabled publication, failed acceptance or provider termination.
    pub async fn accept(&self) -> Result<I2pStream, I2pError> {
        if !self.profile.publish {
            return Err(I2pError::Profile);
        }
        tokio::select! {
            result = self.session.accept() => result.map(|(stream, _)| self.wrap(stream)).map_err(|_| I2pError::Unavailable),
            _ = self.closed() => Err(I2pError::Unavailable),
        }
    }
    fn wrap(&self, stream: truchman::Stream) -> I2pStream {
        let session = self.session.clone();
        I2pStream {
            stream,
            closed: Box::pin(async move { session.closed().await }),
            ended: false,
        }
    }
}

/// A SAM byte stream that fails when its parent control session terminates.
/// No task is spawned per stream and no payload is queued or persisted here.
pub struct I2pStream {
    stream: truchman::Stream,
    closed: Pin<Box<dyn Future<Output = ()> + Send>>,
    ended: bool,
}
impl I2pStream {
    fn available(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if !self.ended {
            self.ended = self.closed.as_mut().poll(cx).is_ready();
        }
        if self.ended {
            Err(io::ErrorKind::NotConnected.into())
        } else {
            Ok(())
        }
    }
}
impl AsyncRead for I2pStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.available(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for I2pStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.available(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.available(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;

/// Scripted loopback SAM fixture, available only to throwaway-identity tests.
#[cfg(feature = "test-identity")]
#[doc(hidden)]
pub mod fixture;
