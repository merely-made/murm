//! Radio traffic: inbound frames, protocol polling, sends, and the transmit handshake.

use super::*;

impl Runtime {
    pub fn ingest(&mut self, now: u64, frame: &[u8]) -> Result<Report, Error> {
        self.time(now)?;
        if frame.len() > selvage::MAX_RADIO_FRAME_LEN {
            if self
                .active()
                .is_some_and(|active| active.instance == RETINUE)
            {
                self.carrier_rejected_rx = self.carrier_rejected_rx.saturating_add(1);
            }
            return Err(Error::FrameTooLong);
        }
        self.expiry(now)?;
        match self.active().ok_or(Error::Inactive)?.instance {
            RETINUE => {
                let p = self.carrier.decode(frame).map_err(|reason| {
                    self.carrier_rejected_rx = self.carrier_rejected_rx.saturating_add(1);
                    Error::Carrier(reason)
                })?;
                let a = self
                    .retinue
                    .as_mut()
                    .ok_or(Error::WrongInstance)?
                    .ingest(now, &p)?;
                self.retinue_actions(now, a)?;
            }
            SENNET => {
                let e = self
                    .sennet
                    .as_mut()
                    .ok_or(Error::WrongInstance)?
                    .receive(now, frame)?;
                self.event(Event::SennetReceived(e));
                self.queue_sennet_pending(now)?;
            }
            TUCKET => {
                let tucket = self.tucket.as_mut().ok_or(Error::WrongInstance)?;
                let delay = tucket.reply_delay_ms();
                let out = tucket.on_frame(now, frame)?;
                self.tucket_lost(now, out.expired)?;
                for op in out.acknowledged {
                    for id in self.work.cancel_operation(now, u64::from(op.0))? {
                        self.event(Event::WorkCancelled(id));
                    }
                    self.event(Event::TucketAcknowledged(op));
                }
                for e in out.events {
                    self.event(Event::Tucket(e));
                }
                let deadline = self.ttl(now)?;
                let ready_at = now.saturating_add(delay);
                for f in out.outbound {
                    self.queue_at(now, None, ready_at, deadline, &f);
                }
            }
            _ => return Err(Error::WrongInstance),
        }
        Ok(self.take_report())
    }
    pub fn poll(&mut self, now: u64, blob: Option<&AnnounceBlob>) -> Result<Report, Error> {
        self.time(now)?;
        self.expiry(now)?;
        match self.active().ok_or(Error::Inactive)?.instance {
            RETINUE => {
                let a = self
                    .retinue
                    .as_mut()
                    .ok_or(Error::WrongInstance)?
                    .poll(now, blob)?;
                self.retinue_actions(now, a)?;
            }
            TUCKET => {
                let due: Vec<_, 8> = self
                    .tucket
                    .as_ref()
                    .ok_or(Error::WrongInstance)?
                    .operations()
                    .filter(|op| op.retry_at <= now && op.attempts_remaining > 0)
                    .take(8)
                    .collect();
                for op in due {
                    if self.work.available() == 0 {
                        break;
                    }
                    if self.work.contains_operation(u64::from(op.id.0)) {
                        continue;
                    }
                    let finish = now
                        .checked_add(self.config.tx_budget_ms)
                        .ok_or(Error::TimeOverflow)?;
                    if finish >= op.expires_at
                        || finish > op.allowed_until
                        || finish > self.work_until()
                    {
                        continue;
                    }
                    if let Some(a) = self
                        .tucket
                        .as_mut()
                        .ok_or(Error::WrongInstance)?
                        .next_retry(now, op.id, finish)?
                    {
                        self.queue(
                            now,
                            Some(u64::from(op.id.0)),
                            op.expires_at.min(op.allowed_until),
                            &a.frame,
                        );
                    }
                }
            }
            SENNET => {
                self.queue_sennet_pending(now)?;
            }
            _ => return Err(Error::WrongInstance),
        }
        Ok(self.take_report())
    }
    pub fn send_sennet(
        &mut self,
        now: u64,
        header: sennet::transport::Header,
        text: &str,
    ) -> Result<Report, Error> {
        self.require(now, SENNET)?;
        if self.work.available() == 0 {
            return Err(WorkError::Full.into());
        }
        let deadline = self.ttl(now)?.min(self.work_until());
        if now
            .checked_add(self.config.tx_budget_ms)
            .ok_or(Error::TimeOverflow)?
            > deadline
        {
            return Err(WorkError::Expired.into());
        }
        self.sennet
            .as_mut()
            .ok_or(Error::WrongInstance)?
            .queue_text(now, header, text)?;
        self.queue_sennet_pending(now)?;
        Ok(self.take_report())
    }
    /// Directed PKI text uses a caller-generated extra nonce and the retained
    /// instance's existing durable packet-ID lease, never a fresh identity.
    pub fn send_sennet_directed(
        &mut self,
        now: u64,
        header: sennet::transport::Header,
        text: &str,
        extra: [u8; 4],
    ) -> Result<Report, Error> {
        self.require(now, SENNET)?;
        if self.work.available() == 0 {
            return Err(WorkError::Full.into());
        }
        let deadline = self.ttl(now)?.min(self.work_until());
        if now
            .checked_add(self.config.tx_budget_ms)
            .ok_or(Error::TimeOverflow)?
            > deadline
        {
            return Err(WorkError::Expired.into());
        }
        self.sennet
            .as_mut()
            .ok_or(Error::WrongInstance)?
            .queue_directed_text(now, header, text, extra)?;
        self.queue_sennet_pending(now)?;
        Ok(self.take_report())
    }
    fn queue_sennet_pending(&mut self, now: u64) -> Result<(), Error> {
        let a = self.require(now, SENNET)?;
        let deadline = self.ttl(now)?.min(self.work_until());
        if let Some(out) = self
            .sennet
            .as_mut()
            .ok_or(Error::WrongInstance)?
            .take_outbound(now)?
            && let Err(reason) = self.work.enqueue(
                now,
                a,
                Some(u64::from(out.operation_id)),
                deadline.min(out.expires_at),
                &out.frame,
            )
        {
            let e = self.sennet.as_mut().ok_or(Error::WrongInstance)?.fail_tx(
                now,
                out.operation_id,
                out.identity,
            )?;
            self.event(Event::Sennet(e));
            self.event(Event::FrameDropped {
                instance: SENNET,
                reason,
            });
        }
        Ok(())
    }
    pub fn send_tucket(
        &mut self,
        now: u64,
        to: u8,
        text: &str,
        policy: tucket::node::TextRetryPolicy,
        timing: tucket::instance::SendTiming,
    ) -> Result<tucket::instance::OperationId, Error> {
        self.require(now, TUCKET)?;
        if now
            .checked_add(self.config.tx_budget_ms)
            .ok_or(Error::TimeOverflow)?
            > self.work_until()
        {
            return Err(WorkError::Expired.into());
        }
        Ok(self
            .tucket
            .as_mut()
            .ok_or(Error::WrongInstance)?
            .begin_send(now, to, text, policy, timing)?)
    }
    pub fn advertise_tucket(
        &mut self,
        now: u64,
        timestamp: u32,
        data: &[u8],
    ) -> Result<Report, Error> {
        self.require(now, TUCKET)?;
        if data.len() > 32 {
            return Err(Error::FrameTooLong);
        }
        let f = self
            .tucket
            .as_mut()
            .ok_or(Error::WrongInstance)?
            .active_node_mut()?
            .advert_frame(timestamp, data);
        let d = self.ttl(now)?;
        self.queue(now, None, d, &f);
        Ok(self.take_report())
    }
    pub fn begin_tx(&mut self, now: u64) -> Result<Option<Transmission>, Error> {
        self.time(now)?;
        let a = self.active().ok_or(Error::Inactive)?;
        let Some(deadline) = self.work.first_deadline() else {
            return Ok(None);
        };
        let finish = now
            .checked_add(self.config.tx_budget_ms)
            .ok_or(Error::TimeOverflow)?;
        if finish >= deadline.min(self.work_until()) {
            return Ok(None);
        }
        Ok(self.work.begin(now, a)?)
    }
    pub fn complete_tx(
        &mut self,
        now: u64,
        id: WorkId,
        transmitted: bool,
    ) -> Result<Report, Error> {
        self.time(now)?;
        self.work.complete(now, id)?;
        if id.activation.instance == SENNET {
            let identity = self
                .sennet
                .as_mut()
                .ok_or(Error::WrongInstance)?
                .pending_identity()
                .ok_or(Error::Inactive)?;
            let op = id.operation.ok_or(Error::Inactive)? as u32;
            let e = if transmitted {
                self.sennet
                    .as_mut()
                    .ok_or(Error::WrongInstance)?
                    .complete_tx(now, op, identity)?
            } else {
                self.sennet
                    .as_mut()
                    .ok_or(Error::WrongInstance)?
                    .fail_tx(now, op, identity)?
            };
            self.event(Event::Sennet(e));
        }
        if !transmitted {
            self.event(Event::WorkLost(id));
        }
        Ok(self.take_report())
    }
}
