// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::{Lease, PreparedStream, StreamKind};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

#[derive(Clone, Copy)]
pub(super) enum FrameFormat {
    Noise,
    Application,
}
impl FrameFormat {
    fn prefix(self) -> usize {
        match self {
            Self::Noise => 2,
            Self::Application => 4,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Frame {
    prefix: [u8; 4],
    filled: usize,
    body_remaining: usize,
}
impl Frame {
    fn cap(&self, prefix: usize) -> usize {
        if self.body_remaining > 0 {
            self.body_remaining
        } else {
            prefix - self.filled
        }
    }
    fn observe(
        &mut self,
        mut bytes: &[u8],
        prefix: usize,
        maximum: usize,
        frames: &mut usize,
        max_frames: usize,
    ) -> io::Result<()> {
        while !bytes.is_empty() {
            if self.body_remaining > 0 {
                let take = bytes.len().min(self.body_remaining);
                self.body_remaining -= take;
                bytes = &bytes[take..];
            } else {
                let take = bytes.len().min(prefix - self.filled);
                self.prefix[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
                self.filled += take;
                bytes = &bytes[take..];
                if self.filled == prefix {
                    let length = if prefix == 2 {
                        usize::from(u16::from_be_bytes([self.prefix[0], self.prefix[1]]))
                    } else {
                        u32::from_be_bytes(self.prefix) as usize
                    };
                    if length > maximum || *frames >= max_frames {
                        return Err(limit());
                    }
                    *frames += 1;
                    self.body_remaining = length;
                    self.filled = 0;
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Usage {
    read: usize,
    written: usize,
    frames: usize,
    polls: usize,
}

pub(super) struct BudgetIo<S> {
    inner: S,
    lease: Arc<Lease>,
    format: FrameFormat,
    read: Frame,
    write: Frame,
}
impl<S> BudgetIo<S> {
    pub(super) fn new(inner: S, lease: Arc<Lease>, format: FrameFormat) -> Self {
        Self {
            inner,
            lease,
            format,
            read: Frame::default(),
            write: Frame::default(),
        }
    }
    fn active(&self) -> io::Result<bool> {
        match self.lease.status.load(Ordering::Acquire) {
            1 => Ok(false),
            0 if Instant::now() < self.lease.deadline => Ok(true),
            0 => {
                self.lease.close(3);
                Err(io::Error::new(io::ErrorKind::TimedOut, "setup deadline"))
            },
            _ => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "setup closed",
            )),
        }
    }
    fn poll_budget(&self, usage: &mut Usage) -> io::Result<()> {
        if usage.polls >= self.lease.limits.max_io_operations {
            self.lease.close(2);
            return Err(limit());
        }
        usage.polls += 1;
        Ok(())
    }
}

fn limit() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "setup I/O budget")
}

impl<S: AsyncRead + Unpin> AsyncRead for BudgetIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.active()? {
            return Pin::new(&mut this.inner).poll_read(cx, buf);
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let state = this.lease.clone();
        let mut usage = state.usage.lock().unwrap_or_else(|p| p.into_inner());
        this.poll_budget(&mut usage)?;
        let remaining = state.limits.max_read_bytes.saturating_sub(usage.read);
        if remaining == 0 {
            state.close(2);
            return Poll::Ready(Err(limit()));
        }
        // Never deliver a complete unchecked prefix to a decoder. In particular,
        // an oversized Noise frame is refused before NoiseStream allocates it.
        let cap = buf
            .remaining()
            .min(this.read.cap(this.format.prefix()))
            .min(remaining);
        let mut part = ReadBuf::new(buf.initialize_unfilled_to(cap));
        match Pin::new(&mut this.inner).poll_read(cx, &mut part) {
            Poll::Ready(Ok(())) => {
                let n = part.filled().len();
                let observed = this.read.observe(
                    part.filled(),
                    this.format.prefix(),
                    state.limits.max_frame_bytes,
                    &mut usage.frames,
                    state.limits.max_frames,
                );
                usage.read += n;
                if let Err(error) = observed {
                    state.close(2);
                    return Poll::Ready(Err(error));
                }
                // Reserve the declared body against the remaining byte budget
                // before releasing its prefix to an allocating decoder.
                if this.read.body_remaining > state.limits.max_read_bytes - usage.read {
                    state.close(2);
                    return Poll::Ready(Err(limit()));
                }
                buf.advance(n);
                Poll::Ready(Ok(()))
            },
            other => other,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BudgetIo<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if !this.active()? {
            return Pin::new(&mut this.inner).poll_write(cx, buf);
        }
        let state = this.lease.clone();
        let mut usage = state.usage.lock().unwrap_or_else(|p| p.into_inner());
        this.poll_budget(&mut usage)?;
        let mut trial = this.write;
        let mut frames = usage.frames;
        if buf.len() > state.limits.max_write_bytes.saturating_sub(usage.written)
            || trial
                .observe(
                    buf,
                    this.format.prefix(),
                    state.limits.max_frame_bytes,
                    &mut frames,
                    state.limits.max_frames,
                )
                .is_err()
        {
            state.close(2);
            return Poll::Ready(Err(limit()));
        }
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if let Err(error) = this.write.observe(
                    &buf[..n],
                    this.format.prefix(),
                    state.limits.max_frame_bytes,
                    &mut usage.frames,
                    state.limits.max_frames,
                ) {
                    state.close(2);
                    return Poll::Ready(Err(error));
                }
                usage.written += n;
                Poll::Ready(Ok(n))
            },
            other => other,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.active()? {
            let state = this.lease.clone();
            let mut usage = state.usage.lock().unwrap_or_else(|p| p.into_inner());
            this.poll_budget(&mut usage)?;
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PreparedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.permitted()?;
        match &mut this.inner {
            StreamKind::Noise(s) => Pin::new(s).poll_read(cx, buf),
            StreamKind::Plain(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for PreparedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.permitted()?;
        match &mut this.inner {
            StreamKind::Noise(s) => Pin::new(s).poll_write(cx, buf),
            StreamKind::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.permitted()?;
        match &mut this.inner {
            StreamKind::Noise(s) => Pin::new(s).poll_flush(cx),
            StreamKind::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            StreamKind::Noise(s) => Pin::new(s).poll_shutdown(cx),
            StreamKind::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl<S> PreparedStream<S> {
    fn permitted(&self) -> io::Result<()> {
        match self.lease.status.load(Ordering::Acquire) {
            1 => Ok(()),
            0 if Instant::now() < self.lease.deadline => Ok(()),
            0 => {
                self.lease.close(3);
                Err(io::Error::new(io::ErrorKind::TimedOut, "setup deadline"))
            },
            _ => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "setup closed",
            )),
        }
    }
}
