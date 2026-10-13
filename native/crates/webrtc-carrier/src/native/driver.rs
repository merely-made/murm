// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The Sans-IO driver: the loop str0m does not write for you.
//!
//! str0m owns no socket, no runtime, and no timer. Everything below is the part
//! that fills those three holes, and every interesting decision in this file is
//! forced by that contract rather than chosen for taste.
//!
//! ## The single-mutation invariant
//!
//! str0m states one rule: *every* mutation of an `Rtc` must be followed by a
//! complete drain of `poll_output` until it returns `Output::Timeout`, before
//! the next mutation. The loop below is written so that rule is visible rather
//! than merely obeyed:
//!
//! ```text
//! (A) drain  -> deadline          poll_output until Timeout
//! (B) gauge                       read SCTP depth, decide pause/resume
//! (C) deliver                     hand decoded frames to the reader
//! (D) flush                       at most ONE channel write, then goto (A)
//! (E) close                       at most ONE rtc.close(), then goto (A)
//! (F) wait   -> one event         then at most ONE handle_input, goto (A)
//! ```
//!
//! Steps (D), (E) and (F) each perform at most one mutation and then `continue`,
//! so control always returns to the drain. Steps (B) and (C) touch queues, not
//! the engine. Taking a frame off the hand-off queue in (F) does *not* write it
//! — it parks it for (D) on the next turn — because writing it there would be a
//! second mutation in the same turn.
//!
//! ## What drives the loop
//!
//! Four things, and `select!` takes exactly one per turn:
//!
//! 1. the control command (cancel or close), biased first so a cancellation is
//!    never starved by a busy socket;
//! 2. a datagram off the UDP socket, but only while the reader has room — that
//!    is how inbound backpressure reaches the peer, by declining to read rather
//!    than by buffering without bound;
//! 3. a frame from the writer, but only while the queue is below the
//!    high-water mark and no frame is already held;
//! 4. the timer.
//!
//! ## Timer granularity
//!
//! str0m returns an absolute `Instant`, sometimes already in the past when it
//! wants an immediate turn. Feeding that back without a floor is exactly the
//! hot loop C1 names as a failure condition, so the wait is clamped:
//!
//! - **floor of 1 ms.** str0m's own async guidance clamps the same way. It caps
//!   the loop at a thousand turns a second even in the pathological case where
//!   the engine asks for "now" forever, and costs a millisecond of scheduling
//!   latency that DTLS and SCTP retransmit timers do not notice.
//! - **ceiling of 1 s.** Not needed for responsiveness — cancellation has its
//!   own branch — but it gives the disconnect deadline and the closing flush a
//!   tick to be noticed on, without a second timer.
//! - **5 ms while something is held or the sender is paused.** A frame SCTP
//!   declined, a frame *this loop* declined to hand SCTP because its window is
//!   full, a decoded frame the reader has no room for, and a sender parked at
//!   the high-water mark all need a turn the engine has no reason to
//!   schedule. Two hundred retries a second is a cadence, not a spin. The
//!   paused case is the one that bites: without it a sustained transfer stalls
//!   for up to `MAX_WAIT` on every pause cycle, because the resume decision is
//!   only taken once per turn.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use str0m::channel::{ChannelData, ChannelId, Reliability};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output, Rtc, RtcError};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};

use crate::error::FrameError;
use crate::fingerprint::FingerprintRole;
use crate::frame::{MAX_FRAME_BYTES, decode_frame};
use crate::native::answerer::fingerprint_from_str0m;
use crate::native::error::NativeError;
use crate::native::session::{
    CarrierConfig, Command, Delivery, SessionFingerprints, Shared, Terminal,
};

/// Read buffer for one datagram. str0m targets a ~1200-byte MTU; this is the
/// size its own examples use.
const SOCKET_BUFFER_BYTES: usize = 2048;

/// The floor on the wait. See the module docs.
const MIN_WAIT: Duration = Duration::from_millis(1);

/// The ceiling on the wait. See the module docs.
const MAX_WAIT: Duration = Duration::from_secs(1);

/// Retry cadence while a frame is held on either side.
const FLUSH_RETRY: Duration = Duration::from_millis(5);

/// How long a polite close waits for the outbound queue to empty.
const CLOSE_FLUSH_LIMIT: Duration = Duration::from_secs(5);

/// How long the exit path waits for the reader to take what was decoded.
const FINISH_DELIVER_LIMIT: Duration = Duration::from_secs(2);

/// Consecutive "peer is gone" socket errors tolerated before ending.
///
/// On Windows a UDP socket reports one `ConnectionReset` per datagram that drew
/// an ICMP port-unreachable, so these are ordinarily self-limiting. The streak
/// is the guard for the case where they are not: an unbounded stream of instant
/// errors is a hot loop by another name.
const PEER_GONE_STREAK: u64 = 512;

/// Ceiling on the receive buffer.
///
/// After a full drain the residue is always shorter than one maximum frame —
/// anything longer would have decoded. So the buffer can only exceed that by
/// one inbound SCTP message, and str0m accepts at most 256 KiB. Past this the
/// invariant is broken, and saying so is better than growing.
const MAX_RECEIVE_BUFFER_BYTES: usize = MAX_FRAME_BYTES + 256 * 1024;

fn engine(err: RtcError) -> NativeError {
    NativeError::Engine(err.to_string())
}

/// A socket error that means "the peer is not there", not "the socket broke".
fn peer_is_gone(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused
    )
}

/// What one turn of the loop woke up for.
enum Wake {
    /// The control channel changed; `false` means every sender is gone.
    Command(bool),
    /// A datagram, or the error the socket gave instead.
    Datagram(std::io::Result<(usize, SocketAddr)>),
    /// A frame from the writer; `None` means every writer is gone.
    Outbound(Option<Vec<u8>>),
    /// The timer.
    Timeout,
}

pub(crate) struct Driver {
    rtc: Rtc,
    socket: Arc<UdpSocket>,
    /// The local address str0m is told each datagram was addressed to.
    ///
    /// Not necessarily the socket's own address: see
    /// [`serve_advertised`](crate::native::serve_advertised).
    advertised_addr: SocketAddr,
    shared: Arc<Shared>,
    command_rx: watch::Receiver<Command>,
    resume_tx: watch::Sender<u64>,
    outbound_rx: mpsc::Receiver<Vec<u8>>,
    delivery: Delivery,
    opened: Option<oneshot::Sender<()>>,

    channel: Option<ChannelId>,
    rx_buf: Vec<u8>,
    pending_out: Option<Vec<u8>>,
    outbound_open: bool,
    paused: bool,
    closing: bool,
    close_started: bool,
    close_deadline: Option<Instant>,
    disconnect_deadline: Option<Instant>,
    idle_timeout: Duration,
    sctp_window: usize,
    local_role: FingerprintRole,
    finished: bool,
    reset_streak: u64,
    tick: u64,
    published_queue: usize,
}

impl Driver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rtc: Rtc,
        socket: Arc<UdpSocket>,
        advertised_addr: SocketAddr,
        shared: Arc<Shared>,
        command_rx: watch::Receiver<Command>,
        resume_tx: watch::Sender<u64>,
        outbound_rx: mpsc::Receiver<Vec<u8>>,
        inbound_tx: mpsc::Sender<Vec<u8>>,
        opened: oneshot::Sender<()>,
        config: &CarrierConfig,
    ) -> Self {
        Self {
            rtc,
            socket,
            advertised_addr,
            shared,
            command_rx,
            resume_tx,
            outbound_rx,
            delivery: Delivery {
                pending: VecDeque::new(),
                hold_limit: config.inbound_queue_frames.max(1),
                tx: inbound_tx,
                closed: false,
            },
            opened: Some(opened),
            channel: None,
            rx_buf: Vec::new(),
            pending_out: None,
            outbound_open: true,
            paused: false,
            closing: false,
            close_started: false,
            close_deadline: None,
            disconnect_deadline: None,
            idle_timeout: config.idle_timeout,
            sctp_window: config.sctp_window_bytes.max(1),
            local_role: config.local_dtls_role,
            finished: false,
            reset_streak: 0,
            tick: 0,
            published_queue: 0,
        }
    }

    /// Runs the loop and records why it stopped.
    ///
    /// The terminal reason is published *before* this returns, so the channels
    /// this struct owns are still open when a reader or writer goes looking for
    /// it. Dropping them afterwards is what wakes those handles.
    pub(crate) async fn run(mut self) -> Result<(), NativeError> {
        let result = self.drive().await;
        self.flush_delivery_on_exit().await;

        let terminal = match &result {
            Ok(()) => Terminal::Closed,
            Err(NativeError::Cancelled) => Terminal::Cancelled,
            Err(NativeError::Write(text)) => Terminal::Write(text.clone()),
            Err(NativeError::Frame(err)) => Terminal::Frame(err.clone()),
            Err(other) => Terminal::Failed(other.to_string()),
        };
        self.shared.set_terminal(terminal);

        result
    }

    async fn drive(&mut self) -> Result<(), NativeError> {
        let socket = Arc::clone(&self.socket);
        let mut buf = vec![0u8; SOCKET_BUFFER_BYTES];

        loop {
            self.shared
                .counters
                .iterations
                .fetch_add(1, Ordering::Relaxed);

            // (A) Drain str0m completely. Nothing below may mutate the engine
            //     until this has returned a timeout.
            let deadline = self.drain(&socket).await?;

            if !self.rtc.is_alive() || self.finished {
                return Ok(());
            }
            if self
                .disconnect_deadline
                .is_some_and(|at| Instant::now() >= at)
            {
                return Ok(());
            }
            if self.reset_streak >= PEER_GONE_STREAK {
                return Ok(());
            }

            // (B) Republish the queue gauge; re-decide pause and resume.
            self.refresh_queue();

            // (C) Hand the reader whatever it has room for.
            self.deliver();
            if self.delivery.closed {
                return Ok(());
            }

            // (D) One write attempt for a held frame, then back to the drain.
            if self.pending_out.is_some() && self.try_flush()? {
                continue;
            }

            // (E) One close, once everything queued is out or time ran out.
            if self.closing && !self.close_started && self.close_drained() {
                self.rtc.close().map_err(engine)?;
                self.close_started = true;
                continue;
            }

            // (F) Wait for exactly one thing.
            let now = Instant::now();
            let wait = self.wait_deadline(deadline, now);
            // Deliberately NOT gated on `self.paused`. The high-water mark
            // stops the *application* handing over more work; it must never
            // stop the driver moving work already handed over towards the
            // wire. Gating here deadlocks: the bytes sitting in the hand-off
            // queue are counted in the gauge, only this branch can drain them,
            // and a driver that declines to drain them keeps the gauge above
            // the mark forever. The real brake on the wire side is SCTP
            // declining the write, which `try_flush` already honours.
            let take_outbound =
                self.outbound_open && self.pending_out.is_none() && self.channel.is_some();
            let read_socket = self.delivery.pending.len() < self.delivery.hold_limit;

            let wake = tokio::select! {
                biased;
                changed = self.command_rx.changed() => Wake::Command(changed.is_ok()),
                got = socket.recv_from(&mut buf), if read_socket => Wake::Datagram(got),
                frame = self.outbound_rx.recv(), if take_outbound => Wake::Outbound(frame),
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(wait)) => Wake::Timeout,
            };

            // ... and perform at most one mutation for it.
            match wake {
                Wake::Command(false) => return Err(NativeError::Cancelled),
                Wake::Command(true) => {
                    let command = *self.command_rx.borrow_and_update();
                    match command {
                        Command::Cancel => return Err(NativeError::Cancelled),
                        Command::Close => {
                            if !self.closing {
                                self.closing = true;
                                self.close_deadline = Some(Instant::now() + CLOSE_FLUSH_LIMIT);
                            }
                        },
                        Command::Run => {},
                    }
                },
                Wake::Datagram(Ok((len, source))) => {
                    self.reset_streak = 0;
                    self.shared
                        .counters
                        .datagrams_received
                        .fetch_add(1, Ordering::Relaxed);
                    match Receive::new(Protocol::Udp, source, self.advertised_addr, &buf[..len]) {
                        Ok(receive) => {
                            self.rtc
                                .handle_input(Input::Receive(Instant::now(), receive))
                                .map_err(engine)?;
                        },
                        Err(_) => {
                            // Not STUN, DTLS, RTP or RTCP. A stray datagram on
                            // a public UDP port is ordinary; it is counted and
                            // dropped, never fed to the engine.
                            self.shared
                                .counters
                                .malformed_datagrams
                                .fetch_add(1, Ordering::Relaxed);
                        },
                    }
                },
                Wake::Datagram(Err(err)) if peer_is_gone(&err) => {
                    self.reset_streak += 1;
                    self.shared
                        .counters
                        .socket_resets
                        .fetch_add(1, Ordering::Relaxed);
                },
                Wake::Datagram(Err(err)) => return Err(NativeError::Socket(err)),
                Wake::Outbound(Some(frame)) => self.pending_out = Some(frame),
                Wake::Outbound(None) => self.outbound_open = false,
                Wake::Timeout => {
                    self.shared
                        .counters
                        .timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    self.rtc
                        .handle_input(Input::Timeout(Instant::now()))
                        .map_err(engine)?;
                },
            }
        }
    }

    /// Steps 3 to 5 of str0m's canonical shape: poll, handle, repeat until the
    /// engine reports its next deadline.
    async fn drain(&mut self, socket: &UdpSocket) -> Result<Instant, NativeError> {
        loop {
            match self.rtc.poll_output().map_err(engine)? {
                Output::Timeout(at) => return Ok(at),
                Output::Transmit(transmit) => {
                    self.shared
                        .counters
                        .datagrams_sent
                        .fetch_add(1, Ordering::Relaxed);
                    match socket
                        .send_to(&transmit.contents, transmit.destination)
                        .await
                    {
                        Ok(_) => self.reset_streak = 0,
                        Err(err) if peer_is_gone(&err) => {
                            self.reset_streak += 1;
                            self.shared
                                .counters
                                .socket_resets
                                .fetch_add(1, Ordering::Relaxed);
                        },
                        Err(err) => return Err(NativeError::Socket(err)),
                    }
                },
                Output::Event(event) => self.on_event(event)?,
            }
        }
    }

    fn on_event(&mut self, event: Event) -> Result<(), NativeError> {
        match event {
            Event::ChannelOpen(id, label) => self.on_channel_open(id, label)?,
            Event::ChannelData(data) => self.on_channel_data(data)?,
            Event::ChannelClose(id) => {
                if self.channel == Some(id) {
                    self.channel = None;
                    self.finished = true;
                }
            },
            Event::ChannelBufferedAmountLow(id) => {
                if self.channel == Some(id) {
                    self.shared
                        .counters
                        .low_water_events
                        .fetch_add(1, Ordering::Relaxed);
                }
            },
            Event::Connected => self.capture_fingerprints()?,
            Event::Closed => self.finished = true,
            Event::IceConnectionStateChange(state) => match state {
                IceConnectionState::Disconnected => {
                    self.disconnect_deadline = Some(Instant::now() + self.idle_timeout);
                },
                IceConnectionState::Connected | IceConnectionState::Completed => {
                    self.disconnect_deadline = None;
                },
                _ => {},
            },
            _ => {},
        }
        Ok(())
    }

    /// Records both DTLS fingerprints, once the handshake has actually run.
    ///
    /// str0m publishes the peer's only after the certificate has been presented
    /// and matched against the SDP's promise, so this cannot be done at
    /// signalling time — and doing it here means the transcript binds the
    /// certificates in use rather than the ones announced.
    ///
    /// A peer whose fingerprint is not a 32-byte SHA-256 digest fails the
    /// session closed. The alternative is a transcript this crate cannot form,
    /// which is not a session worth continuing.
    fn capture_fingerprints(&mut self) -> Result<(), NativeError> {
        let local = self.rtc.direct_api().local_dtls_fingerprint().clone();
        let Some(remote) = self.rtc.direct_api().remote_dtls_fingerprint().cloned() else {
            return Ok(());
        };

        let (client, server) = match self.local_role {
            FingerprintRole::Server => (
                fingerprint_from_str0m(FingerprintRole::Client, &remote)?,
                fingerprint_from_str0m(FingerprintRole::Server, &local)?,
            ),
            FingerprintRole::Client => (
                fingerprint_from_str0m(FingerprintRole::Client, &local)?,
                fingerprint_from_str0m(FingerprintRole::Server, &remote)?,
            ),
        };
        self.shared
            .set_fingerprints(SessionFingerprints::new(client, server));
        Ok(())
    }

    fn on_channel_open(&mut self, id: ChannelId, label: String) -> Result<(), NativeError> {
        if self.channel.is_some() || label != self.shared.channel_label {
            // One session, one channel, one label. A second channel or a
            // different label is closed rather than quietly ignored: an
            // unattended open stream is a resource the peer controls.
            self.rtc.direct_api().close_data_channel(id);
            return Ok(());
        }

        // Read the negotiated policy out before the borrow ends.
        let policy = self
            .rtc
            .channel(id)
            .and_then(|channel| channel.config().map(|cfg| (cfg.ordered, cfg.reliability)));

        if let Some((ordered, reliability)) = policy {
            if !ordered || reliability != Reliability::Reliable {
                return Err(NativeError::UnreliableChannel {
                    label,
                    ordered,
                    reliability: format!("{reliability:?}"),
                });
            }
        }

        self.channel = Some(id);
        if let Some(mut channel) = self.rtc.channel(id) {
            // str0m raises ChannelBufferedAmountLow at this threshold, which is
            // the same signal the browser gets from
            // `bufferedAmountLowThreshold`. The loop does not depend on it —
            // the gauge is recomputed every turn — but it makes the low mark
            // observable rather than merely configured.
            channel.set_buffered_amount_low_threshold(self.shared.backpressure.low_water_bytes());
        }
        if let Some(tx) = self.opened.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    fn on_channel_data(&mut self, data: ChannelData) -> Result<(), NativeError> {
        if self.channel != Some(data.id) {
            return Ok(());
        }

        self.rx_buf.extend_from_slice(&data.data);
        if self.rx_buf.len() > MAX_RECEIVE_BUFFER_BYTES {
            return Err(NativeError::ReceiveOverflow {
                len: self.rx_buf.len(),
                max: MAX_RECEIVE_BUFFER_BYTES,
            });
        }

        let mut consumed = 0usize;
        loop {
            // `decode_frame` reads the four-byte prefix and compares it against
            // the ceiling before it looks at a payload. An oversize declaration
            // therefore lands here as `Oversize` with nothing reserved for the
            // payload it announced — which is the whole reason the check is a
            // header check and not a length check after the read.
            match decode_frame(&self.rx_buf[consumed..]) {
                Ok((payload, used)) => {
                    let frame = payload.to_vec();
                    consumed += used;
                    self.shared
                        .counters
                        .frames_received
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared
                        .counters
                        .bytes_received
                        .fetch_add(used as u64, Ordering::Relaxed);
                    self.delivery.pending.push_back(frame);
                },
                Err(FrameError::Incomplete { .. } | FrameError::ShortHeader { .. }) => break,
                Err(err) => {
                    self.rx_buf.drain(..consumed);
                    return Err(NativeError::Frame(err));
                },
            }
        }
        self.rx_buf.drain(..consumed);
        Ok(())
    }

    fn deliver(&mut self) {
        while !self.delivery.pending.is_empty() {
            match self.delivery.tx.try_reserve() {
                Ok(permit) => {
                    let frame = self
                        .delivery
                        .pending
                        .pop_front()
                        .expect("the queue was checked non-empty");
                    permit.send(frame);
                },
                Err(mpsc::error::TrySendError::Full(())) => break,
                Err(mpsc::error::TrySendError::Closed(())) => {
                    // The reader is gone. Reading is not optional on a session
                    // carrying a protocol, so this ends it rather than
                    // discarding the peer's frames indefinitely.
                    self.shared
                        .counters
                        .frames_dropped
                        .fetch_add(self.delivery.pending.len() as u64, Ordering::Relaxed);
                    self.delivery.pending.clear();
                    self.delivery.closed = true;
                    break;
                },
            }
        }
    }

    /// Re-reads the outbound queue and re-decides pause and resume.
    ///
    /// The gauge is `queued_outbound + sctp_buffered`: bytes this crate is
    /// holding, plus bytes str0m's SCTP association has accepted and not yet
    /// flushed. **Do not simplify this to one source.** str0m stops accepting
    /// writes at 128 KiB across all streams (`MAX_BUFFERED_ACROSS_STREAMS`),
    /// which is below this crate's default high-water mark of eight maximum
    /// frames — so a gauge reading only SCTP's depth can never reach the
    /// configured mark, and the whole policy becomes decoration that no test
    /// can catch. Counting only our own queue is wrong the other way: the
    /// bytes SCTP is sitting on are still owed to the peer.
    fn refresh_queue(&mut self) {
        let buffered = match self.channel {
            Some(id) => self
                .rtc
                .channel(id)
                .map_or(0, |mut channel| channel.buffered_amount()),
            None => 0,
        };
        self.shared
            .counters
            .sctp_buffered
            .store(buffered, Ordering::Relaxed);

        let queued = self.shared.counters.queued_bytes();
        self.shared.counters.record_peak(queued);

        let was_paused = self.paused;
        if self.paused {
            if self.shared.backpressure.should_resume(queued) {
                self.paused = false;
                self.shared.counters.resumes.fetch_add(1, Ordering::Relaxed);
            }
        } else if self.shared.backpressure.should_pause(queued) {
            self.paused = true;
            self.shared.counters.pauses.fetch_add(1, Ordering::Relaxed);
        }

        // Wake waiting writers on a state change as well as a size change. A
        // resume implies the size moved, so the second condition is redundant
        // today — but it is the one a writer is actually blocked on, and
        // leaving it implicit makes the wakeup depend on an invariant two
        // functions apart.
        if queued != self.published_queue || was_paused != self.paused {
            self.published_queue = queued;
            self.tick = self.tick.wrapping_add(1);
            let _ = self.resume_tx.send(self.tick);
        }
    }

    fn try_flush(&mut self) -> Result<bool, NativeError> {
        let Some(id) = self.channel else {
            return Ok(false);
        };
        let Some(frame) = self.pending_out.take() else {
            return Ok(false);
        };
        let Some(mut channel) = self.rtc.channel(id) else {
            self.pending_out = Some(frame);
            return Ok(false);
        };

        // The SCTP window ceiling. See `CarrierConfig::sctp_window_bytes`: this
        // is the one place that decides how much SCTP is holding at once, and
        // therefore how deep str0m's own `do_poll_output` recursion can go.
        //
        // `buffered_amount` is released on *acknowledgement*, not on send, so
        // it is the outstanding window rather than a send queue: capping it
        // caps both what is waiting and what is in flight, which is exactly
        // what the burst size follows. Declining leaves the frame ours, still
        // counted as queued, retried on the next turn — the same contract as
        // SCTP declining below.
        if channel.buffered_amount() >= self.sctp_window {
            self.pending_out = Some(frame);
            self.shared
                .counters
                .window_holds
                .fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        match channel.write(true, &frame) {
            Ok(true) => {
                let len = frame.len();
                self.shared
                    .counters
                    .queued_outbound
                    .fetch_sub(len, Ordering::Relaxed);
                self.shared
                    .counters
                    .frames_sent
                    .fetch_add(1, Ordering::Relaxed);
                self.shared
                    .counters
                    .bytes_sent
                    .fetch_add(len as u64, Ordering::Relaxed);
                Ok(true)
            },
            // SCTP declined: it stays ours, still counted as queued, retried on
            // the next turn. This is the case the 5 ms clamp exists for.
            Ok(false) => {
                self.pending_out = Some(frame);
                Ok(false)
            },
            Err(err) => Err(NativeError::Write(err.to_string())),
        }
    }

    fn close_drained(&self) -> bool {
        if self.close_deadline.is_some_and(|at| Instant::now() >= at) {
            return true;
        }
        self.pending_out.is_none()
            && self.shared.counters.queued_outbound.load(Ordering::Relaxed) == 0
            && self.shared.counters.sctp_buffered.load(Ordering::Relaxed) == 0
    }

    fn wait_deadline(&self, deadline: Instant, now: Instant) -> Instant {
        let mut wait = deadline;
        // Anything held on either side, or a paused sender, needs a turn the
        // engine has no reason to schedule. `paused` is the subtle one: the
        // resume decision lives in `refresh_queue`, which runs once per turn,
        // so sleeping on str0m's own deadline means the queue can drain below
        // the low-water mark and sit there until the engine next has something
        // to say. That is not a hot loop, it is the opposite — a sustained
        // transfer stalling for up to `MAX_WAIT` per pause cycle.
        if self.paused || self.pending_out.is_some() || !self.delivery.pending.is_empty() {
            wait = wait.min(now + FLUSH_RETRY);
        }
        if self.closing && !self.close_started {
            wait = wait.min(now + FLUSH_RETRY);
        }
        if let Some(at) = self.disconnect_deadline {
            wait = wait.min(at);
        }
        wait = wait.min(now + MAX_WAIT);
        wait.max(now + MIN_WAIT)
    }

    async fn flush_delivery_on_exit(&mut self) {
        let deadline = Instant::now() + FINISH_DELIVER_LIMIT;
        while !self.delivery.pending.is_empty() && !self.delivery.closed {
            self.deliver();
            if self.delivery.pending.is_empty() || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let left = self.delivery.pending.len();
        if left > 0 {
            self.shared
                .counters
                .frames_dropped
                .fetch_add(left as u64, Ordering::Relaxed);
            self.delivery.pending.clear();
        }
    }
}
