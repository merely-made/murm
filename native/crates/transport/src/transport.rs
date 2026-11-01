// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! [`Transport`] trait and stream-type bound.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{AcceptedSession, Alpn, OutboundSession, PeerID, TransportError};

/// A peer-to-peer transport for the Mere browser.
///
/// `Transport` provides authenticated, encrypted bidirectional streams between
/// peers identified by [`PeerID`]. The reference implementation will be
/// iroh-backed (Phase 2C work); an in-memory backend for tests is Phase 2B
/// work.
///
/// ## Generic over implementation
///
/// Consumers (`murm`, `gemot`, etc.) take a `Transport` as a generic
/// parameter rather than `Box<dyn Transport>`, so the same code works
/// against real iroh in production and against an in-memory transport in
/// tests:
///
/// ```ignore
/// async fn open_cable_stream<T: Transport>(
///     transport: &T,
///     peer: PeerID,
/// ) -> Result<T::Stream, TransportError> {
///     transport
///         .connect(peer, Alpn::new("mere/cable/v1"))
///         .await
/// }
/// ```
///
/// The associated `Stream` type implements [`tokio::io::AsyncRead`] and
/// [`tokio::io::AsyncWrite`], so consumers can use standard tokio I/O
/// patterns (length-prefixed framing, line-delimited reads, codec layers,
/// etc.) without caring about the underlying transport.
///
/// ## Send + Sync + 'static
///
/// All trait bounds (`Send + Sync` on the trait, `Send + Unpin + 'static` on
/// `Stream`, and `+ Send` on returned futures) are present so transport
/// objects can be shared across `tokio::spawn`-ed tasks. This matches the
/// expected runtime topology where a single transport is used by many
/// concurrent protocol handlers.
pub trait Transport: Send + Sync {
    /// The bidirectional stream type returned by `connect` and `accept`.
    ///
    /// Implements [`tokio::io::AsyncRead`] + [`tokio::io::AsyncWrite`] so
    /// standard tokio I/O works on it directly.
    type Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// The local node identifier (derived from the master public key in
    /// [`identity`]).
    fn local_peer_id(&self) -> PeerID;

    /// Open a stream to a peer for a specific protocol (ALPN).
    ///
    /// Returns once the peer accepts on the matching ALPN. The returned
    /// stream is bidirectional and ready for I/O.
    fn connect(
        &self,
        peer: PeerID,
        alpn: Alpn,
    ) -> impl Future<Output = Result<Self::Stream, TransportError>> + Send;

    /// Open an outbound stream while retaining its common proof-binding facts.
    ///
    /// The default adapter has no knowledge of carrier authentication or link
    /// identity and therefore reports neither. Carriers that establish those
    /// facts override this method. Callers must not infer them from
    /// [`Self::local_peer_id`], which may be only a routing identity.
    fn connect_session(
        &self,
        peer: PeerID,
        alpn: Alpn,
    ) -> impl Future<Output = Result<OutboundSession<Self::Stream>, TransportError>> + Send {
        async move {
            let stream = self.connect(peer, alpn.clone()).await?;
            Ok(OutboundSession::new(stream, alpn, None, None))
        }
    }

    /// Accept a single incoming session for a specific protocol.
    ///
    /// Returns once a peer connects on the given ALPN, with the stream and the
    /// facts the transport can honestly report about it: the protocol, the
    /// authenticated peer (if the transport authenticates), and where it
    /// arrived. See [`AcceptedSession`] for why `peer` is an `Option` and why
    /// it is never filled from application bytes.
    ///
    /// ```ignore
    /// let alpn = Alpn::new("mere/cable/v1");
    /// loop {
    ///     let session = transport.accept(alpn.clone()).await?;
    ///     tokio::spawn(handle_cable_stream(session.stream));
    /// }
    /// ```
    fn accept(
        &self,
        alpn: Alpn,
    ) -> impl Future<Output = Result<AcceptedSession<Self::Stream>, TransportError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryTransport;
    use identity::{IdentityProvider, InMemoryProvider};
    use tokio::io::DuplexStream;

    // An external adapter that has not implemented outbound observation yet.
    struct UnknownOutbound(MemoryTransport);

    impl Transport for UnknownOutbound {
        type Stream = DuplexStream;

        fn local_peer_id(&self) -> PeerID {
            self.0.local_peer_id()
        }

        async fn connect(&self, peer: PeerID, alpn: Alpn) -> Result<Self::Stream, TransportError> {
            self.0.connect(peer, alpn).await
        }

        async fn accept(
            &self,
            alpn: Alpn,
        ) -> Result<AcceptedSession<Self::Stream>, TransportError> {
            self.0.accept(alpn).await
        }
    }

    #[tokio::test]
    async fn default_outbound_facts_do_not_guess_from_a_local_routing_identity() {
        let local =
            PeerID::from_public_key(InMemoryProvider::from_seed([91; 32]).master_public_key());
        let remote =
            PeerID::from_public_key(InMemoryProvider::from_seed([92; 32]).master_public_key());
        let (left, right) = MemoryTransport::pair(local, remote);
        let alpn = Alpn::new("mere/outbound-test/v1");
        let outbound = UnknownOutbound(left)
            .connect_session(remote, alpn.clone())
            .await
            .unwrap();
        assert_eq!(outbound.authenticated_initiator, None);
        assert_eq!(outbound.shared_link, None);
        assert_eq!(outbound.protocol, alpn);
        assert_eq!(right.accept(alpn).await.unwrap().peer, Some(local));
    }
}
