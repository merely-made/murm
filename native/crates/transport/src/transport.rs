// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! [`Transport`] trait and stream-type bound.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{AcceptedSession, Alpn, PeerID, TransportError};

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
