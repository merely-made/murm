//! Bounded, cancellation-safe serial framing. Partial reads survive poll timeouts.
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) struct Frames<T> {
    io: T,
    meshcore: bool,
    buffered: Vec<u8>,
}
impl<T: AsyncRead + AsyncWrite + Unpin> Frames<T> {
    pub fn new(io: T, meshcore: bool) -> Self {
        Self {
            io,
            meshcore,
            buffered: Vec::new(),
        }
    }
    pub async fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.is_empty() || payload.len() > 512 {
            return Err(invalid("companion payload bounds"));
        }
        let mut bytes = Vec::with_capacity(payload.len() + 4);
        if self.meshcore {
            bytes.push(b'<');
            bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        } else {
            bytes.extend_from_slice(&[0x94, 0xc3]);
            bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        bytes.extend_from_slice(payload);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            self.io.write_all(&bytes).await?;
            self.io.flush().await
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "companion write timed out; outcome uncertain",
            )
        })?
    }
    pub async fn next(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes_read = 0;
        loop {
            let header = if self.meshcore { 3 } else { 4 };
            let magic: &[u8] = if self.meshcore { b">" } else { &[0x94, 0xc3] };
            while !self.buffered.is_empty() && !self.buffered.starts_with(magic) {
                if !self.meshcore && self.buffered == [0x94] {
                    break;
                }
                self.buffered.remove(0);
            }
            if self.buffered.len() >= header {
                let n = if self.meshcore {
                    u16::from_le_bytes([self.buffered[1], self.buffered[2]])
                } else {
                    u16::from_be_bytes([self.buffered[2], self.buffered[3]])
                } as usize;
                if n == 0 || n > 512 {
                    return Err(invalid("invalid companion frame length"));
                }
                if self.buffered.len() >= header + n {
                    let frame = self.buffered[header..header + n].to_vec();
                    self.buffered.drain(..header + n);
                    return Ok(frame);
                }
            }
            let mut chunk = [0; 256];
            let n = self.io.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "companion disconnected",
                ));
            }
            bytes_read += n;
            if bytes_read > 4096 {
                return Err(invalid("companion noise/read budget exhausted"));
            }
            self.buffered.extend_from_slice(&chunk[..n]);
            if self.buffered.len() > 772 {
                return Err(invalid("companion input bounds"));
            }
        }
    }
}
pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
