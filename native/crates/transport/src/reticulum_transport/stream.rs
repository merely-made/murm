// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! An `AsyncRead + AsyncWrite` byte stream over a retinue link.
//!
//! retinue's [`LinkStream`] already presents a link as a tokio duplex (a relay
//! task chunks writes into encrypted link-data packets and feeds decrypted
//! inbound bytes back in), so this is a thin newtype: it gives the transport a
//! stable, documented [`Transport::Stream`](crate::Transport::Stream) type and
//! keeps retinue's `LinkStream` from leaking across the crate boundary.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use retinue::endpoint::LinkStream;

/// Bidirectional byte stream over a retinue link.
///
/// Implements [`AsyncRead`] + [`AsyncWrite`] by delegating to the wrapped
/// [`LinkStream`] (which is [`Unpin`]).
pub struct ReticulumStream(LinkStream);

impl ReticulumStream {
    /// Wrap a retinue link stream.
    pub(super) fn new(inner: LinkStream) -> Self {
        Self(inner)
    }

    /// The id of the link carrying this stream.
    ///
    /// Both ends of a retinue link compute the same id, which is what lets an
    /// initiator bind a session proof to the link it is about to speak on and
    /// a responder verify that proof against the link it observed. The
    /// initiator has no other way to learn it: `accept` reports ingress on the
    /// responder side only.
    pub fn link_id(&self) -> [u8; 16] {
        *self.0.link_id().as_bytes()
    }
}

impl std::fmt::Debug for ReticulumStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReticulumStream").finish_non_exhaustive()
    }
}

impl AsyncRead for ReticulumStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for ReticulumStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
