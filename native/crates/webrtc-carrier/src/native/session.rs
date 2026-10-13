// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The live session: the handles an application holds, and the state they
//! share with the driver task.
//!
//! The split is deliberate. The driver owns the `Rtc` state machine and the
//! socket and never leaves its task; the application holds
//! [`FrameReader`], [`FrameWriter`] and [`CarrierControl`], which are three
//! ordinary async handles over channels. Nothing in this module touches str0m,
//! which is what keeps the Sans-IO loop in one file with one shape.
//!
//! ## Where backpressure is decided
//!
//! [`crate::Backpressure`] is evaluated in two places against *one* number:
//!
//! - the writer, before it enqueues, so a caller learns at its own call site
//!   that the queue is full — [`FrameWriter::try_send_frame`] refuses,
//!   [`FrameWriter::send_frame`] defers;
//! - the driver, before it takes another frame off the hand-off queue, so a
//!   writer that ignored the mark cannot push past it anyway.
//!
//! The number is `queued_outbound + sctp_buffered`: bytes this crate is
//! holding, plus bytes str0m's SCTP association has accepted and not yet
//! flushed. That sum is the native analogue of the browser's
//! `RTCDataChannel.bufferedAmount`, which is what
//! [`crate::backpressure`](crate::Backpressure) says both ends must agree on.
//!
//! Counting *both* matters: str0m stops accepting writes at 128 KiB across all
//! streams, which is below this crate's default high-water mark. If only SCTP's
//! depth were counted the configured mark could never be reached, and the
//! policy would be decoration.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::backpressure::Backpressure;
use crate::error::FrameError;
use crate::frame::encode_frame;
use crate::native::driver::Driver;
use crate::native::error::NativeError;

/// The label this carrier opens by default.
pub const DEFAULT_CHANNEL_LABEL: &str = "mere-graphshell";

/// How long [`serve`] waits for the data channel before giving up.
pub const DEFAULT_OPEN_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the driver tolerates a disconnected ICE agent before ending.
///
/// The alternative — waiting forever — is the hot-loop-adjacent failure C1
/// names: a peer that vanished leaves a task retransmitting at whatever cadence
/// ICE picked, forever, with nothing to report.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Frames the hand-off queue holds between the writer and the driver.
pub const DEFAULT_OUTBOUND_QUEUE_FRAMES: usize = 8;

/// Frames the delivery queue holds between the driver and the reader.
pub const DEFAULT_INBOUND_QUEUE_FRAMES: usize = 64;

/// Bytes SCTP may hold outstanding before the driver stops handing it frames.
///
/// This is a **stack** budget wearing a throughput knob's clothes, and the
/// number was measured rather than chosen. str0m 0.23.1's `do_poll_output`
/// calls itself once per SCTP packet it drains — `str0m-0.23.1/src/lib.rs`
/// lines 1719 and 1734, both `return self.do_poll_output();` inside the
/// `SctpEvent::Transmit` arm — so one `poll_output` call recurses as deep as
/// the burst SCTP hands it, and the burst follows the outstanding window.
/// Nothing unwinds until the whole burst is drained.
///
/// With a peer that stops acknowledging mid-transfer the window sits at
/// str0m's own ceiling of 128 KiB (`MAX_BUFFERED_ACROSS_STREAMS`), and the
/// next drain overflows the stack rather than reporting a dead session. The
/// minimum thread stack that survives that moment, measured on a debug build
/// against 8 KiB frames:
///
/// ```text
/// window   128 KiB (str0m's own)   overflows 2 MiB — the live crash
/// window    64 KiB                 needs ~1.3-2 MiB, non-monotonic: marginal
/// window    32 KiB                 needs 1 MiB
/// window    16 KiB                 needs 640 KiB
/// window     8 KiB                 needs 512 KiB
/// ```
///
/// The 128 KiB row was later resolved rather than left as "overflows", by
/// putting the driver on a thread whose stack this crate chooses — see
/// [`DriverPlacement::DedicatedThread`]. Same workload, same debug build, one
/// process per size because an overflow aborts:
///
/// ```text
/// window   128 KiB   512 KiB stack  overflows
/// window   128 KiB     1 MiB stack  overflows
/// window   128 KiB     2 MiB stack  overflows   <- a tokio worker's size
/// window   128 KiB     4 MiB stack  survives
/// window   128 KiB     8 MiB stack  survives    <- DEDICATED_DRIVER_STACK_BYTES
/// ```
///
/// The 2 MiB row is the control that makes the rest mean something: a 16 KiB
/// window on that same 2 MiB thread survives the identical workload, so the
/// overflow is the window's, not the thread's.
///
/// 16 KiB is the default because tokio's worker threads get 2 MiB, which is
/// where the live crash happened: it leaves a factor of three in a *debug*
/// build, where the frames of that function are at their fattest. The cost is
/// a 16 KiB outstanding window — `buffered_amount` is released on
/// acknowledgement, not on send — so a link with a long round trip will want
/// this raised, which is why it is a field and not a constant.
///
/// Raise it deliberately, with the driver's thread stack in hand — which is
/// what [`DriverPlacement::DedicatedThread`] exists to put there.
pub const DEFAULT_SCTP_WINDOW_BYTES: usize = 16 * 1024;

/// The window a driver on its own thread can afford: str0m's own ceiling.
///
/// 128 KiB is `MAX_BUFFERED_ACROSS_STREAMS`, the point past which str0m stops
/// accepting writes at all, so nothing is gained by asking for more. It is also
/// the window that overflows a 2 MiB stack, which is why it is reachable only
/// with [`DEDICATED_DRIVER_STACK_BYTES`] underneath it — see
/// [`DEFAULT_SCTP_WINDOW_BYTES`] for the stack table, and
/// [`CarrierConfig::dedicated`] for the pairing.
///
/// Chosen at 64 KiB, deliberately **below** str0m's own
/// `MAX_BUFFERED_ACROSS_STREAMS` (128 KiB): this crate's ceiling stays the
/// primary backpressure and [`CarrierStats::window_holds`] keeps firing, so
/// the 8 MiB stack is margin rather than the sole line between str0m's
/// per-packet recursion and the guard page. It is also the loopback
/// throughput optimum with the ceiling engaged (32 KiB and 64 KiB both sit at
/// the knee; the shared-runtime 16 KiB default and the full 128 KiB are both
/// slower here).
///
/// Setting the window to str0m's 128 KiB instead would **switch this crate's
/// ceiling off** — the two limits coincide there, str0m refuses the crossing
/// write first, `window_holds` stays zero, and the stack alone stands between
/// the recursion and the guard page. [`CarrierConfig::dedicated`] takes the
/// window explicitly for a caller who wants that (e.g. a high-round-trip link
/// where a bigger outstanding window earns its keep), with the driver's stack
/// in hand.
pub const DEDICATED_SCTP_WINDOW_BYTES: usize = 64 * 1024;

/// The stack [`DriverPlacement::DedicatedThread`] gives the driver by default.
///
/// Sized for [`DEDICATED_SCTP_WINDOW_BYTES`] with margin, measured rather than
/// extrapolated: at a 128 KiB window the vanished-peer moment overflows 512
/// KiB, 1 MiB and 2 MiB, and survives 4 MiB — so 8 MiB is 2x the smallest size
/// that survives, on a debug build, where the frames of str0m's `do_poll_output`
/// are at their fattest. The full sweep is in [`DEFAULT_SCTP_WINDOW_BYTES`].
///
/// A release build wants less, and nothing here depends on knowing which build
/// it is: a thread stack is virtual address space, committed page by page as
/// the recursion actually descends, so the unused margin costs address space
/// rather than memory.
pub const DEDICATED_DRIVER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// The floor [`DriverPlacement::DedicatedThread`] clamps a stack request to.
///
/// 256 KiB does not survive this crate's *ordinary* work in a debug build — a
/// run at that size dies generating the DTLS certificate, before a single
/// `poll_output` — and 512 KiB clears the handshake. A thread too small to
/// finish DTLS is never what a caller meant, so the request is raised rather
/// than honoured into an immediate crash.
pub const MIN_DEDICATED_DRIVER_STACK_BYTES: usize = 512 * 1024;

/// The name the dedicated driver thread carries, so a backtrace names it.
const DEDICATED_DRIVER_THREAD_NAME: &str = "mere-webrtc-driver";

/// Where the Sans-IO driver loop runs.
///
/// The driver is one `select!` loop over a socket, a timer and three channels,
/// and it spawns nothing, so it will run anywhere. What differs between the two
/// placements is **whose stack it gets**, and that is not a detail: str0m
/// 0.23.1's `do_poll_output` recurses once per queued SCTP packet, so the stack
/// under the driver is what bounds
/// [`CarrierConfig::sctp_window_bytes`](CarrierConfig::sctp_window_bytes), and
/// the window is what bounds throughput on a link with any round trip at all.
///
/// A window smaller than one frame is the pathological case: every frame waits
/// for SCTP to drain to empty before the next is handed over, which turns a
/// pipelined transfer into a stop-and-wait one. Measured on a LAN echo of
/// 200 x 16 KiB frames, a 16 KiB window took 116.6 s against 1.5 s — a 65x
/// penalty paid entirely to keep the driver inside a 2 MiB tokio worker.
///
/// ## What the placement is, and is not, worth
///
/// That 65x is a *latency* result: the cost is one round trip per frame, so it
/// scales with the round trip. On loopback there is almost none, and the same
/// 200 x 16 KiB echo measures the other way — three passes, debug build,
/// `tests/native_loopback.rs`:
///
/// ```text
/// placement    window    elapsed (median)   window_holds
/// shared        16 KiB   678 ms             524
/// shared       128 KiB   1.76 s             0
/// dedicated     16 KiB   743 ms             532
/// dedicated    128 KiB   1.85 s             0
/// ```
///
/// Two readings. The **placement itself is throughput-neutral**: same window,
/// either side of the thread boundary, same time within noise. And the **raised
/// window is a bet on latency**, not a free win — with no round trip to save,
/// the larger burst is simply more work per `poll_output`, about 2.6x more
/// here. What moves deterministically is `window_holds`, the count of turns the
/// driver kept a frame back: hundreds at 16 KiB, zero at 128 KiB. That is the
/// stop-and-wait appearing and disappearing, and it is the thing the LAN's
/// round trip multiplies into 116 seconds.
///
/// So: reach for [`DedicatedThread`](Self::DedicatedThread) when the link has a
/// round trip worth pipelining over, and leave it alone for a local one.
///
/// Both placements are joined, cancelled and reported on identically:
/// [`CarrierControl::cancel`], [`CarrierControl::close`],
/// [`CarrierControl::join`], [`CarrierControl::is_finished`] and the terminal
/// reason a reader or writer restates behave the same under either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DriverPlacement {
    /// On the caller's tokio runtime, via `tokio::spawn`. The default.
    ///
    /// Costs nothing and adds no thread, and is right whenever the window this
    /// crate defaults to is enough. The constraint it carries is the runtime's:
    /// tokio's worker threads get 2 MiB, which caps the SCTP window at roughly
    /// [`DEFAULT_SCTP_WINDOW_BYTES`].
    #[default]
    SharedRuntime,
    /// On a thread of this crate's own, hosting a current-thread runtime.
    ///
    /// The point is `stack_bytes`: a driver that owns its thread owns its
    /// stack, so the window can rise to [`DEDICATED_SCTP_WINDOW_BYTES`] without
    /// putting a recursion depth nobody chose onto a worker shared with the
    /// rest of the application.
    ///
    /// The runtime is `new_current_thread().enable_all()` — the driver needs
    /// tokio's io and time drivers and nothing else, and a work-stealing
    /// scheduler for a single task would only add threads with 2 MiB stacks
    /// back. The socket is handed over as a `std::net::UdpSocket` and
    /// re-registered inside that runtime, because a tokio socket's readiness is
    /// driven by the io driver it was registered with and moving one between
    /// runtimes silently leaves it depending on the old one.
    ///
    /// `stack_bytes` is clamped up to [`MIN_DEDICATED_DRIVER_STACK_BYTES`].
    DedicatedThread {
        /// The thread's stack, in bytes. See [`DEDICATED_DRIVER_STACK_BYTES`].
        stack_bytes: usize,
    },
}

/// The two role-tagged fingerprints of the DTLS handshake that actually ran.
///
/// Not the SDP's claims. str0m publishes the peer's fingerprint only once the
/// certificate has been presented and checked against what signalling promised,
/// so these are the certificates in use rather than the ones announced — which
/// is what [`crate::LinkChallenge`] must bind if the transcript is to mean
/// "this connection and no other".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionFingerprints {
    client: crate::fingerprint::DtlsFingerprint,
    server: crate::fingerprint::DtlsFingerprint,
}

impl SessionFingerprints {
    pub(crate) const fn new(
        client: crate::fingerprint::DtlsFingerprint,
        server: crate::fingerprint::DtlsFingerprint,
    ) -> Self {
        Self { client, server }
    }

    /// The initiating end's fingerprint: the browser's.
    pub const fn client(&self) -> &crate::fingerprint::DtlsFingerprint {
        &self.client
    }

    /// The responding end's fingerprint: the native host's.
    pub const fn server(&self) -> &crate::fingerprint::DtlsFingerprint {
        &self.server
    }
}

/// How a live carrier behaves once the channel is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierConfig {
    /// The data-channel label to accept. Any other label is refused.
    pub channel_label: String,
    /// Which half of the DTLS handshake this end plays.
    ///
    /// It decides which role tag each fingerprint carries in
    /// [`SessionFingerprints`], and getting it wrong produces a transcript
    /// whose link the peer cannot reproduce.
    ///
    /// [`Answerer`](super::Answerer) leaves this at
    /// [`Server`](crate::FingerprintRole::Server) and is right to: RFC 8842
    /// requires an offerer to send `a=setup:actpass`, and str0m's answerer
    /// always replies `passive`, so the answering end is always the DTLS
    /// server. A peer built directly through [`serve`] declares its own.
    pub local_dtls_role: crate::fingerprint::FingerprintRole,
    /// The queue policy both ends obey.
    pub backpressure: Backpressure,
    /// How long to wait for the channel to open.
    pub open_timeout: Duration,
    /// How long to tolerate a disconnected ICE agent before ending.
    pub idle_timeout: Duration,
    /// Depth of the writer-to-driver hand-off queue, in frames.
    pub outbound_queue_frames: usize,
    /// Depth of the driver-to-reader delivery queue, in frames.
    pub inbound_queue_frames: usize,
    /// Bytes SCTP may hold outstanding before the driver stops handing it
    /// frames. See [`DEFAULT_SCTP_WINDOW_BYTES`] — this bounds the depth of
    /// str0m's own recursion, and therefore the driver task's stack.
    ///
    /// Clamped to at least one byte, which means "hand SCTP a frame only when
    /// it is completely drained". A single frame is written whole either way:
    /// the ceiling is checked *before* the write, never in the middle of one,
    /// so the real bound is this plus one frame.
    pub sctp_window_bytes: usize,
    /// Where the driver loop runs, and therefore what stack it gets.
    ///
    /// [`SharedRuntime`](DriverPlacement::SharedRuntime) by default, which is
    /// today's `tokio::spawn` and the reason
    /// [`sctp_window_bytes`](Self::sctp_window_bytes) defaults low. Raising the
    /// window without moving the driver is the stack overflow
    /// [`DEFAULT_SCTP_WINDOW_BYTES`] describes, so the two are set together —
    /// see [`CarrierConfig::dedicated`].
    pub placement: DriverPlacement,
}

impl Default for CarrierConfig {
    fn default() -> Self {
        Self {
            channel_label: DEFAULT_CHANNEL_LABEL.to_owned(),
            local_dtls_role: crate::fingerprint::FingerprintRole::Server,
            backpressure: Backpressure::default(),
            open_timeout: DEFAULT_OPEN_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            outbound_queue_frames: DEFAULT_OUTBOUND_QUEUE_FRAMES,
            inbound_queue_frames: DEFAULT_INBOUND_QUEUE_FRAMES,
            sctp_window_bytes: DEFAULT_SCTP_WINDOW_BYTES,
            placement: DriverPlacement::default(),
        }
    }
}

impl CarrierConfig {
    /// The throughput pairing: driver on its own thread, window raised to match.
    ///
    /// The two numbers are one decision, which is why this exists as a
    /// constructor rather than as advice. A raised
    /// [`sctp_window_bytes`](Self::sctp_window_bytes) is a raised recursion
    /// depth inside str0m, and a stack that does not cover it is a process
    /// death rather than a slow session; the default placement's 16 KiB window
    /// is the value that fits a 2 MiB tokio worker and it stays exactly where
    /// it is.
    ///
    /// The pairing this crate measured and stands behind:
    ///
    /// ```
    /// # use webrtc_carrier::native::{CarrierConfig, DEDICATED_DRIVER_STACK_BYTES,
    /// #     DEDICATED_SCTP_WINDOW_BYTES, DriverPlacement};
    /// let config = CarrierConfig::dedicated(
    ///     DEDICATED_DRIVER_STACK_BYTES,
    ///     DEDICATED_SCTP_WINDOW_BYTES,
    /// );
    /// assert_eq!(
    ///     config.placement,
    ///     DriverPlacement::DedicatedThread { stack_bytes: DEDICATED_DRIVER_STACK_BYTES },
    /// );
    /// ```
    ///
    /// Everything else is [`Default`], so the usual struct-update syntax still
    /// applies for a label, a timeout or a backpressure policy.
    #[must_use]
    pub fn dedicated(stack_bytes: usize, sctp_window_bytes: usize) -> Self {
        Self {
            sctp_window_bytes,
            placement: DriverPlacement::DedicatedThread { stack_bytes },
            ..Self::default()
        }
    }
}

/// Why the driver stopped, in the form a handle can restate.
#[derive(Debug, Clone)]
pub(crate) enum Terminal {
    /// The session ended the way sessions end.
    Closed,
    /// The control handle cancelled, or was dropped.
    Cancelled,
    /// A write to the data channel failed.
    Write(String),
    /// A framing rule was broken on the wire.
    Frame(FrameError),
    /// Anything else, already rendered.
    Failed(String),
}

impl Terminal {
    pub(crate) fn to_error(&self) -> NativeError {
        match self {
            Self::Closed => NativeError::Closed,
            Self::Cancelled => NativeError::Cancelled,
            Self::Write(text) => NativeError::Write(text.clone()),
            Self::Frame(err) => NativeError::Frame(err.clone()),
            Self::Failed(text) => NativeError::Driver(text.clone()),
        }
    }
}

/// What the control handle asks the driver to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    /// Keep going.
    Run,
    /// Flush what is queued, then shut the session down politely.
    Close,
    /// Stop now.
    Cancel,
}

/// The atomics the driver publishes and the handles read.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub(crate) queued_outbound: AtomicUsize,
    pub(crate) sctp_buffered: AtomicUsize,
    pub(crate) peak_queued: AtomicUsize,
    pub(crate) pauses: AtomicU64,
    pub(crate) resumes: AtomicU64,
    pub(crate) low_water_events: AtomicU64,
    pub(crate) write_waits: AtomicU64,
    pub(crate) frames_enqueued: AtomicU64,
    pub(crate) frames_sent: AtomicU64,
    pub(crate) frames_received: AtomicU64,
    pub(crate) bytes_sent: AtomicU64,
    pub(crate) bytes_received: AtomicU64,
    pub(crate) datagrams_sent: AtomicU64,
    pub(crate) datagrams_received: AtomicU64,
    pub(crate) malformed_datagrams: AtomicU64,
    pub(crate) socket_resets: AtomicU64,
    pub(crate) iterations: AtomicU64,
    pub(crate) timeouts: AtomicU64,
    pub(crate) frames_dropped: AtomicU64,
    pub(crate) window_holds: AtomicU64,
}

impl Counters {
    pub(crate) fn queued_bytes(&self) -> usize {
        self.queued_outbound
            .load(Ordering::Relaxed)
            .saturating_add(self.sctp_buffered.load(Ordering::Relaxed))
    }

    pub(crate) fn record_peak(&self, queued: usize) {
        self.peak_queued.fetch_max(queued, Ordering::Relaxed);
    }
}

/// Everything the driver and the handles both need.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) counters: Counters,
    pub(crate) backpressure: Backpressure,
    pub(crate) channel_label: String,
    pub(crate) local_addr: SocketAddr,
    pub(crate) resume_rx: watch::Receiver<u64>,
    pub(crate) terminal: std::sync::Mutex<Option<Terminal>>,
    pub(crate) fingerprints: std::sync::Mutex<Option<SessionFingerprints>>,
}

impl Shared {
    pub(crate) fn set_fingerprints(&self, fingerprints: SessionFingerprints) {
        *self.fingerprints.lock().expect("fingerprint mutex") = Some(fingerprints);
    }

    pub(crate) fn fingerprints(&self) -> Option<SessionFingerprints> {
        *self.fingerprints.lock().expect("fingerprint mutex")
    }

    pub(crate) fn set_terminal(&self, terminal: Terminal) {
        let mut slot = self.terminal.lock().expect("terminal mutex");
        if slot.is_none() {
            *slot = Some(terminal);
        }
    }

    /// The error a handle reports once the driver is gone.
    ///
    /// `Closed` is the honest default: if the driver finished without leaving a
    /// reason, it finished.
    pub(crate) fn terminal_error(&self) -> NativeError {
        self.terminal
            .lock()
            .expect("terminal mutex")
            .as_ref()
            .map_or(NativeError::Closed, Terminal::to_error)
    }

    fn terminal_is_close(&self) -> bool {
        matches!(
            self.terminal.lock().expect("terminal mutex").as_ref(),
            None | Some(Terminal::Closed)
        )
    }
}

/// A snapshot of what the carrier has done.
///
/// Every field is a counter the driver maintains at runtime rather than a
/// figure derived after the fact, so a test can assert that the high and low
/// water marks were actually crossed rather than that they could have been.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CarrierStats {
    /// Bytes this crate holds for the peer: hand-off queue plus a held frame.
    pub queued_outbound: usize,
    /// Bytes str0m's SCTP association has accepted and not yet flushed.
    pub sctp_buffered: usize,
    /// The largest `queued_outbound + sctp_buffered` ever observed.
    pub peak_queued: usize,
    /// Times the driver stopped taking frames because of the high-water mark.
    pub pauses: u64,
    /// Times it started again because of the low-water mark.
    pub resumes: u64,
    /// `ChannelBufferedAmountLow` events str0m raised at the low-water mark.
    pub low_water_events: u64,
    /// Times a writer had to wait for the queue to drain.
    pub write_waits: u64,
    /// Frames handed to the hand-off queue.
    pub frames_enqueued: u64,
    /// Frames accepted by SCTP.
    pub frames_sent: u64,
    /// Frames decoded off the wire.
    pub frames_received: u64,
    /// Frame bytes accepted by SCTP, prefix included.
    pub bytes_sent: u64,
    /// Frame bytes decoded off the wire, prefix included.
    pub bytes_received: u64,
    /// Datagrams written to the socket.
    pub datagrams_sent: u64,
    /// Datagrams read off the socket and fed to str0m.
    pub datagrams_received: u64,
    /// Datagrams that were not STUN, DTLS, RTP or RTCP, and were ignored.
    pub malformed_datagrams: u64,
    /// Socket errors the OS raised for a peer that stopped listening.
    pub socket_resets: u64,
    /// Turns of the Sans-IO loop. The instrument for "no hot loop".
    pub iterations: u64,
    /// Timeout inputs fed to str0m.
    pub timeouts: u64,
    /// Decoded frames discarded because the reader was gone at shutdown.
    pub frames_dropped: u64,
    /// Times a held frame was kept back because SCTP's outstanding window was
    /// already at [`CarrierConfig::sctp_window_bytes`].
    pub window_holds: u64,
}

impl CarrierStats {
    fn read(counters: &Counters) -> Self {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        let size = |value: &AtomicUsize| value.load(Ordering::Relaxed);
        Self {
            queued_outbound: size(&counters.queued_outbound),
            sctp_buffered: size(&counters.sctp_buffered),
            peak_queued: size(&counters.peak_queued),
            pauses: load(&counters.pauses),
            resumes: load(&counters.resumes),
            low_water_events: load(&counters.low_water_events),
            write_waits: load(&counters.write_waits),
            frames_enqueued: load(&counters.frames_enqueued),
            frames_sent: load(&counters.frames_sent),
            frames_received: load(&counters.frames_received),
            bytes_sent: load(&counters.bytes_sent),
            bytes_received: load(&counters.bytes_received),
            datagrams_sent: load(&counters.datagrams_sent),
            datagrams_received: load(&counters.datagrams_received),
            malformed_datagrams: load(&counters.malformed_datagrams),
            socket_resets: load(&counters.socket_resets),
            iterations: load(&counters.iterations),
            timeouts: load(&counters.timeouts),
            frames_dropped: load(&counters.frames_dropped),
            window_holds: load(&counters.window_holds),
        }
    }
}

/// The read half: whole frames, in order, as the peer sent them.
#[derive(Debug)]
pub struct FrameReader {
    rx: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Shared>,
}

impl FrameReader {
    /// Waits for the next frame's payload.
    ///
    /// `Ok(None)` is end of stream: the session closed, which is not a failure.
    /// A cancelled task, a failed write, or an oversize frame from the peer
    /// each arrive as their own [`NativeError`] instead.
    pub async fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, NativeError> {
        match self.rx.recv().await {
            Some(payload) => Ok(Some(payload)),
            None if self.shared.terminal_is_close() => Ok(None),
            None => Err(self.shared.terminal_error()),
        }
    }

    /// A snapshot of the session's counters.
    pub fn stats(&self) -> CarrierStats {
        CarrierStats::read(&self.shared.counters)
    }
}

/// The write half: whole frames, framed by [`crate::encode_frame`].
#[derive(Debug, Clone)]
pub struct FrameWriter {
    tx: mpsc::Sender<Vec<u8>>,
    shared: Arc<Shared>,
}

impl FrameWriter {
    /// Frames `payload` and enqueues it, waiting while the queue is full.
    ///
    /// This is the deferring half of the backpressure policy. It waits at the
    /// high-water mark and returns once the driver has reported a queue at or
    /// below the low-water mark; the gap between the two marks is what stops
    /// the wait from re-arming on every completed write.
    ///
    /// The wait is a plain `.await` on a watch channel, so a caller that wants
    /// a deadline wraps this in `tokio::time::timeout` and a caller that wants
    /// cancellation drops the future. Neither leaves the driver spinning.
    pub async fn send_frame(&self, payload: &[u8]) -> Result<(), NativeError> {
        let framed = encode_frame(payload)?;
        let len = framed.len();

        self.await_room().await?;
        self.enqueue(framed, len).await
    }

    /// Frames `payload` and enqueues it, or refuses.
    ///
    /// This is the refusing half of the same policy: at or above the high-water
    /// mark it returns [`NativeError::WouldBlock`] carrying the numbers, and
    /// enqueues nothing.
    pub fn try_send_frame(&self, payload: &[u8]) -> Result<(), NativeError> {
        let framed = encode_frame(payload)?;
        let len = framed.len();
        let queued = self.shared.counters.queued_bytes();
        if self.shared.backpressure.should_pause(queued) {
            return Err(NativeError::WouldBlock {
                queued,
                high_water: self.shared.backpressure.high_water_bytes(),
            });
        }

        self.shared
            .counters
            .queued_outbound
            .fetch_add(len, Ordering::Relaxed);
        match self.tx.try_send(framed) {
            Ok(()) => {
                self.shared
                    .counters
                    .frames_enqueued
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.unqueue(len);
                Err(NativeError::WouldBlock {
                    queued: self.shared.counters.queued_bytes(),
                    high_water: self.shared.backpressure.high_water_bytes(),
                })
            },
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.unqueue(len);
                Err(self.shared.terminal_error())
            },
        }
    }

    /// Enqueues `message` as one data-channel message, unframed.
    ///
    /// Not the method an application wants: the framing is the carrier's, and
    /// bypassing it puts bytes on the wire the peer's decoder is entitled to
    /// reject. It exists because proving the receive path rejects an oversize
    /// length prefix *on the wire* requires a peer willing to send one, and a
    /// carrier that can only emit well-formed frames cannot be that peer.
    ///
    /// Backpressure applies exactly as it does to a framed write.
    pub async fn send_message(&self, message: Vec<u8>) -> Result<(), NativeError> {
        let len = message.len();
        self.await_room().await?;
        self.enqueue(message, len).await
    }

    /// Waits until the queue is back at or below the low-water mark.
    ///
    /// The ordering here is the whole correctness argument, and it is easy to
    /// get wrong in a way no type catches. `watch::Receiver::clone` inherits
    /// the *source* receiver's seen-version, and the receiver parked in
    /// `Shared` is never polled, so its version never advances -- a bare clone
    /// is therefore permanently "behind" and `changed()` returns instantly,
    /// which is a hot spin rather than a wait.
    ///
    /// So: clone, mark the clone seen with `borrow_and_update`, *then* read the
    /// gauge, *then* wait. A tick published between the mark and the read still
    /// wakes us, because it moves the version past what we marked. That is the
    /// missed-wakeup hazard and the spin hazard closed in one order.
    async fn await_room(&self) -> Result<(), NativeError> {
        loop {
            let mut resume = self.shared.resume_rx.clone();
            let _seen = *resume.borrow_and_update();
            if !self
                .shared
                .backpressure
                .should_pause(self.shared.counters.queued_bytes())
            {
                return Ok(());
            }
            self.shared
                .counters
                .write_waits
                .fetch_add(1, Ordering::Relaxed);
            if resume.changed().await.is_err() {
                return Err(self.shared.terminal_error());
            }
        }
    }

    async fn enqueue(&self, framed: Vec<u8>, len: usize) -> Result<(), NativeError> {
        self.shared
            .counters
            .queued_outbound
            .fetch_add(len, Ordering::Relaxed);
        match self.tx.send(framed).await {
            Ok(()) => {
                self.shared
                    .counters
                    .frames_enqueued
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
            Err(_) => {
                self.unqueue(len);
                Err(self.shared.terminal_error())
            },
        }
    }

    fn unqueue(&self, len: usize) {
        self.shared
            .counters
            .queued_outbound
            .fetch_sub(len, Ordering::Relaxed);
    }

    /// The queue policy this writer obeys.
    pub fn backpressure(&self) -> Backpressure {
        self.shared.backpressure
    }

    /// A snapshot of the session's counters.
    pub fn stats(&self) -> CarrierStats {
        CarrierStats::read(&self.shared.counters)
    }
}

/// The one thing a handle needs from the driver, whichever thread it is on.
///
/// Two placements, one contract: "has it finished" and "wait for its result".
/// The tokio arm is a `JoinHandle`; the thread arm is an `AtomicBool` the
/// thread raises and a `oneshot` it reports through, which is a `JoinHandle`
/// rebuilt out of the two pieces an OS thread cannot supply on its own. The
/// distinction is deliberately not visible above this enum: see
/// [`DriverPlacement`] for why the placements must be indistinguishable to
/// [`CarrierControl`].
#[derive(Debug)]
pub(crate) enum DriverHandle {
    /// Spawned onto the caller's runtime.
    Task(JoinHandle<Result<(), NativeError>>),
    /// Running on a thread of this crate's own.
    Thread {
        /// Raised by the thread after the driver returns *and* its runtime has
        /// been dropped, so `is_finished` never reports done while the socket
        /// is still registered.
        finished: Arc<AtomicBool>,
        /// The driver's own result, sent after `finished` is raised.
        result: oneshot::Receiver<Result<(), NativeError>>,
        /// Kept only to recover a panic payload: a thread that dies without
        /// sending drops the sender, and joining it is the only way to say why.
        thread: Option<std::thread::JoinHandle<()>>,
    },
}

impl DriverHandle {
    fn is_finished(&self) -> bool {
        match self {
            Self::Task(task) => task.is_finished(),
            Self::Thread { finished, .. } => finished.load(Ordering::Acquire),
        }
    }

    async fn join(self) -> Result<(), NativeError> {
        match self {
            Self::Task(task) => match task.await {
                Ok(result) => result,
                Err(err) => Err(NativeError::Join(err.to_string())),
            },
            Self::Thread { result, thread, .. } => match result.await {
                Ok(outcome) => outcome,
                // The sender was dropped without sending, which on this path
                // means the thread unwound. `tokio::spawn` renders that as a
                // `JoinError`; render it the same way rather than as a bare
                // "channel closed" that says nothing.
                Err(_) => Err(NativeError::Join(dedicated_failure(thread))),
            },
        }
    }
}

/// Why a dedicated driver thread ended without reporting.
fn dedicated_failure(thread: Option<std::thread::JoinHandle<()>>) -> String {
    let Some(thread) = thread else {
        return "the carrier driver thread ended without reporting".to_owned();
    };
    // The thread is already unwinding or gone by the time the sender dropped,
    // so this join does not wait on live work.
    match thread.join() {
        Ok(()) => "the carrier driver thread ended without reporting".to_owned(),
        Err(payload) => {
            let text = payload
                .downcast_ref::<&'static str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a payload that is not a string".to_owned());
            format!("the carrier driver thread panicked: {text}")
        },
    }
}

/// Starts the driver on a thread of this crate's own.
///
/// `build` runs *inside* the new runtime, which is not a nicety: the driver's
/// socket has to be registered with the io driver that will poll it, and
/// `UdpSocket::from_std` can only reach the runtime it is called from.
///
/// The result is published after the runtime is dropped, so a caller whose
/// [`CarrierControl::join`] has returned holds the guarantee it holds today —
/// the socket is closed and nothing is still running.
fn spawn_dedicated_driver<F>(stack_bytes: usize, build: F) -> Result<DriverHandle, NativeError>
where
    F: FnOnce() -> Result<Driver, NativeError> + Send + 'static,
{
    let finished = Arc::new(AtomicBool::new(false));
    let (result_tx, result_rx) = oneshot::channel();
    let raise = Arc::clone(&finished);

    let thread = std::thread::Builder::new()
        .name(DEDICATED_DRIVER_THREAD_NAME.to_owned())
        .stack_size(stack_bytes.max(MIN_DEDICATED_DRIVER_STACK_BYTES))
        .spawn(move || {
            let outcome = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => {
                    let outcome = runtime.block_on(async move {
                        match build() {
                            Ok(driver) => driver.run().await,
                            Err(err) => Err(err),
                        }
                    });
                    // Before the result is published, not after: a joined
                    // carrier must not leave a socket registered behind it.
                    drop(runtime);
                    outcome
                },
                Err(err) => Err(NativeError::DriverThread(err)),
            };
            raise.store(true, Ordering::Release);
            let _ = result_tx.send(outcome);
        })
        .map_err(NativeError::DriverThread)?;

    Ok(DriverHandle::Thread {
        finished,
        result: result_rx,
        thread: Some(thread),
    })
}

/// The session's lifetime.
///
/// Holding this is what keeps the driver running: dropping it cancels, which is
/// the same rule a `JoinHandle`-less task ought to have and the reason a
/// forgotten session cannot outlive the code that opened it.
#[derive(Debug)]
pub struct CarrierControl {
    command: watch::Sender<Command>,
    driver: Option<DriverHandle>,
    shared: Arc<Shared>,
}

impl CarrierControl {
    /// Asks the driver to stop now.
    ///
    /// The driver's result becomes [`NativeError::Cancelled`], and so does the
    /// error any pending read or write reports.
    pub fn cancel(&self) {
        let _ = self.command.send(Command::Cancel);
    }

    /// Flushes what is queued, shuts the peer connection down, and waits.
    ///
    /// The polite ending. Frames already enqueued are written first, bounded by
    /// an internal deadline so a peer that stopped reading cannot hold this
    /// open.
    pub async fn close(mut self) -> Result<(), NativeError> {
        let _ = self.command.send(Command::Close);
        self.join_task().await
    }

    /// Waits for the driver to finish on its own terms.
    pub async fn join(mut self) -> Result<(), NativeError> {
        self.join_task().await
    }

    async fn join_task(&mut self) -> Result<(), NativeError> {
        match self.driver.take() {
            Some(driver) => driver.join().await,
            None => Err(NativeError::Join("already joined".to_owned())),
        }
    }

    /// Whether the driver has already finished.
    ///
    /// False once it has been joined, under either
    /// [`DriverPlacement`]: the question is about a driver this handle still
    /// holds.
    pub fn is_finished(&self) -> bool {
        self.driver.as_ref().is_some_and(DriverHandle::is_finished)
    }

    /// The local UDP address the carrier is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.shared.local_addr
    }

    /// The role-tagged fingerprints of the DTLS handshake that ran.
    ///
    /// Available from the moment the session is live: str0m reports `Connected`
    /// once DTLS is established, and the channel cannot open before that.
    pub fn fingerprints(&self) -> Option<SessionFingerprints> {
        self.shared.fingerprints()
    }

    /// A snapshot of the session's counters.
    pub fn stats(&self) -> CarrierStats {
        CarrierStats::read(&self.shared.counters)
    }
}

impl Drop for CarrierControl {
    fn drop(&mut self) {
        let _ = self.command.send(Command::Cancel);
    }
}

/// A live carrier: one ordered, reliable data channel, framed.
#[derive(Debug)]
pub struct Carrier {
    reader: FrameReader,
    writer: FrameWriter,
    control: CarrierControl,
}

impl Carrier {
    /// Takes the three handles apart, so reading, writing, and lifetime can go
    /// to different tasks.
    pub fn into_parts(self) -> (FrameReader, FrameWriter, CarrierControl) {
        (self.reader, self.writer, self.control)
    }

    /// Waits for the next frame's payload. See [`FrameReader::recv_frame`].
    pub async fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, NativeError> {
        self.reader.recv_frame().await
    }

    /// Frames and enqueues `payload`, waiting at the high-water mark.
    pub async fn send_frame(&self, payload: &[u8]) -> Result<(), NativeError> {
        self.writer.send_frame(payload).await
    }

    /// Frames and enqueues `payload`, or refuses at the high-water mark.
    pub fn try_send_frame(&self, payload: &[u8]) -> Result<(), NativeError> {
        self.writer.try_send_frame(payload)
    }

    /// The queue policy this carrier obeys.
    pub fn backpressure(&self) -> Backpressure {
        self.writer.backpressure()
    }

    /// The channel label the session negotiated.
    pub fn channel_label(&self) -> &str {
        &self.control.shared.channel_label
    }

    /// The local UDP address the carrier is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.control.local_addr()
    }

    /// The role-tagged fingerprints of the DTLS handshake that ran.
    ///
    /// These are the two fingerprint slots [`crate::LinkChallenge`] binds. The
    /// remaining four facts — protocol, channel label, invite id, and the two
    /// nonces — are not this crate's to supply: the core generates nothing, and
    /// the invitation is C2's.
    pub fn fingerprints(&self) -> Option<SessionFingerprints> {
        self.control.fingerprints()
    }

    /// A snapshot of the session's counters.
    pub fn stats(&self) -> CarrierStats {
        self.control.stats()
    }

    /// Asks the driver to stop now.
    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// Flushes, closes, and waits. See [`CarrierControl::close`].
    pub async fn close(self) -> Result<(), NativeError> {
        let Self {
            reader,
            writer,
            control,
        } = self;
        drop(writer);
        drop(reader);
        control.close().await
    }
}

/// Drives an already-negotiated `Rtc` over `socket` until its data channel
/// opens, and hands back the live carrier.
///
/// This is the seam the answerer is built on, and the seam a test peer uses:
/// the Sans-IO loop does not care which side made the offer, only that the SDP
/// exchange is finished and the socket is bound with its candidate declared.
///
/// The driver is spawned before the wait, so ICE, DTLS, and SCTP all proceed
/// while this call is pending. On timeout the driver is cancelled rather than
/// left running.
///
/// *Where* it is spawned is [`CarrierConfig::placement`]'s to say — this
/// call's own contract, including the cancel-on-timeout, is the same either
/// way. Note that a caller who is already running on a current-thread runtime
/// and passes [`DriverPlacement::SharedRuntime`] gets the driver on that same
/// thread, sharing its stack with everything else there.
pub async fn serve(
    rtc: str0m::Rtc,
    socket: UdpSocket,
    config: CarrierConfig,
) -> Result<Carrier, NativeError> {
    let local_addr = socket.local_addr().map_err(NativeError::Socket)?;
    if local_addr.ip().is_unspecified() {
        // Refused rather than served, because serving it would silently never
        // connect: str0m matches an arriving datagram's *destination* against
        // its local candidates by exact equality, and the unspecified address
        // is never a candidate. See `serve_advertised`, which is what a
        // wildcard bind must use, and which the answerer already uses.
        return Err(NativeError::NoCandidate(format!(
            "the carrier socket is bound to the unspecified address \
             {local_addr}: `serve` cannot tell str0m which local candidate a \
             datagram arrived on. Bind a concrete address, or call \
             `serve_advertised` with the address the ICE candidates declare."
        )));
    }
    serve_advertised(rtc, socket, config, local_addr).await
}

/// Drives an already-negotiated `Rtc` whose candidates declare `advertised`.
///
/// The same loop as [`serve`], with the one fact [`serve`] cannot work out for
/// itself: which local address the peer's datagrams were *addressed to*.
///
/// ## Why str0m needs to be told
///
/// A UDP `recv_from` reports the source and nothing else, so the destination
/// handed to `str0m::net::Receive` is supplied by the adapter. str0m passes it
/// into the ICE agent as `StunPacket::destination`, and `is` — str0m's ICE
/// crate — matches it against the local candidate list by **exact equality**
/// (`is-0.11.0/src/agent.rs`, `stun_server_handle_request`: `v.addr() ==
/// req.destination`, with a `Discarding STUN request on unknown interface`
/// debug line and a `return` when nothing matches).
///
/// So a socket bound to `0.0.0.0` whose candidates declare `127.0.0.1` must
/// hand str0m `127.0.0.1:<port>`. Handing it `0.0.0.0:<port>` — the socket's
/// own address — matches no candidate, every binding request is discarded
/// without a reply, and both ends sit in `checking` until the open timeout.
/// There is no error to catch: the packets are counted as received and then
/// dropped inside the engine.
///
/// ## The assumption this makes
///
/// One socket, one declared address. A carrier that advertises several
/// addresses on one wildcard socket cannot know which of them a datagram
/// arrived on — the OS does not say without `IP_PKTINFO`, which this crate
/// will not reach for — so it attributes every datagram to `advertised`. ICE
/// still works from any of them, because the reply goes out the one socket
/// this end owns; what is lost is only the truth of which local candidate the
/// nominated pair names. [`Answerer`](super::Answerer) passes the first
/// address it successfully declared as a candidate, which is the bound address
/// for a concrete bind and `advertise[0]` on the bound port for a wildcard one.
pub async fn serve_advertised(
    rtc: str0m::Rtc,
    socket: UdpSocket,
    config: CarrierConfig,
    advertised: SocketAddr,
) -> Result<Carrier, NativeError> {
    let local_addr = socket.local_addr().map_err(NativeError::Socket)?;

    let (resume_tx, resume_rx) = watch::channel(0u64);
    let (command_tx, command_rx) = watch::channel(Command::Run);
    let (outbound_tx, outbound_rx) = mpsc::channel(config.outbound_queue_frames.max(1));
    let (inbound_tx, inbound_rx) = mpsc::channel(config.inbound_queue_frames.max(1));
    let (opened_tx, opened_rx) = oneshot::channel();

    let shared = Arc::new(Shared {
        counters: Counters::default(),
        backpressure: config.backpressure,
        channel_label: config.channel_label.clone(),
        local_addr,
        resume_rx,
        terminal: std::sync::Mutex::new(None),
        fingerprints: std::sync::Mutex::new(None),
    });

    let driver = match config.placement {
        DriverPlacement::SharedRuntime => {
            let driver = Driver::new(
                rtc,
                Arc::new(socket),
                advertised,
                Arc::clone(&shared),
                command_rx,
                resume_tx,
                outbound_rx,
                inbound_tx,
                opened_tx,
                &config,
            );
            DriverHandle::Task(tokio::spawn(driver.run()))
        },
        DriverPlacement::DedicatedThread { stack_bytes } => {
            // Handed over as a std socket and re-registered on the other side.
            // A tokio socket carries the io driver it was registered with, so
            // moving one across runtimes leaves its readiness quietly driven by
            // the runtime it left — which is the one thing this placement
            // exists to stop depending on. `into_std` leaves it non-blocking,
            // which is exactly what `from_std` requires.
            let socket = socket.into_std().map_err(NativeError::Socket)?;
            let driver_shared = Arc::clone(&shared);
            let driver_config = config.clone();
            spawn_dedicated_driver(stack_bytes, move || {
                let socket = match UdpSocket::from_std(socket) {
                    Ok(socket) => socket,
                    Err(err) => {
                        // Nothing above will ever see the driver run, so the
                        // reason has to be recorded here or a reader waking on
                        // a closed channel finds only the `Closed` default.
                        let err = NativeError::Socket(err);
                        driver_shared.set_terminal(Terminal::Failed(err.to_string()));
                        return Err(err);
                    },
                };
                Ok(Driver::new(
                    rtc,
                    Arc::new(socket),
                    advertised,
                    driver_shared,
                    command_rx,
                    resume_tx,
                    outbound_rx,
                    inbound_tx,
                    opened_tx,
                    &driver_config,
                ))
            })?
        },
    };

    let mut control = CarrierControl {
        command: command_tx,
        driver: Some(driver),
        shared: Arc::clone(&shared),
    };

    match tokio::time::timeout(config.open_timeout, opened_rx).await {
        Ok(Ok(())) => {},
        Ok(Err(_)) => {
            // The driver dropped the signal, which means it ended first.
            let reason = control.join_task().await;
            return Err(match reason {
                Ok(()) => NativeError::Closed,
                Err(err) => err,
            });
        },
        Err(_) => {
            control.cancel();
            let _ = control.join_task().await;
            return Err(NativeError::OpenTimeout {
                label: config.channel_label,
                timeout: config.open_timeout,
            });
        },
    }

    Ok(Carrier {
        reader: FrameReader {
            rx: inbound_rx,
            shared: Arc::clone(&shared),
        },
        writer: FrameWriter {
            tx: outbound_tx,
            shared: Arc::clone(&shared),
        },
        control,
    })
}

/// The driver's private view of the delivery queue, kept here so the queue's
/// two halves are declared together.
pub(crate) struct Delivery {
    pub(crate) pending: VecDeque<Vec<u8>>,
    pub(crate) hold_limit: usize,
    pub(crate) tx: mpsc::Sender<Vec<u8>>,
    pub(crate) closed: bool,
}
