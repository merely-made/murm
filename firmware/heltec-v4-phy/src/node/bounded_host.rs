//! The node loop's host link: every write is bounded, and a stalled host is skipped.
//!
//! The node runs with nothing on USB, and this board's USB Serial/JTAG cannot tell that a host
//! has left: a write to a port nobody drains waits for a drain that never comes. So each write
//! gets [`HOST_WRITE_BOUND`]; one that runs out closes the [`WriteGate`] until the host next
//! sends bytes. Reads are never bounded: the loop races them, so they hold nothing.

use embassy_time::{Duration, with_timeout};
use radio_hand::link::{HostLink, LinkFault, WriteGate};

/// Longest a single host write may hold the node loop. Under the airtime of the shortest
/// LongFast frame, so a stalled write cannot cost a frame already in the radio.
pub const HOST_WRITE_BOUND: Duration = Duration::from_millis(100);

pub struct BoundedHost<L> {
    inner: L,
    gate: WriteGate,
}

impl<L: HostLink> BoundedHost<L> {
    pub fn new(inner: L) -> Self {
        Self {
            inner,
            gate: WriteGate::open(),
        }
    }

    fn settle(
        &mut self,
        outcome: Result<Result<(), LinkFault>, embassy_time::TimeoutError>,
    ) -> Result<(), LinkFault> {
        outcome.unwrap_or_else(|_| {
            self.gate.expired();
            Err(LinkFault::Detached)
        })
    }
}

impl<L: HostLink> HostLink for BoundedHost<L> {
    async fn attached(&mut self) {
        self.inner.attached().await
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, LinkFault> {
        let result = self.inner.read(buf).await;
        if let Ok(length) = result {
            self.gate.heard(length);
        }
        result
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), LinkFault> {
        if !self.gate.is_open() {
            return Err(LinkFault::Detached);
        }
        let outcome = with_timeout(HOST_WRITE_BOUND, self.inner.write_all(bytes)).await;
        self.settle(outcome)
    }

    async fn write_diagnostic(&mut self, bytes: &[u8]) -> Result<(), LinkFault> {
        if !self.gate.is_open() {
            return Err(LinkFault::Detached);
        }
        let outcome = with_timeout(HOST_WRITE_BOUND, self.inner.write_diagnostic(bytes)).await;
        self.settle(outcome)
    }
}
