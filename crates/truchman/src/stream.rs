//! A connected I2P stream: the bridge socket after `STREAM STATUS RESULT=OK`, replaying any
//! bytes that were read along with the status line.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

/// A reader that yields `prefix` before reading `inner`.
#[derive(Debug)]
pub struct Prefixed<R> {
    prefix: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R: AsyncRead + Unpin> AsyncRead for Prefixed<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.prefix.len() {
            let take = (this.prefix.len() - this.pos).min(buf.remaining());
            buf.put_slice(&this.prefix[this.pos..this.pos + take]);
            this.pos += take;
            if this.pos == this.prefix.len() {
                this.prefix = Vec::new();
                this.pos = 0;
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Prefixed<W> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, b)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A byte stream to a remote destination.
pub type Stream = Prefixed<TcpStream>;
/// The read half of a split [`Stream`].
pub type ReadHalf = Prefixed<OwnedReadHalf>;
/// The write half of a split [`Stream`].
pub type WriteHalf = OwnedWriteHalf;

impl Stream {
    pub(crate) fn new(inner: TcpStream, prefix: Vec<u8>) -> Self {
        Self {
            prefix,
            pos: 0,
            inner,
        }
    }

    /// Split into halves that can be driven from separate tasks or futures.
    pub fn into_split(self) -> (ReadHalf, WriteHalf) {
        let (read, write) = self.inner.into_split();
        let read = Prefixed {
            prefix: self.prefix,
            pos: self.pos,
            inner: read,
        };
        (read, write)
    }
}
