//! Bounded physical carriage for a caller-commissioned resident session.
//! The host supplies the runtime, radio, profile admission and identity custody.
//! Errors after a send begins remain uncertain; callers must stop and retire
//! the session rather than turn an uncertain physical outcome into a retry.
use super::{ResidentSession, backend};
use crate::comms_adapter::comms::AdapterError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::{Instant, timeout};
use tulle::radio_io::PacketRadio;
#[cfg(test)]
mod tests;

pub struct ResidentCarriage<'a, 'r, R> {
    session: ResidentSession<'a>,
    radio: &'r mut R,
    epoch: Instant,
    initial_ms: u64,
    receive_wait: Duration,
    stopped: bool,
}
impl<'a, 'r, R: PacketRadio> ResidentCarriage<'a, 'r, R> {
    /// `epoch`/`initial_ms` continue the runtime's caller-owned monotonic clock.
    /// The radio must already be online under the runtime's confirmed profile.
    /// Its receive future must be cancellation-safe, as the supplied Tulle
    /// serial links are: a timed receive cannot discard buffered frame bytes.
    pub fn new(
        session: ResidentSession<'a>,
        radio: &'r mut R,
        epoch: Instant,
        initial_ms: u64,
        receive_wait: Duration,
    ) -> Result<Self, AdapterError> {
        if receive_wait.is_zero() || receive_wait > Duration::from_secs(1) {
            return Err(backend(
                "resident receive wait must be greater than zero and at most one second",
            ));
        }
        Ok(Self {
            session,
            radio,
            epoch,
            initial_ms,
            receive_wait,
            stopped: false,
        })
    }
    fn now(&self) -> Result<u64, AdapterError> {
        self.initial_ms
            .checked_add(u64::try_from(self.epoch.elapsed().as_millis()).map_err(backend)?)
            .ok_or_else(|| backend("resident monotonic clock overflow"))
    }
    pub fn knows_peer(&self, key: &[u8; 32]) -> bool {
        self.session.knows_tucket_peer(key)
    }
    pub fn advertise(&mut self, name: &str) -> Result<(), AdapterError> {
        if self.stopped {
            return Err(backend("resident carriage stopped"));
        }
        self.session
            .advertise_tucket(self.now()?, unix_seconds()?, name)
    }
    /// Receive, advance protocol state, dispatch one persisted intent and
    /// perform at most one physical transmission. RX stays queued by the pump
    /// while a TX is in flight; no frame is dropped to cancel a receive wait.
    pub async fn poll(&mut self) -> Result<bool, AdapterError> {
        if self.stopped {
            return Err(backend("resident carriage stopped"));
        }
        let result = self.turn().await;
        if result.is_err() {
            self.stopped = true;
        }
        result
    }
    async fn turn(&mut self) -> Result<bool, AdapterError> {
        let mut changed = false;
        match timeout(self.receive_wait, self.radio.recv_frame()).await {
            Ok(Some(frame)) => {
                // The protocol owns RF validation and keeps loss accounting.
                match self.session.ingest(self.now()?, &frame.frame) {
                    Ok(()) => changed = true,
                    Err(error) => return Err(error),
                }
            }
            Ok(None) => return Err(backend("resident physical receive ended")),
            Err(_) => (),
        }
        changed |= self.session.poll_changed(self.now()?)?;
        changed |= self
            .session
            .dispatch_next(self.now()?, unix_seconds()?)?
            .is_some();
        changed |= self.session.poll_changed(self.now()?)?;
        if let Some(tx) = self.session.begin_tx(self.now()?)? {
            if tx.frame.len() > self.radio.max_frame_len() {
                self.session.complete_tx(self.now()?, tx.id, false)?;
                return Err(backend("resident frame exceeds physical radio limit"));
            }
            let remaining = tx.deadline.saturating_sub(self.now()?);
            let budget = remaining.min(self.session.transmit_budget_ms());
            if budget == 0 {
                return Err(backend("resident physical deadline expired before send"));
            }
            match timeout(
                Duration::from_millis(budget),
                self.radio.send_frame(tx.frame.to_vec()),
            )
            .await
            {
                Ok(Ok(_)) => self.session.complete_tx(self.now()?, tx.id, true)?,
                Ok(Err(error)) => {
                    return Err(backend(format!(
                        "resident physical send stopped; outcome uncertain: {error}"
                    )));
                }
                Err(_) => {
                    return Err(backend(
                        "resident physical send deadline exceeded; outcome uncertain",
                    ));
                }
            }
            changed = true;
        }
        Ok(changed)
    }
}
fn unix_seconds() -> Result<u32, AdapterError> {
    u32::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(backend)?
            .as_secs(),
    )
    .map_err(backend)
}
