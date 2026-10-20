//! Single-profile Tucket endpoint. Retains no other protocol's inline storage.
//! The hardware owner confirms RX before construction, drains RX before TX,
//! and resets on uncertain driver cancellation. All times are monotonic ms.
use crate::work::{Transmission, WorkError, WorkId, WorkQueue};
use heapless::Vec;
use selvage::personality::PersonalityId;
use tucket::instance::{Instance, InstanceError, OperationId, SendTiming};
use tucket::node::TextRetryPolicy;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Clock,
    Overflow,
    FrameTooLong,
    Protocol(InstanceError),
    Work(WorkError),
}
impl From<InstanceError> for Error {
    fn from(e: InstanceError) -> Self {
        Self::Protocol(e)
    }
}
impl From<WorkError> for Error {
    fn from(e: WorkError) -> Self {
        Self::Work(e)
    }
}
#[derive(Debug)]
pub enum Event {
    Tucket(tucket::node::Event),
    TucketAcknowledged(OperationId),
    TucketLost(OperationId),
    WorkLost(WorkId),
    WorkCancelled(WorkId),
    FrameDropped(WorkError),
}
#[derive(Default)]
pub struct Report {
    pub events: Vec<Event, 32>,
    pub overflowed: usize,
}
impl Report {
    pub fn format(&self) -> heapless::String<2048> {
        use core::fmt::Write;
        let mut line = heapless::String::<2048>::new();
        let mut omitted = 0;
        for event in &self.events {
            let start = line.len();
            if writeln!(line, "resident event {event:?}").is_err() || line.len() > 1920 {
                line.truncate(start);
                omitted += 1;
            }
        }
        writeln!(
            line,
            "resident report events={} dropped={} omitted={omitted}",
            self.events.len(),
            self.overflowed
        )
        .expect("reserved footer");
        line
    }
    fn push(&mut self, e: Event) {
        if self.events.push(e).is_err() {
            self.overflowed += 1;
        }
    }
}
pub struct Endpoint {
    instance: Instance,
    work: WorkQueue<8>,
    now: u64,
    budget: u64,
    ttl: u64,
}

impl Endpoint {
    pub fn new(now: u64, instance: Instance, budget: u64, ttl: u64) -> Result<Self, Error> {
        if budget == 0 || ttl <= budget || instance.is_paused() {
            return Err(Error::Configuration);
        }
        let mut work = WorkQueue::new(now);
        work.activate(now, PersonalityId(2))?;
        Ok(Self {
            instance,
            work,
            now,
            budget,
            ttl,
        })
    }
    pub fn instance(&self) -> &Instance {
        &self.instance
    }
    pub fn budget(&self) -> u64 {
        self.budget
    }
    pub fn next_deadline(&self) -> Option<u64> {
        [self.instance.next_deadline(), self.work.next_deadline()]
            .into_iter()
            .flatten()
            .min()
    }
    fn time(&mut self, now: u64) -> Result<(), Error> {
        if now < self.now {
            return Err(Error::Clock);
        }
        self.now = now;
        Ok(())
    }
    fn queue(
        &mut self,
        now: u64,
        op: Option<u64>,
        ready: u64,
        until: u64,
        frame: &[u8],
        out: &mut Report,
    ) {
        if let Err(e) = self.work.enqueue_at(
            now,
            self.work.activation().unwrap(),
            op,
            ready,
            until,
            frame,
        ) {
            out.push(Event::FrameDropped(e));
        }
    }
    fn expire(&mut self, now: u64, out: &mut Report) -> Result<(), Error> {
        self.time(now)?;
        for id in self.work.expire(now)? {
            out.push(Event::WorkLost(id));
        }
        for op in self.instance.advance(now)?.operations {
            for id in self.work.cancel_operation(now, u64::from(op.0))? {
                out.push(Event::WorkLost(id));
            }
            out.push(Event::TucketLost(op));
        }
        Ok(())
    }
    /// Call poll before commands at the same timestamp to collect expiry first.
    pub fn send(
        &mut self,
        now: u64,
        to: u8,
        text: &str,
        policy: TextRetryPolicy,
        timing: SendTiming,
    ) -> Result<OperationId, Error> {
        self.time(now)?;
        Ok(self.instance.begin_send(now, to, text, policy, timing)?)
    }
    pub fn advertise(&mut self, now: u64, timestamp: u32, data: &[u8]) -> Result<Report, Error> {
        self.time(now)?;
        if data.len() > 32 {
            return Err(Error::FrameTooLong);
        }
        let until = now.checked_add(self.ttl).ok_or(Error::Overflow)?;
        let frame = self
            .instance
            .active_node_mut()?
            .advert_frame(timestamp, data);
        let mut out = Report::default();
        self.queue(now, None, now, until, &frame, &mut out);
        Ok(out)
    }
    pub fn ingest(&mut self, now: u64, frame: &[u8]) -> Result<Report, Error> {
        self.time(now)?;
        if frame.len() > selvage::MAX_RADIO_FRAME_LEN {
            return Err(Error::FrameTooLong);
        }
        let mut out = Report::default();
        self.expire(now, &mut out)?;
        let incoming = self.instance.on_frame(now, frame)?;
        for op in incoming.acknowledged {
            for id in self.work.cancel_operation(now, u64::from(op.0))? {
                out.push(Event::WorkCancelled(id));
            }
            out.push(Event::TucketAcknowledged(op));
        }
        for e in incoming.events {
            out.push(Event::Tucket(e));
        }
        // advance above has already collected the same timestamp's expiry.
        debug_assert!(incoming.expired.operations.is_empty());
        let ready = now
            .checked_add(self.instance.reply_delay_ms())
            .ok_or(Error::Overflow)?;
        let until = now.checked_add(self.ttl).ok_or(Error::Overflow)?;
        for frame in incoming.outbound {
            self.queue(now, None, ready, until, &frame, &mut out);
        }
        Ok(out)
    }
    pub fn poll(&mut self, now: u64) -> Result<Report, Error> {
        let mut out = Report::default();
        self.expire(now, &mut out)?;
        let due: Vec<_, 2> = self
            .instance
            .operations()
            .filter(|op| op.retry_at <= now && op.attempts_remaining > 0)
            .take(2)
            .collect();
        for op in due {
            if self.work.available() == 0 {
                break;
            }
            if self.work.contains_operation(u64::from(op.id.0)) {
                continue;
            }
            let finish = now.checked_add(self.budget).ok_or(Error::Overflow)?;
            if finish >= op.expires_at || finish > op.allowed_until {
                continue;
            }
            if let Some(attempt) = self.instance.next_retry(now, op.id, finish)? {
                self.queue(
                    now,
                    Some(u64::from(op.id.0)),
                    now,
                    op.expires_at.min(op.allowed_until),
                    &attempt.frame,
                    &mut out,
                );
            }
        }
        Ok(out)
    }
    pub fn begin_tx(&mut self, now: u64) -> Result<Option<Transmission>, Error> {
        self.time(now)?;
        let Some(until) = self.work.first_deadline() else {
            return Ok(None);
        };
        if now.checked_add(self.budget).ok_or(Error::Overflow)? >= until {
            return Ok(None);
        }
        Ok(self.work.begin(now, self.work.activation().unwrap())?)
    }
    pub fn complete_tx(&mut self, now: u64, id: WorkId, sent: bool) -> Result<Report, Error> {
        self.time(now)?;
        self.work.complete(now, id)?;
        let mut out = Report::default();
        if !sent {
            out.push(Event::WorkLost(id));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tucket::{
        identity::LocalIdentity,
        instance::InstanceConfig,
        node::{Node, NodeCapacity},
    };
    fn endpoint(seed: u8, peers: &[[u8; 32]]) -> Endpoint {
        let mut instance = Instance::new(
            Node::with_capacity(
                LocalIdentity::from_seed([seed; 32]),
                false,
                NodeCapacity::new(8, 32).unwrap(),
            )
            .unwrap(),
            InstanceConfig::for_radio(2, 10, 200).unwrap(),
        );
        instance.restore_contacts(peers).unwrap();
        Endpoint::new(0, instance, 10, 10_000).unwrap()
    }
    fn key(seed: u8) -> [u8; 32] {
        LocalIdentity::from_seed([seed; 32]).identity().pub_key
    }
    #[test]
    fn queue_refusal_expiry_and_exact_failed_completion() {
        let mut e = endpoint(3, &[]);
        for _ in 0..8 {
            assert!(e.advertise(0, 1, b"bench").unwrap().events.is_empty());
        }
        assert!(matches!(
            e.advertise(0, 2, b"").unwrap().events.as_slice(),
            [Event::FrameDropped(WorkError::Full)]
        ));
        let tx = e.begin_tx(0).unwrap().unwrap();
        let mut wrong = tx.id;
        wrong.sequence += 1;
        assert_eq!(
            e.complete_tx(0, wrong, false).err(),
            Some(Error::Work(WorkError::WrongCompletion))
        );
        assert!(
            matches!(e.complete_tx(0,tx.id,false).unwrap().events.as_slice(),[Event::WorkLost(id)] if *id==tx.id)
        );
        assert_eq!(e.poll(10_000).unwrap().events.len(), 7);
        assert!(e.begin_tx(10_000).unwrap().is_none());
        assert_eq!(e.poll(9999).err(), Some(Error::Clock));
        assert_eq!(e.ingest(10_000, &[0; 256]).err(), Some(Error::FrameTooLong));
    }
    #[test]
    fn restored_peers_exchange_text_and_ack_without_adverts() {
        let mut a = endpoint(3, &[key(7)]);
        let mut b = endpoint(7, &[key(3)]);
        let id = a
            .send(
                0,
                key(7)[0],
                "restored",
                TextRetryPolicy::new(3, true).unwrap(),
                SendTiming {
                    timestamp: 1,
                    expires_at: 10_000,
                    allowed_until: 10_000,
                },
            )
            .unwrap();
        a.poll(0).unwrap();
        let tx = a.begin_tx(0).unwrap().unwrap();
        a.complete_tx(10, tx.id, true).unwrap();
        let out = b.ingest(10, &tx.frame).unwrap();
        assert!(
            out.events
                .iter()
                .any(|e| matches!(e, Event::Tucket(tucket::node::Event::Message { .. })))
        );
        assert!(
            b.begin_tx(209).unwrap().is_none(),
            "reply turnaround must hold"
        );
        let mut ack = false;
        for now in [210, 220, 230, 240, 250, 260, 470, 480] {
            for direction in 0..2 {
                let (sender, receiver) = if direction == 0 {
                    (&mut b, &mut a)
                } else {
                    (&mut a, &mut b)
                };
                sender.poll(now).unwrap();
                if let Some(tx) = sender.begin_tx(now).unwrap() {
                    sender.complete_tx(now, tx.id, true).unwrap();
                    let out = receiver.ingest(now, &tx.frame).unwrap();
                    ack |= out
                        .events
                        .iter()
                        .any(|e| matches!(e,Event::TucketAcknowledged(op) if *op==id));
                }
            }
        }
        assert!(ack);
        assert_eq!(a.instance().pending_len(), 0);
    }
    #[test]
    fn pending_capacity_and_expiry_cancel_unsent_retry() {
        let mut e = endpoint(3, &[key(7)]);
        for n in 0..2 {
            e.send(
                0,
                key(7)[0],
                "pending",
                TextRetryPolicy::new(2, true).unwrap(),
                SendTiming {
                    timestamp: n + 1,
                    expires_at: 1000,
                    allowed_until: 1000,
                },
            )
            .unwrap();
        }
        assert_eq!(
            e.send(
                0,
                key(7)[0],
                "third",
                TextRetryPolicy::default(),
                SendTiming {
                    timestamp: 3,
                    expires_at: 1000,
                    allowed_until: 1000
                }
            )
            .err(),
            Some(Error::Protocol(InstanceError::PendingFull))
        );
        e.poll(0).unwrap();
        let out = e.poll(1000).unwrap();
        assert_eq!(
            out.events
                .iter()
                .filter(|e| matches!(e, Event::TucketLost(_)))
                .count(),
            2
        );
        assert!(e.begin_tx(1000).unwrap().is_none());
    }
}
