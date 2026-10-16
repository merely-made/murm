//! Retain the first USB bytes while the physical-claim observer owns startup.

use embedded_io_async::{ErrorType, Read};

const QUERIES: [&[u8]; 12] = [
    b"status\n",
    b"status\r\n",
    b"name\n",
    b"name\r\n",
    b"region\n",
    b"region\r\n",
    b"channel\n",
    b"channel\r\n",
    b"sync\n",
    b"sync\r\n",
    b"ui\n",
    b"ui\r\n",
];

pub struct BootRx<R> {
    inner: R,
    bytes: [u8; 64],
    length: usize,
    offset: usize,
}

impl<R: Read> BootRx<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            bytes: [0; 64],
            length: 0,
            offset: 0,
        }
    }

    /// Read-only queries can select ordinary service only when both journals
    /// permit it. Other bytes remain intact for the physical first-owner carrier
    /// or the ordinary parser after the existing observation window expires.
    pub async fn prefetch_query(&mut self, ordinary_eligible: bool) -> Result<bool, R::Error> {
        while self.length < self.bytes.len() {
            let read = self.inner.read(&mut self.bytes[self.length..]).await?;
            if read == 0 {
                return Ok(false);
            }
            self.length += read;
            let packet = &self.bytes[..self.length];
            if !ordinary_eligible {
                return Ok(false);
            }
            if QUERIES.contains(&packet) {
                return Ok(true);
            }
            if !QUERIES.iter().any(|query| query.starts_with(packet)) {
                return Ok(false);
            }
        }
        Ok(false)
    }
}

impl<R: Read> ErrorType for BootRx<R> {
    type Error = R::Error;
}

impl<R: Read> Read for BootRx<R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.offset < self.length {
            let count = out.len().min(self.length - self.offset);
            out[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            return Ok(count);
        }
        self.inner.read(out).await
    }
}
