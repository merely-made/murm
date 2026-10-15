//! The router's inbound queue: four bounded lanes drained in strict priority (`Transport.py`
//! 46-95, 111-114, 1796-1912).
//!
//! Each packet is classified as it is queued, cheaply: announces take the announce lane, a path
//! request the path-request lane, or the ingress-limited lane while its interface is in a
//! path-request burst, and everything else the data lane. A full lane drops only its own
//! class, so an announce flood cannot crowd out link traffic. The router keeps the costly
//! checks (signatures, tag dedup) that RNS runs before queueing.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::Notify;

use crate::packet::{Packet, PacketType};

use super::iface_stats::{InterfaceStats, is_path_request};
use super::interface::InterfaceId;
use super::shared::Shared;

/// A traffic class, in drain order (`Transport.py` 111-114).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    Data,
    Announce,
    PathRequest,
    IngressLimited,
}

impl Lane {
    const COUNT: usize = 4;
    const ALL: [Self; Self::COUNT] = [
        Self::Data,
        Self::Announce,
        Self::PathRequest,
        Self::IngressLimited,
    ];

    /// RNS's default depths (`Transport.py` 91-94). A stream reader awaits room in the data
    /// lane, back-pressuring its peer; every other full lane drops.
    const fn depth(self) -> usize {
        match self {
            Self::Data => 1024,
            Self::Announce | Self::PathRequest => 128,
            Self::IngressLimited => 8,
        }
    }

    fn dropped(self, shared: &Shared) -> &AtomicU64 {
        let stats = &shared.routing_stats;
        match self {
            Self::Data => &stats.inbound_data_dropped,
            Self::Announce => &stats.inbound_announces_dropped,
            Self::PathRequest => &stats.inbound_path_requests_dropped,
            Self::IngressLimited => &stats.inbound_ingress_limited_dropped,
        }
    }
}

/// Why [`InboundTx::try_send`] did not queue a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrySendError {
    /// Its lane is full; the packet was dropped.
    Full,
    /// The router has stopped.
    Closed,
}

#[derive(Default)]
struct Lanes {
    queues: [VecDeque<(InterfaceId, Packet)>; Lane::COUNT],
    /// The router is gone.
    closed: bool,
}

struct Inner {
    lanes: Mutex<Lanes>,
    /// Wakes the router: one consumer, so a stored permit is never lost.
    ready: Notify,
    /// Wakes stream readers waiting for room in the data lane.
    room: Notify,
    senders: AtomicUsize,
    /// The endpoint, for the lane drop counters and each interface's stats.
    shared: OnceLock<Weak<Shared>>,
    /// Interface stats by id, resolved on first use: ids are never reused.
    stats: Mutex<HashMap<InterfaceId, Weak<InterfaceStats>>>,
}

/// The sending half, cloned into every interface.
pub(super) struct InboundTx {
    inner: Arc<Inner>,
}

/// A received packet, as the router takes it.
pub(super) struct Inbound {
    pub(super) iface: InterfaceId,
    pub(super) packet: Packet,
    /// Taken from the ingress-limited lane: a path request from an interface in a path-request
    /// burst, which must not start a recursive search (`Transport.py` 1859-1860, 3422, 3542).
    pub(super) ingress_limited: bool,
}

/// The router's receiving half.
pub(super) struct InboundRx {
    inner: Arc<Inner>,
}

pub(super) fn channel() -> (InboundTx, InboundRx) {
    let inner = Arc::new(Inner {
        lanes: Mutex::default(),
        ready: Notify::new(),
        room: Notify::new(),
        senders: AtomicUsize::new(1),
        shared: OnceLock::new(),
        stats: Mutex::default(),
    });
    let rx = InboundRx {
        inner: Arc::clone(&inner),
    };
    (InboundTx { inner }, rx)
}

impl Clone for InboundTx {
    fn clone(&self) -> Self {
        self.inner.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for InboundTx {
    fn drop(&mut self) {
        if self.inner.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.ready.notify_one();
        }
    }
}

impl InboundTx {
    /// Give the queue its endpoint, once, for drop counters and interface stats.
    pub(super) fn bind(&self, shared: &Arc<Shared>) {
        let _ = self.inner.shared.set(Arc::downgrade(shared));
    }

    /// Queue a packet. A data-lane packet waits for room, which back-pressures a stream
    /// reader; any other full lane drops, as RNS's `put(block=False)` does. False once the
    /// router has stopped.
    pub(super) async fn send(&self, iface: InterfaceId, pkt: Packet) -> bool {
        let lane = self.classify(iface, &pkt);
        if lane != Lane::Data {
            return self.offer(lane, iface, pkt) != Err(TrySendError::Closed);
        }
        let mut pkt = pkt;
        loop {
            let room = self.inner.room.notified();
            tokio::pin!(room);
            room.as_mut().enable();
            match self.push(lane, iface, pkt) {
                Ok(()) => return true,
                Err((TrySendError::Closed, _)) => return false,
                Err((TrySendError::Full, back)) => pkt = back,
            }
            room.await;
        }
    }

    /// Queue a packet without waiting; a full lane drops it.
    pub(super) fn try_send(&self, iface: InterfaceId, pkt: Packet) -> Result<(), TrySendError> {
        let lane = self.classify(iface, &pkt);
        self.offer(lane, iface, pkt)
    }

    fn offer(&self, lane: Lane, iface: InterfaceId, pkt: Packet) -> Result<(), TrySendError> {
        self.push(lane, iface, pkt).map_err(|(error, _)| {
            if error == TrySendError::Full
                && let Some(shared) = self.shared()
            {
                lane.dropped(&shared).fetch_add(1, Ordering::Relaxed);
            }
            error
        })
    }

    fn push(
        &self,
        lane: Lane,
        iface: InterfaceId,
        pkt: Packet,
    ) -> Result<(), (TrySendError, Packet)> {
        let mut lanes = self.inner.lanes.lock().unwrap();
        if lanes.closed {
            return Err((TrySendError::Closed, pkt));
        }
        let queue = &mut lanes.queues[lane as usize];
        if queue.len() >= lane.depth() {
            return Err((TrySendError::Full, pkt));
        }
        queue.push_back((iface, pkt));
        drop(lanes);
        self.inner.ready.notify_one();
        Ok(())
    }

    /// The packet's lane, counting announce and path-request receptions on the way
    /// (`Transport.py` 1803-1863; `Interface.py` 303-319). A flagged frame is left to the router,
    /// which refuses it.
    fn classify(&self, iface: InterfaceId, pkt: &Packet) -> Lane {
        if pkt.ifac {
            return Lane::Data;
        }
        if pkt.packet_type == PacketType::Announce {
            if let Some(stats) = self.stats(iface) {
                stats.note_announce_rx(pkt.encoded_len());
            }
            return Lane::Announce;
        }
        if !is_path_request(pkt) {
            return Lane::Data;
        }
        let Some(stats) = self.stats(iface) else {
            return Lane::PathRequest;
        };
        stats.note_path_request_rx(pkt.encoded_len());
        if stats.path_request_burst.load(Ordering::Relaxed) {
            Lane::IngressLimited
        } else {
            Lane::PathRequest
        }
    }

    fn shared(&self) -> Option<Arc<Shared>> {
        self.inner.shared.get()?.upgrade()
    }

    fn stats(&self, iface: InterfaceId) -> Option<Arc<InterfaceStats>> {
        if let Some(stats) = self.inner.stats.lock().unwrap().get(&iface)
            && let Some(stats) = stats.upgrade()
        {
            return Some(stats);
        }
        // Resolved outside the registry lock: the interface table has its own.
        let stats = self
            .shared()?
            .with_iface(iface, |record| Arc::clone(&record.stats))?;
        let mut registry = self.inner.stats.lock().unwrap();
        registry.retain(|_, cell| cell.strong_count() > 0);
        registry.insert(iface, Arc::downgrade(&stats));
        Some(stats)
    }
}

impl InboundRx {
    /// The next packet to route, highest lane first (`Transport.py` 70-80); `None` once every
    /// sender is gone.
    pub(super) async fn recv(&mut self) -> Option<Inbound> {
        // A busy router must still yield, as an mpsc receiver's budget makes it: the tasks it
        // wakes (interface writers, link sessions) wait in this worker's unstealable LIFO slot.
        tokio::task::coop::consume_budget().await;
        let mut yielded = false;
        loop {
            let ready = self.inner.ready.notified();
            tokio::pin!(ready);
            ready.as_mut().enable();
            match self.head() {
                Some(lane) if lane == Lane::Data || yielded => return self.pop(),
                // A control packet costs a signature check or a search: first let what the
                // router woke run, and re-check for data that arrived meanwhile.
                Some(_) => {
                    yielded = true;
                    tokio::task::yield_now().await;
                    continue;
                }
                None => {}
            }
            if self.inner.senders.load(Ordering::Acquire) == 0 {
                return None;
            }
            ready.await;
        }
    }

    fn head(&self) -> Option<Lane> {
        let lanes = self.inner.lanes.lock().unwrap();
        Lane::ALL
            .into_iter()
            .find(|&lane| !lanes.queues[lane as usize].is_empty())
    }

    fn pop(&self) -> Option<Inbound> {
        let mut lanes = self.inner.lanes.lock().unwrap();
        let (lane, (iface, packet)) = Lane::ALL
            .into_iter()
            .find_map(|lane| Some((lane, lanes.queues[lane as usize].pop_front()?)))?;
        drop(lanes);
        if lane == Lane::Data {
            self.inner.room.notify_one();
        }
        Some(Inbound {
            iface,
            packet,
            ingress_limited: lane == Lane::IngressLimited,
        })
    }

    /// Packets waiting in each lane, in drain order.
    #[cfg(test)]
    pub(super) fn heights(&self) -> [usize; Lane::COUNT] {
        let lanes = self.inner.lanes.lock().unwrap();
        core::array::from_fn(|i| lanes.queues[i].len())
    }
}

impl Drop for InboundRx {
    fn drop(&mut self) {
        let mut lanes = self.inner.lanes.lock().unwrap();
        lanes.closed = true;
        lanes.queues.iter_mut().for_each(VecDeque::clear);
        drop(lanes);
        self.inner.room.notify_waiters();
    }
}
