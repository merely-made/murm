// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A byte stream over a completed Noise session.
//!
//! ## Framing, and why it is ours to choose
//!
//! Noise messages are **discrete and capped at 65535 bytes**; TCP is a byte
//! stream with no message boundaries. Something has to reconcile them, and the
//! Noise specification deliberately does not: framing is left to the
//! application.
//!
//! So this carrier defines it, and the choice is the conventional one: a
//! **2-byte big-endian length prefix** ahead of each Noise message, exactly as
//! libp2p-noise does. That is a legitimate decision *for our own carrier*,
//! where both ends are this code. It is emphatically not licence to guess
//! another protocol's framing.
//!
//! ## The plaintext budget
//!
//! A Noise message holds 65535 bytes, of which 16 are the authentication tag,
//! so at most [`MAX_PLAINTEXT`] bytes of payload ride in one frame. Writes
//! larger than that are split across frames; a reader sees a byte stream and
//! never has to know.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use snow::TransportState;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

/// The largest Noise message, per the specification.
pub(super) const MAX_MESSAGE: usize = 65535;
/// Payload budget once the 16-byte AEAD tag is accounted for.
pub(super) const MAX_PLAINTEXT: usize = MAX_MESSAGE - 16;

/// Read one length-prefixed Noise message from `stream`.
pub(super) async fn read_frame<S>(stream: &mut S) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut length = [0u8; 2];
    stream.read_exact(&mut length).await?;
    let length = usize::from(u16::from_be_bytes(length));
    let mut frame = vec![0u8; length];
    stream.read_exact(&mut frame).await?;
    Ok(frame)
}

/// Write one length-prefixed Noise message to `stream`.
pub(super) async fn write_frame<S>(stream: &mut S, message: &[u8]) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let length = u16::try_from(message.len())
        .map_err(|_| io::Error::other("noise message exceeds 65535"))?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(message).await?;
    stream.flush().await
}

/// An encrypted byte stream over a completed Noise handshake.
///
/// Implements [`AsyncRead`] and [`AsyncWrite`], so everything above it -- the
/// ALPN exchange, and whatever protocol runs on top -- is written against
/// ordinary tokio I/O and never sees a Noise message.
pub struct NoiseStream<S> {
    inner: S,
    session: TransportState,
    /// Decrypted bytes not yet handed to a reader.
    plaintext: Vec<u8>,
    plaintext_at: usize,
    /// The frame currently being read: its declared length, then its body.
    read_state: ReadState,
    /// Ciphertext queued for writing, and how much has gone out.
    pending_write: Vec<u8>,
    pending_at: usize,
}

enum ReadState {
    Length { buffer: [u8; 2], filled: usize },
    Body { buffer: Vec<u8>, filled: usize },
}

impl<S> NoiseStream<S> {
    pub(super) fn new(inner: S, session: TransportState) -> Self {
        Self {
            inner,
            session,
            plaintext: Vec::new(),
            plaintext_at: 0,
            read_state: ReadState::Length {
                buffer: [0u8; 2],
                filled: 0,
            },
            pending_write: Vec::new(),
            pending_at: 0,
        }
    }

    /// The peer's Noise static public key, as proven by the handshake.
    pub fn peer_static_key(&self) -> Option<&[u8]> {
        self.session.get_remote_static()
    }
}

impl<S> std::fmt::Debug for NoiseStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately opaque: a session's buffers hold plaintext.
        f.debug_struct("NoiseStream").finish_non_exhaustive()
    }
}

impl<S> AsyncRead for NoiseStream<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        loop {
            // Hand over buffered plaintext first.
            if this.plaintext_at < this.plaintext.len() {
                let available = &this.plaintext[this.plaintext_at..];
                let take = available.len().min(buf.remaining());
                buf.put_slice(&available[..take]);
                this.plaintext_at += take;
                if this.plaintext_at == this.plaintext.len() {
                    this.plaintext.clear();
                    this.plaintext_at = 0;
                }
                return Poll::Ready(Ok(()));
            }

            // Otherwise pull the next frame, a piece at a time.
            match &mut this.read_state {
                ReadState::Length { buffer, filled } => {
                    let mut read = ReadBuf::new(&mut buffer[*filled..]);
                    match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Ready(Ok(())) => {
                            let got = read.filled().len();
                            // A clean end of stream between frames is EOF.
                            if got == 0 {
                                return Poll::Ready(if *filled == 0 {
                                    Ok(())
                                } else {
                                    Err(io::ErrorKind::UnexpectedEof.into())
                                });
                            }
                            *filled += got;
                            if *filled == 2 {
                                let length = usize::from(u16::from_be_bytes(*buffer));
                                this.read_state = ReadState::Body {
                                    buffer: vec![0u8; length],
                                    filled: 0,
                                };
                            }
                        },
                    }
                },
                ReadState::Body { buffer, filled } => {
                    if *filled < buffer.len() {
                        let mut read = ReadBuf::new(&mut buffer[*filled..]);
                        match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                            Poll::Ready(Ok(())) => {
                                let got = read.filled().len();
                                if got == 0 {
                                    return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
                                }
                                *filled += got;
                            },
                        }
                    }
                    if *filled == buffer.len() {
                        let mut plaintext = vec![0u8; MAX_MESSAGE];
                        let written = this
                            .session
                            .read_message(buffer, &mut plaintext)
                            // A decryption failure is a torn or tampered
                            // frame, and the session cannot continue.
                            .map_err(|e| io::Error::other(format!("noise decrypt: {e}")))?;
                        plaintext.truncate(written);
                        this.plaintext = plaintext;
                        this.plaintext_at = 0;
                        this.read_state = ReadState::Length {
                            buffer: [0u8; 2],
                            filled: 0,
                        };
                    }
                },
            }
        }
    }
}

impl<S> AsyncWrite for NoiseStream<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        // Finish flushing any frame already encrypted before taking more.
        while this.pending_at < this.pending_write.len() {
            match Pin::new(&mut this.inner).poll_write(cx, &this.pending_write[this.pending_at..]) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) => this.pending_at += n,
            }
        }
        this.pending_write.clear();
        this.pending_at = 0;

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // One frame per poll; a longer write is split and the caller loops.
        let take = buf.len().min(MAX_PLAINTEXT);
        let mut ciphertext = vec![0u8; MAX_MESSAGE];
        let written = this
            .session
            .write_message(&buf[..take], &mut ciphertext)
            .map_err(|e| io::Error::other(format!("noise encrypt: {e}")))?;

        let length =
            u16::try_from(written).map_err(|_| io::Error::other("noise message exceeds 65535"))?;
        this.pending_write.reserve(2 + written);
        this.pending_write.extend_from_slice(&length.to_be_bytes());
        this.pending_write.extend_from_slice(&ciphertext[..written]);

        // Push what we can now; the rest drains on the next call or flush.
        while this.pending_at < this.pending_write.len() {
            match Pin::new(&mut this.inner).poll_write(cx, &this.pending_write[this.pending_at..]) {
                Poll::Pending => break,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) => this.pending_at += n,
            }
        }
        // The plaintext is committed to the session's nonce sequence, so it
        // must be reported as accepted even if its ciphertext is still queued.
        Poll::Ready(Ok(take))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        while this.pending_at < this.pending_write.len() {
            match Pin::new(&mut this.inner).poll_write(cx, &this.pending_write[this.pending_at..]) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) => this.pending_at += n,
            }
        }
        this.pending_write.clear();
        this.pending_at = 0;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut *this).poll_flush(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => Pin::new(&mut this.inner).poll_shutdown(cx),
        }
    }
}
