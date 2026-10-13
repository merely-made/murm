// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The native C1 receipts: two str0m peers, one loopback UDP pair, real DTLS.
//!
//! Nothing here is simulated. Each test binds two ordinary UDP sockets on
//! `127.0.0.1`, exchanges a real SDP offer and answer, runs a real ICE and DTLS
//! handshake, and opens a real SCTP data channel. The answerer under test is
//! the same [`Answerer`] a browser would meet; the peer is a str0m offerer
//! driven by the same public [`serve`] loop, which is the closest a test can
//! get to "a headed browser opened it" without a browser.
//!
//! Seven properties. The first four are C1's done-conditions; the next two are
//! the defects a headed Chrome run found that no loopback test had been shaped
//! to catch; the last is C3's driver placement.
//!
//! 1. bounded ping frames cross both ways and the session closes cleanly;
//! 2. an oversize frame is rejected from its four-byte prefix, on the wire;
//! 3. a sustained transfer crosses the configured high and low water marks;
//! 4. cancellation terminates promptly, and neither an idle session nor a
//!    vanished peer spins;
//! 5. a peer that vanishes *mid-transfer*, with SCTP holding a full window,
//!    ends the session instead of overflowing the driver task's stack;
//! 6. a wildcard bind connects on the address its ICE candidates advertise;
//! 7. the SCTP window can rise to str0m's own ceiling once the driver is on a
//!    stack of its own, and stops holding every frame back when it does.
//!
//! Properties 1 to 4 and 6 each run **twice**, once per
//! [`DriverPlacement`]: that is the whole receipt for the claim that a driver
//! on its own thread is indistinguishable from one on the caller's runtime.
//! Property 5 runs twice with different windows, because the placement is what
//! makes the larger window survivable at all.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;

use webrtc_carrier::native::str0m::RtcConfig;
use webrtc_carrier::native::{
    Answerer, AnswererConfig, Carrier, CarrierConfig, CarrierStats, DEDICATED_DRIVER_STACK_BYTES,
    DEDICATED_SCTP_WINDOW_BYTES, DEFAULT_SCTP_WINDOW_BYTES, DriverPlacement,
    LoopbackOfferer as Offerer, NativeError, loopback_pair, serve,
};
use webrtc_carrier::{
    Backpressure, FingerprintRole, FrameError, MAX_FRAME_BYTES, MAX_FRAME_PAYLOAD_BYTES,
    MAX_QUEUED_BYTES,
};

/// Long enough that a slow CI machine is not the thing being measured, short
/// enough that a genuinely stuck handshake fails rather than hangs.
const HANDSHAKE_BUDGET: Duration = Duration::from_secs(20);

/// The dedicated placement, at the stack this crate ships as its default.
const DEDICATED: DriverPlacement = DriverPlacement::DedicatedThread {
    stack_bytes: DEDICATED_DRIVER_STACK_BYTES,
};

/// The same session policy under a named [`DriverPlacement`].
///
/// The SCTP window deliberately does **not** move with the placement here. The
/// four property tests below each run twice, once per placement, to check the
/// claim `DriverPlacement` actually makes — that join, cancel, close,
/// `is_finished` and the terminal reason are indistinguishable between a driver
/// on the caller's runtime and a driver on its own thread. One variable at a
/// time: what the dedicated placement *buys* (a window past 16 KiB) is proven
/// by the two tests that raise it, `a_vanished_peer_...on_a_dedicated_thread`
/// and `a_raised_window_on_a_dedicated_thread_...`.
fn carrier_config_on(backpressure: Backpressure, placement: DriverPlacement) -> CarrierConfig {
    CarrierConfig {
        backpressure,
        open_timeout: HANDSHAKE_BUDGET,
        idle_timeout: Duration::from_secs(3),
        placement,
        ..CarrierConfig::default()
    }
}

fn carrier_config(backpressure: Backpressure) -> CarrierConfig {
    carrier_config_on(backpressure, DriverPlacement::SharedRuntime)
}

/// Runs one full handshake and returns both live carriers.
///
/// `(answerer, offerer)` — the exported harness, whose own asserts carry the
/// fingerprint receipts this file used to hold privately.
async fn connect(config: CarrierConfig) -> (Carrier, Carrier) {
    loopback_pair(config).await
}

async fn expect_frame(carrier: &mut Carrier, expected: &[u8]) {
    let frame = tokio::time::timeout(Duration::from_secs(10), carrier.recv_frame())
        .await
        .expect("a frame arrives within the budget")
        .expect("the session is still live")
        .expect("the stream has not ended");
    assert_eq!(frame, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_ping_frames_cross_in_both_directions_and_close_cleanly() {
    bounded_ping_frames(DriverPlacement::SharedRuntime).await;
}

/// The same receipt with both drivers on threads of the carrier's own.
///
/// `close` is the interesting half: it must still flush what is queued, end the
/// peer's stream with `Ok(None)` rather than an error, and return — across a
/// thread boundary and a `oneshot` instead of a `JoinHandle`.
#[tokio::test(flavor = "multi_thread")]
async fn bounded_ping_frames_cross_in_both_directions_on_a_dedicated_thread() {
    bounded_ping_frames(DEDICATED).await;
}

async fn bounded_ping_frames(placement: DriverPlacement) {
    let (mut answerer, offerer) =
        connect(carrier_config_on(Backpressure::default(), placement)).await;

    for round in 0u8..8 {
        let to_host = format!("ping {round}");
        offerer
            .send_frame(to_host.as_bytes())
            .await
            .expect("the browser side writes");
        expect_frame(&mut answerer, to_host.as_bytes()).await;

        let to_peer = format!("pong {round}");
        answerer
            .send_frame(to_peer.as_bytes())
            .await
            .expect("the host side writes");
    }

    let (mut peer_reader, _peer_writer, peer_control) = offerer.into_parts();
    for round in 0u8..8 {
        let expected = format!("pong {round}");
        let frame = tokio::time::timeout(Duration::from_secs(10), peer_reader.recv_frame())
            .await
            .expect("a frame arrives within the budget")
            .expect("the session is still live")
            .expect("the stream has not ended");
        assert_eq!(frame, expected.as_bytes());
    }

    // A frame exactly at the ceiling still crosses: the bound is a real
    // maximum, not a value the wire path quietly refuses short of.
    let ceiling = vec![0x5au8; MAX_FRAME_PAYLOAD_BYTES];
    answerer
        .send_frame(&ceiling)
        .await
        .expect("a maximum-size frame is writable");
    let arrived = tokio::time::timeout(Duration::from_secs(15), peer_reader.recv_frame())
        .await
        .expect("the maximum-size frame arrives")
        .expect("the session is still live")
        .expect("the stream has not ended");
    assert_eq!(arrived.len(), MAX_FRAME_PAYLOAD_BYTES);
    assert_eq!(arrived, ceiling);

    let host_stats = answerer.stats();
    assert_eq!(host_stats.frames_received, 8, "eight pings, decoded");
    assert_eq!(
        host_stats.frames_sent, 9,
        "eight pongs and the ceiling frame"
    );
    assert_eq!(host_stats.malformed_datagrams, 0);
    assert!(
        host_stats.datagrams_received > 0 && host_stats.datagrams_sent > 0,
        "the loopback pair actually carried UDP: {host_stats:?}"
    );

    // The host closes. The peer must see end-of-stream, not an error, and its
    // own close must then succeed against an already-finished driver.
    answerer.close().await.expect("a clean close");

    let ended = tokio::time::timeout(Duration::from_secs(10), peer_reader.recv_frame())
        .await
        .expect("the peer notices the close within the budget")
        .expect("a close is not a failure");
    assert!(ended.is_none(), "a closed session ends the stream");

    peer_control.join().await.expect("the peer ends cleanly");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversize_frame_is_rejected_from_its_prefix_on_the_wire() {
    an_oversize_frame_is_rejected(DriverPlacement::SharedRuntime).await;
}

/// The same receipt with the driver on a thread of the carrier's own.
///
/// The terminal reason is what is under test here: a `Frame(Oversize)` raised
/// inside the driver has to reach a reader that woke on a closed channel, and
/// it now has to do that having been published from a different thread.
#[tokio::test(flavor = "multi_thread")]
async fn an_oversize_frame_is_rejected_from_its_prefix_on_a_dedicated_thread() {
    an_oversize_frame_is_rejected(DEDICATED).await;
}

async fn an_oversize_frame_is_rejected(placement: DriverPlacement) {
    let (mut answerer, offerer) =
        connect(carrier_config_on(Backpressure::default(), placement)).await;

    // One well-formed frame first: the positive control. Without it, a session
    // that never carried anything would produce the same "no data" result as a
    // rejection, and the test would prove nothing.
    offerer
        .send_frame(b"well formed")
        .await
        .expect("the control frame is writable");
    expect_frame(&mut answerer, b"well formed").await;

    // Now four bytes declaring a 4 GiB payload, and nothing else. No payload
    // exists to be read, so an `Oversize` here can only have come from the
    // prefix — which is the proof that the ceiling is checked before the
    // declared length is ever treated as a size to reserve.
    let (_peer_reader, peer_writer, _peer_control) = offerer.into_parts();
    peer_writer
        .send_message(u32::MAX.to_be_bytes().to_vec())
        .await
        .expect("the malformed message is put on the wire");

    let outcome = tokio::time::timeout(Duration::from_secs(10), answerer.recv_frame())
        .await
        .expect("the host reacts within the budget");

    match outcome {
        Err(NativeError::Frame(FrameError::Oversize { declared, max })) => {
            assert_eq!(declared, u64::from(u32::MAX));
            assert_eq!(max, MAX_FRAME_PAYLOAD_BYTES);
        },
        other => panic!("expected an oversize rejection, got {other:?}"),
    }

    let stats = answerer.stats();
    assert_eq!(
        stats.frames_received, 1,
        "only the control frame was ever decoded"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sustained_transfer_crosses_the_high_and_low_water_marks() {
    a_sustained_transfer_crosses_the_marks(DriverPlacement::SharedRuntime).await;
}

/// The same receipt with both drivers on threads of the carrier's own.
///
/// Backpressure is the placement's sharpest edge: the pause and resume
/// decisions are taken in the driver and the wait is served by a `watch` tick
/// the writer is parked on, so a driver on another thread and another runtime
/// is exactly the arrangement a missed wakeup would hide in. Same marks, same
/// hysteresis, same overshoot bound.
#[tokio::test(flavor = "multi_thread")]
async fn a_sustained_transfer_crosses_the_water_marks_on_a_dedicated_thread() {
    a_sustained_transfer_crosses_the_marks(DEDICATED).await;
}

async fn a_sustained_transfer_crosses_the_marks(placement: DriverPlacement) {
    // Deliberately small marks. The defaults are eight and two maximum frames,
    // which a loopback link drains faster than a test can fill; the point of
    // the policy being configurable is that a receipt can choose marks it can
    // actually observe being crossed.
    let backpressure = Backpressure::new(8 * 1024, 2 * 1024).expect("separated marks");
    assert!(backpressure.high_water_bytes() < MAX_QUEUED_BYTES);

    let (answerer, offerer) = connect(carrier_config_on(backpressure, placement)).await;

    const FRAMES: usize = 400;
    const PAYLOAD: usize = 1024;
    let payload = vec![0xa5u8; PAYLOAD];
    let frame_len = PAYLOAD + 4;

    let (_peer_reader, peer_writer, peer_control) = offerer.into_parts();
    let sender = tokio::spawn(async move {
        for _ in 0..FRAMES {
            peer_writer
                .send_frame(&payload)
                .await
                .expect("the deferring write eventually succeeds");
        }
        peer_writer
    });

    let (mut host_reader, _host_writer, host_control) = answerer.into_parts();
    let mut received = 0usize;
    while received < FRAMES {
        // A stall here is the interesting failure, so it reports both ends'
        // counters rather than an `Elapsed(())` that says only "something".
        let report = |what: &str, received: usize| -> String {
            format!(
                "{what} after {received}/{FRAMES} frames
  sender task finished: {}
  sender: {:?}
  host:   {:?}",
                sender.is_finished(),
                peer_control.stats(),
                host_control.stats(),
            )
        };
        match tokio::time::timeout(Duration::from_secs(10), host_reader.recv_frame()).await {
            Ok(Ok(Some(frame))) => {
                assert_eq!(frame.len(), PAYLOAD);
                received += 1;
            },
            Ok(Ok(None)) => panic!("{}", report("the stream ended", received)),
            Ok(Err(err)) => panic!(
                "{}",
                report(&format!("the session failed ({err})"), received)
            ),
            Err(_) => panic!("{}", report("the transfer stalled", received)),
        }
    }

    let peer_writer = sender.await.expect("the sender task finishes");
    drop(peer_writer);

    let sent = peer_control.stats();
    assert_eq!(sent.frames_sent as usize, FRAMES);

    // The high mark was reached: the driver stopped taking frames.
    assert!(
        sent.pauses > 0,
        "the high-water mark was never crossed in {FRAMES} frames: {sent:?}"
    );
    // The low mark was reached: it started again. Both counters together are
    // the hysteresis actually happening rather than a single threshold
    // oscillating.
    assert!(
        sent.resumes > 0,
        "the low-water mark never released the sender: {sent:?}"
    );
    assert!(
        sent.write_waits > 0,
        "the writer never had to wait, so nothing deferred: {sent:?}"
    );
    assert!(
        sent.peak_queued >= backpressure.high_water_bytes(),
        "peak queue {} never reached the high mark {}",
        sent.peak_queued,
        backpressure.high_water_bytes()
    );
    // And the mark actually held: the queue overshoots by at most the frames
    // already in flight when the decision was taken, never without bound.
    assert!(
        sent.peak_queued <= backpressure.high_water_bytes() + 4 * frame_len,
        "peak queue {} overshot the high mark {} by more than four frames",
        sent.peak_queued,
        backpressure.high_water_bytes()
    );

    let got = host_control.stats();
    assert_eq!(got.frames_received as usize, FRAMES);
    assert_eq!(got.bytes_received as usize, FRAMES * frame_len);
    assert!(got.peak_queued <= MAX_FRAME_BYTES, "the reader never sent");

    host_control.close().await.expect("a clean close");
    let _ = peer_control.join().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_terminates_promptly_and_nothing_spins() {
    cancellation_terminates_promptly(DriverPlacement::SharedRuntime).await;
}

/// The same receipt with both drivers on threads of the carrier's own.
///
/// This is the join-and-cancel contract itself: `cancel` must still be prompt,
/// `join` must still return `Err(Cancelled)` and not a generic "the task did
/// not finish", the iteration counter must still show no spin, and the
/// orphaned end must still shut down when closed.
#[tokio::test(flavor = "multi_thread")]
async fn cancellation_terminates_promptly_on_a_dedicated_thread() {
    cancellation_terminates_promptly(DEDICATED).await;
}

async fn cancellation_terminates_promptly(placement: DriverPlacement) {
    let (answerer, offerer) = connect(carrier_config_on(Backpressure::default(), placement)).await;

    offerer.send_frame(b"hello").await.expect("a live channel");
    let (mut host_reader, _host_writer, host_control) = answerer.into_parts();
    let frame = tokio::time::timeout(Duration::from_secs(10), host_reader.recv_frame())
        .await
        .expect("the frame arrives")
        .expect("the session is live")
        .expect("the stream has not ended");
    assert_eq!(frame, b"hello");

    // (1) An idle, connected session must not spin. The driver's wait is
    // floored at 1 ms, so a genuine hot loop would show ~1000 turns a second;
    // anything under half that is proof the floor is not being hit repeatedly.
    let before = host_control.stats().iterations;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let idle_turns = host_control.stats().iterations - before;
    assert!(
        idle_turns < 250,
        "an idle session turned {idle_turns} times in 500ms, which is a hot loop"
    );

    // (2) Cancellation is prompt and distinct. Not `Closed`, not a generic
    // failure: the caller can tell that the local side stopped.
    let (_peer_reader, _peer_writer, peer_control) = offerer.into_parts();
    assert!(
        !peer_control.is_finished(),
        "a live driver reports itself finished under {placement:?}"
    );
    let cancelled_at = Instant::now();
    peer_control.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(5), peer_control.join())
        .await
        .expect("the cancelled driver finishes within the budget");
    assert!(
        matches!(outcome, Err(NativeError::Cancelled)),
        "expected Cancelled, got {outcome:?}"
    );
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(2),
        "cancellation took {:?}",
        cancelled_at.elapsed()
    );

    // (3) The surviving side's peer has vanished, socket and all. A hot loop
    // on disconnect is an explicit C1 failure condition, so measure it: the
    // answerer either winds down or keeps waiting, but it does not burn.
    let before = host_control.stats().iterations;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let orphan_turns = host_control.stats().iterations - before;
    assert!(
        orphan_turns < 250,
        "a session whose peer vanished turned {orphan_turns} times in 500ms: {:?}",
        host_control.stats()
    );

    // (4) And the reader learns about it rather than hanging: either the
    // session has already ended, or it ends when asked to.
    drop(host_reader);
    let ended = tokio::time::timeout(Duration::from_secs(10), host_control.close())
        .await
        .expect("the orphaned session shuts down within the budget");
    assert!(
        matches!(ended, Ok(()) | Err(NativeError::Closed)),
        "an orphaned session should end, got {ended:?}"
    );
}

// ── the vanished peer ───────────────────────────────────────────────────────

/// The stack the surviving end's runtime gets: half of tokio's worker default.
///
/// The number is measured, not guessed, and the margins run both ways.
///
/// *Below*: 256 KiB is not enough for this crate's ordinary work at all — a
/// run at that size dies inside `Answerer::bind`, before a single `poll_output`
/// happens, because generating the DTLS certificate is itself stack-hungry in a
/// debug build. 512 KiB clears the handshake. So an overflow at 1 MiB cannot be
/// read as "the stack was never enough for anything"; the ICE exchange, the
/// DTLS handshake and hundreds of framed writes all complete on it first.
///
/// *Above*: with [`DEFAULT_SCTP_WINDOW_BYTES`] the vanished-peer moment needs
/// 640 KiB, measured; without the window ceiling it overflows 2 MiB, which is
/// the stack tokio's workers actually have and where the live crash happened.
/// 1 MiB therefore sits with 1.6x of headroom over the fixed code and still
/// catches the unfixed code, which overflowed at exactly this size.
///
/// Under [`DriverPlacement::DedicatedThread`] this thread carries only the
/// application half — the writer, the sampler and the handshake — because the
/// driver has moved to a stack of its own. That is what makes
/// `a_vanished_peer_at_the_full_window_survives_on_a_dedicated_thread` possible
/// at all: 1 MiB does not survive a 128 KiB window, and it does not have to.
const SURVIVOR_STACK_BYTES: usize = 1024 * 1024;

/// The turn rate a vanished-peer teardown is allowed to reach.
///
/// The wait is floored at 1 ms, so a driver in a genuine hot loop shows ~1000
/// turns a second; the legitimate cadence while a frame is held is the 5 ms
/// retry, or 200. What lives between the two is the socket-error path: a peer
/// whose port is gone answers each retransmission with an ICMP port
/// unreachable, which Windows delivers as a `ConnectionReset` on the next
/// `recv_from`, and every one of those is a turn that did not have to wait.
///
/// Measured on this machine, same workload and same 8 MiB stack throughout, so
/// the two variables separate cleanly:
///
/// ```text
/// shared runtime,   window  16 KiB   150 turns/s   <- the shipped default
/// shared runtime,   window 128 KiB   255 turns/s   <- the window's share
/// dedicated thread, window 128 KiB   640 turns/s   <- plus the placement's
/// ```
///
/// The window's share is str0m holding eight times as much to retransmit. The
/// placement's share is the driver no longer time-sharing a current-thread
/// runtime with the application measuring it — on a thread of its own it takes
/// every turn it is woken for, immediately, instead of queueing behind the
/// writer. Neither share is the 1 ms floor being hit, both are bounded by
/// `PEER_GONE_STREAK`, and both end the session promptly; so the ceilings are
/// set under 1000, which is the number that would mean "spinning".
const CALM_TURNS_PER_SECOND: u64 = 600;

/// The same ceiling for a driver that is not sharing a runtime with anything.
///
/// 900 rather than 600 because 640 was measured, repeatably, and it is not a
/// spin — see [`CALM_TURNS_PER_SECOND`]. Still under the ~1000 the 1 ms floor
/// would produce, which is what makes the assertion worth making at all.
const CALM_TURNS_PER_SECOND_UNSHARED: u64 = 900;

/// What the surviving end saw, reported back across the thread boundary.
struct SurvivorReport {
    frames_sent: usize,
    /// What the application writer learned when the session ended.
    ended: Result<Result<(), NativeError>, tokio::time::error::Elapsed>,
    /// What the driver itself returned.
    joined: Result<Result<(), NativeError>, tokio::time::error::Elapsed>,
    /// The busiest 200 ms window observed after the peer vanished, as a rate.
    peak_turns_per_second: u64,
    stats: webrtc_carrier::native::CarrierStats,
}

/// The surviving end: binds, answers, and then writes until it cannot.
async fn survive(
    config: CarrierConfig,
    offer_rx: std::sync::mpsc::Receiver<String>,
    answer_tx: std::sync::mpsc::Sender<String>,
    vanished: Arc<AtomicU64>,
) -> SurvivorReport {
    const PAYLOAD_BYTES: usize = 8 * 1024;
    const PUMP_CEILING: usize = 20_000;
    const SURVIVOR_BUDGET: Duration = Duration::from_secs(40);

    let mut answerer = Answerer::bind(AnswererConfig {
        bind: "127.0.0.1:0".parse().expect("a literal address"),
        advertise: Vec::new(),
        carrier: config,
    })
    .await
    .expect("the surviving answerer binds");

    let offer = offer_rx.recv().expect("the peer's offer arrives");
    let answer = answerer.answer(&offer).expect("an SDP answer");
    answer_tx.send(answer).expect("the peer takes the answer");

    let carrier = answerer
        .accept()
        .await
        .expect("the surviving end's channel opens");
    let (reader, writer, control) = carrier.into_parts();

    // Turns per window, counted only once the peer is gone: before that a fast
    // loop is the transfer working, not a spin.
    let peak = Arc::new(AtomicU64::new(0));
    let sampler = tokio::spawn({
        let probe = writer.clone();
        let peak = Arc::clone(&peak);
        let vanished = Arc::clone(&vanished);
        async move {
            let window = Duration::from_millis(200);
            let mut last = probe.stats().iterations;
            loop {
                tokio::time::sleep(window).await;
                let now = probe.stats().iterations;
                let turns = now.saturating_sub(last);
                last = now;
                if vanished.load(Ordering::Relaxed) == 1 {
                    peak.fetch_max(turns * 5, Ordering::Relaxed);
                }
            }
        }
    });

    let payload = vec![0xc3u8; PAYLOAD_BYTES];
    let mut frames_sent = 0usize;
    let ended = tokio::time::timeout(SURVIVOR_BUDGET, async {
        loop {
            match writer.send_frame(&payload).await {
                Ok(()) => {
                    frames_sent += 1;
                    if frames_sent >= PUMP_CEILING {
                        break Ok(());
                    }
                },
                Err(err) => break Err(err),
            }
        }
    })
    .await;

    sampler.abort();
    let stats = control.stats();
    drop(writer);
    drop(reader);
    let joined = tokio::time::timeout(Duration::from_secs(15), control.join()).await;

    SurvivorReport {
        frames_sent,
        ended,
        joined,
        peak_turns_per_second: peak.load(Ordering::Relaxed),
        stats,
    }
}

/// The vanishing end: reads a little, stops reading, then disappears.
async fn vanish(
    config: CarrierConfig,
    offer_tx: std::sync::mpsc::Sender<String>,
    answer_rx: std::sync::mpsc::Receiver<String>,
    vanished: Arc<AtomicU64>,
) {
    const PREFIX_FRAMES: usize = 8;
    const FILL: Duration = Duration::from_millis(1_500);

    let mut offerer = Offerer::create(&config.channel_label).await;
    offer_tx
        .send(offerer.offer.clone())
        .expect("the survivor takes the offer");
    let answer = tokio::task::spawn_blocking(move || answer_rx.recv())
        .await
        .expect("the blocking wait finishes")
        .expect("an answer arrives");
    offerer.accept_answer(&answer);

    let peer_config = CarrierConfig {
        local_dtls_role: FingerprintRole::Client,
        ..config
    };
    let peer = serve(offerer.rtc, offerer.socket, peer_config)
        .await
        .expect("the vanishing peer's channel opens");
    let (mut peer_reader, peer_writer, peer_control) = peer.into_parts();

    // Read enough to prove the transfer is genuinely running before it is
    // interrupted. Without this the test could pass on a session that never
    // carried anything.
    for _ in 0..PREFIX_FRAMES {
        let frame = tokio::time::timeout(Duration::from_secs(20), peer_reader.recv_frame())
            .await
            .expect("the transfer starts within the budget")
            .expect("the session is live")
            .expect("the stream has not ended");
        assert!(!frame.is_empty());
    }

    // Now stop reading. What the survivor writes piles up: first in this end's
    // delivery queue, then in its socket buffer, and finally — once SCTP stops
    // acknowledging — in the survivor's own SCTP send queue, which is the state
    // the defect needs.
    tokio::time::sleep(FILL).await;

    // Vanish. `cancel` sends nothing: no close, no stream reset, no goodbye.
    // The driver returns, drops the socket, and the survivor is left talking to
    // a port that is not there — which is what a navigated-away tab looks like.
    vanished.store(1, Ordering::Relaxed);
    peer_control.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), peer_control.join()).await;
    drop(peer_writer);
    drop(peer_reader);
}

/// A peer that disappears mid-transfer must end the session, not the process.
///
/// The live failure this pins: a Chrome peer receiving an echo stream navigated
/// away with the transfer in flight, and the native host died with
/// `thread 'tokio-rt-worker' has overflowed its stack` rather than reporting a
/// dead session. Nothing about that is browser-specific — it needs a deep SCTP
/// send queue and a peer that stops acknowledging, both of which two str0m ends
/// on loopback can produce.
#[test]
fn a_vanished_peer_mid_transfer_ends_the_session_without_crashing() {
    let config = carrier_config(Backpressure::default());
    let window = config.sctp_window_bytes;
    assert_eq!(window, DEFAULT_SCTP_WINDOW_BYTES);
    let report = run_vanished_peer(config, SURVIVOR_STACK_BYTES);
    assert_vanished_peer(&report, window, Binds::ThisCrate, CALM_TURNS_PER_SECOND);
}

/// The same vanish, at str0m's own window, with the driver on its own stack.
///
/// This is the receipt for [`DEDICATED_DRIVER_STACK_BYTES`]. The moment being
/// survived is the same one the test above pins — a peer that stops
/// acknowledging while SCTP holds a full window, so one `poll_output` recurses
/// once per queued packet — but the window is 128 KiB rather than 16 KiB, which
/// is eight times the recursion depth and the size that overflows a 2 MiB tokio
/// worker outright.
///
/// The survivor's *application* thread stays at [`SURVIVOR_STACK_BYTES`], which
/// is the point of the placement: 1 MiB is nowhere near enough for this window,
/// and the run survives anyway because the driver is no longer on it. Measured
/// on a debug build, one process per size because an overflow aborts:
///
/// ```text
/// window 128 KiB, driver stack  512 KiB   overflows
/// window 128 KiB, driver stack    1 MiB   overflows
/// window 128 KiB, driver stack    2 MiB   overflows  <- a tokio worker's size
/// window 128 KiB, driver stack    4 MiB   survives
/// window 128 KiB, driver stack    8 MiB   survives   <- the default, 2x margin
/// ```
///
/// The 2 MiB row is the control, and it was run both ways: the same window on
/// `DriverPlacement::SharedRuntime` with a 2 MiB thread under it overflows,
/// exactly as the live crash did, while a 16 KiB window on that same 2 MiB
/// thread survives. So this test passing is a fact about the placement, not
/// about the machine having a generous default stack somewhere.
///
/// An overflow aborts the process rather than failing an assertion, so this
/// test passing *at all* is the proof; the assertions below are about the
/// session ending well.
#[test]
fn a_vanished_peer_at_the_full_window_survives_on_a_dedicated_thread() {
    let config = CarrierConfig {
        open_timeout: HANDSHAKE_BUDGET,
        idle_timeout: Duration::from_secs(3),
        ..CarrierConfig::dedicated(DEDICATED_DRIVER_STACK_BYTES, DEDICATED_SCTP_WINDOW_BYTES)
    };
    let window = config.sctp_window_bytes;
    assert_eq!(window, DEDICATED_SCTP_WINDOW_BYTES);
    let report = run_vanished_peer(config, SURVIVOR_STACK_BYTES);
    assert_vanished_peer(
        &report,
        window,
        Binds::Str0m,
        CALM_TURNS_PER_SECOND_UNSHARED,
    );
}

/// Which ceiling actually stopped the driver handing SCTP the next frame.
///
/// Below str0m's own `MAX_BUFFERED_ACROSS_STREAMS` the two are distinguishable
/// and this crate's check is what fires. *At* 128 KiB they coincide, and str0m
/// wins the race: it refuses the write that would cross its limit, so SCTP
/// tops out just under the ceiling — 122,940 bytes measured, fifteen 8 KiB
/// frames — and `window_holds` stays at zero because this crate's `>=` never
/// becomes true. Setting the window to str0m's ceiling is therefore the same as
/// switching this crate's ceiling off, which is a thing worth naming rather
/// than a thing to assert around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binds {
    /// This crate's `sctp_window_bytes` is the binding constraint.
    ThisCrate,
    /// str0m's own `MAX_BUFFERED_ACROSS_STREAMS` is.
    Str0m,
}

fn run_vanished_peer(config: CarrierConfig, survivor_stack_bytes: usize) -> SurvivorReport {
    let (offer_tx, offer_rx) = std::sync::mpsc::channel::<String>();
    let (answer_tx, answer_rx) = std::sync::mpsc::channel::<String>();
    let vanished = Arc::new(AtomicU64::new(0));

    let survivor = std::thread::Builder::new()
        .name("vanished-peer-survivor".to_owned())
        .stack_size(survivor_stack_bytes)
        .spawn({
            let config = config.clone();
            let vanished = Arc::clone(&vanished);
            move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a current-thread runtime on the small stack")
                    .block_on(survive(config, offer_rx, answer_tx, vanished))
            }
        })
        .expect("the small-stack survivor thread starts");

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime for the vanishing peer")
        .block_on(vanish(config, offer_tx, answer_rx, vanished));

    survivor.join().expect("the survivor thread did not die")
}

fn assert_vanished_peer(
    report: &SurvivorReport,
    window: usize,
    binds: Binds,
    calm_turns_per_second: u64,
) {
    println!(
        "survivor: {} frames written, window {window}, ended={:?}, joined={:?}, peak {} turns/s after the vanish\n  {:?}",
        report.frames_sent, report.ended, report.joined, report.peak_turns_per_second, report.stats
    );

    assert!(
        report.frames_sent > 0,
        "the survivor never wrote anything, so nothing was in flight to interrupt"
    );
    // The ceiling is the fix, so assert it was actually reached rather than
    // trusting that a passing test means it was in play. Which ceiling that is
    // depends on where the window was set: see `Binds`.
    match binds {
        Binds::ThisCrate => assert!(
            report.stats.window_holds > 0,
            "the SCTP window ceiling never engaged, so this run did not exercise the fix: {:?}",
            report.stats
        ),
        // str0m refuses the write before this crate's check can fire, so the
        // positive control is the depth itself: SCTP has to have actually
        // filled, or the deep recursion this stack is sized for never happened.
        Binds::Str0m => assert!(
            report.stats.sctp_buffered >= window / 2,
            "SCTP only reached {} bytes of a {window}-byte window, so this run never drove              the recursion the dedicated stack exists for: {:?}",
            report.stats.sctp_buffered,
            report.stats
        ),
    }
    assert!(
        report.stats.sctp_buffered <= window + MAX_FRAME_BYTES,
        "SCTP held {} bytes, past the {window}-byte window plus one frame",
        report.stats.sctp_buffered
    );

    // (1) No hang. The writer learned the session was over rather than waiting
    //     on a peer that is never coming back.
    let ended = report
        .ended
        .as_ref()
        .expect("the surviving writer stopped waiting");
    // (2) A matchable reason, not an opaque stall.
    assert!(
        matches!(
            ended,
            Err(NativeError::Closed | NativeError::Socket(_) | NativeError::Write(_))
        ),
        "expected a matchable end for a vanished peer, got {ended:?}"
    );
    // (3) The driver itself finished, and finished promptly.
    let joined = report
        .joined
        .as_ref()
        .expect("the driver finished within the budget");
    assert!(
        matches!(
            joined,
            Ok(()) | Err(NativeError::Closed | NativeError::Socket(_))
        ),
        "expected the driver to wind down, got {joined:?}"
    );
    // (4) And it did not burn while doing it. See `CALM_TURNS_PER_SECOND` for
    //     what the ceiling is measured against and why the unshared driver gets
    //     a higher one.
    assert!(
        report.peak_turns_per_second < calm_turns_per_second,
        "a session whose peer vanished turned {} times a second, past {calm_turns_per_second}: {:?}",
        report.peak_turns_per_second,
        report.stats
    );
}

// ── the wildcard bind ───────────────────────────────────────────────────────

/// A wildcard bind must connect on the address its candidates advertise.
///
/// [`AnswererConfig::bind`] defaults to `0.0.0.0:0`, which is what a host that
/// wants one socket reachable on both loopback and the LAN uses. That path was
/// broken in a way nothing reported: the driver told str0m every datagram had
/// been addressed to the socket's own `0.0.0.0:<port>`, str0m's ICE agent
/// matches that destination against its local candidates by exact equality, no
/// candidate is ever the unspecified address, and so every binding request was
/// counted as received and then discarded inside the engine. A browser sent 78
/// of them and got no reply; both ends sat in `checking` until the open
/// timeout, and the same code with a concrete `--bind` connected in 212 ms.
///
/// This is that configuration end to end. Before the fix it fails by timeout.
#[tokio::test(flavor = "multi_thread")]
async fn a_wildcard_bind_connects_on_the_address_its_candidates_advertise() {
    a_wildcard_bind_connects(DriverPlacement::SharedRuntime).await;
}

/// The same receipt with both drivers on threads of the carrier's own.
///
/// The dedicated placement hands its socket over as a `std::net::UdpSocket` and
/// re-registers it inside the new runtime, so "which address str0m is told a
/// datagram arrived on" travels a path the shared placement never takes. A
/// wildcard bind is where getting that wrong shows up as a timeout rather than
/// as an error, which is what makes this the right test to run twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_wildcard_bind_connects_on_its_advertised_address_on_a_dedicated_thread() {
    a_wildcard_bind_connects(DEDICATED).await;
}

async fn a_wildcard_bind_connects(placement: DriverPlacement) {
    let config = carrier_config_on(Backpressure::default(), placement);
    let mut offerer = Offerer::create(&config.channel_label).await;

    let mut answerer = Answerer::bind(AnswererConfig {
        bind: "0.0.0.0:0".parse().expect("a literal address"),
        advertise: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        carrier: config.clone(),
    })
    .await
    .expect("the answerer binds to the wildcard address");

    // The two addresses really are different here, which is the whole point:
    // one is where the socket listens, the other is what the peer is told.
    assert!(
        answerer.local_addr().ip().is_unspecified(),
        "this test is only meaningful on a wildcard bind, got {}",
        answerer.local_addr()
    );
    assert_eq!(
        answerer.candidate_addr(),
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            answerer.local_addr().port()
        ),
        "the declared candidate is the advertised address on the bound port"
    );

    let answer = answerer.answer(&offerer.offer).expect("an SDP answer");
    assert!(
        answer.contains("127.0.0.1"),
        "the answer must carry the advertised candidate, not the wildcard"
    );
    offerer.accept_answer(&answer);

    let peer_config = CarrierConfig {
        local_dtls_role: FingerprintRole::Client,
        ..config.clone()
    };
    let (answered, offered) = tokio::join!(
        answerer.accept(),
        serve(offerer.rtc, offerer.socket, peer_config)
    );
    let mut answered = answered.expect("the wildcard-bound answerer's channel opens");
    let mut offered = offered.expect("the peer's channel opens");

    // Opened is not enough: a frame each way proves the nominated pair carries
    // data, not just that the handshake finished.
    offered
        .send_frame(b"through the wildcard")
        .await
        .expect("the peer writes");
    expect_frame(&mut answered, b"through the wildcard").await;
    answered
        .send_frame(b"and back again")
        .await
        .expect("the wildcard-bound host writes");
    expect_frame(&mut offered, b"and back again").await;

    let stats = answered.stats();
    assert_eq!(stats.malformed_datagrams, 0);
    assert!(
        stats.datagrams_received > 0,
        "the wildcard socket never fed the engine: {stats:?}"
    );

    answered.close().await.expect("a clean close");
    let _ = offered.close().await;

    // The other half of the fix is a refusal. `serve` on a wildcard-bound
    // socket has nothing to derive the destination from, and says so at the
    // call site rather than handing back a session that would never connect.
    let stray = UdpSocket::bind("0.0.0.0:0")
        .await
        .expect("a wildcard UDP port");
    let rtc = RtcConfig::new().clear_codecs().build(Instant::now());
    let refused = serve(rtc, stray, carrier_config(Backpressure::default())).await;
    assert!(
        matches!(refused, Err(NativeError::NoCandidate(_))),
        "expected a NoCandidate refusal for a wildcard bind, got {refused:?}"
    );
}

// ── what the window costs, and what the placement buys ──────────────────────

/// The headed run's shape, reproduced on loopback: 200 frames of 16 KiB.
const ECHO_FRAMES: usize = 200;
/// One frame's payload. Deliberately *larger* than
/// [`DEFAULT_SCTP_WINDOW_BYTES`], which is the whole pathology: a window
/// smaller than one frame can never hold two, so every frame waits for the one
/// before it to be acknowledged before the driver will hand it over.
const ECHO_PAYLOAD_BYTES: usize = 16 * 1024;

/// One echo pass: what it cost, and what the source end's driver did.
struct EchoRun {
    elapsed: Duration,
    source: CarrierStats,
}

/// Sends `ECHO_FRAMES` frames, has the far end echo each back, and times it.
///
/// Both ends run the same `config`, so the numbers describe one window setting
/// rather than an asymmetric pair.
async fn echo_round_trip(config: CarrierConfig) -> EchoRun {
    let (answerer, offerer) = connect(config).await;

    // The echo end reads and writes in *separate* tasks over an unbounded
    // hand-off. A single read-then-write loop couples the two directions head
    // to head: while it is parked in `send_frame` it is not reading, its
    // delivery queue fills, its driver stops reading the socket, and the peer's
    // datagrams pile up in the kernel until SCTP starts retransmitting. That
    // wedges the session on both placements and measures the deadlock rather
    // than the window.
    let (mut echo_reader, echo_writer, echo_control) = answerer.into_parts();
    let (relay_tx, mut relay_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let echo_in = tokio::spawn(async move {
        for round in 0..ECHO_FRAMES {
            match echo_reader.recv_frame().await {
                Ok(Some(frame)) => relay_tx.send(frame).expect("the relay stays open"),
                other => panic!("the echoing end failed reading frame {round}: {other:?}"),
            }
        }
    });
    let echo_out = tokio::spawn(async move {
        while let Some(frame) = relay_rx.recv().await {
            echo_writer
                .send_frame(&frame)
                .await
                .expect("the echoing end writes back");
        }
        echo_control
    });

    let (mut reader, writer, control) = offerer.into_parts();
    let payload = vec![0x7eu8; ECHO_PAYLOAD_BYTES];
    let started = Instant::now();
    let sender = tokio::spawn({
        let writer = writer.clone();
        let payload = payload.clone();
        async move {
            for _ in 0..ECHO_FRAMES {
                writer
                    .send_frame(&payload)
                    .await
                    .expect("the source writes");
            }
        }
    });

    for round in 0..ECHO_FRAMES {
        let frame = match reader.recv_frame().await {
            Ok(Some(frame)) => frame,
            other => panic!(
                "source read {round}: {other:?}
  source: {:?}",
                control.stats()
            ),
        };
        assert_eq!(frame.len(), ECHO_PAYLOAD_BYTES);
    }
    let elapsed = started.elapsed();
    let source = control.stats();

    sender.await.expect("the sending task finishes");
    echo_in.await.expect("the echoing end's reader finishes");
    let echo_control = echo_out.await.expect("the echoing end's writer finishes");
    drop(writer);
    drop(reader);
    let _ = control.close().await;
    let _ = echo_control.close().await;

    EchoRun { elapsed, source }
}

/// What the raised window does, and what it does not.
///
/// The window ceiling exists to bound str0m's recursion, and at 16 KiB it is
/// below one 16 KiB frame — so the driver can never hold two, and every frame
/// waits for the one before it to be acknowledged before it is handed over.
/// That is stop-and-wait, and on a link with a round trip it is expensive: the
/// headed LAN run measured **116.6 s against 1.5 s** for exactly these 200
/// frames, a 65x penalty paid entirely to keep the driver's recursion inside a
/// 2 MiB tokio worker.
///
/// **Loopback cannot reproduce that**, and this test does not pretend to. The
/// penalty is one round trip per frame and loopback has almost none, so what is
/// left is CPU in a debug build. Measured on this machine, three passes,
/// 200 x 16 KiB echoed:
///
/// ```text
/// placement    window    elapsed (median)   window_holds
/// shared        16 KiB   678 ms             524
/// shared        32 KiB   761 ms             311
/// shared        64 KiB   842 ms             662
/// shared       128 KiB   1.76 s             0
/// dedicated     16 KiB   743 ms             532
/// dedicated     32 KiB   751 ms             581
/// dedicated     64 KiB   1.98 s             872
/// dedicated    128 KiB   1.85 s             0
/// ```
///
/// Two things to read off it. The placement is throughput-neutral — same window,
/// either side of the thread boundary, same time within noise — which is what a
/// placement should be. And on loopback the raised window is *slower*, roughly
/// 2.6x, because a bigger burst is more work per `poll_output` and there is no
/// round trip for the pipelining to save. The raised window is a bet on latency,
/// not on loopback.
///
/// So the assertion here is the *mechanism*, which is machine-independent:
/// `window_holds` counts every turn the driver kept a frame back because SCTP
/// was already at the ceiling. At 16 KiB that is hundreds of holds for 200
/// frames; at str0m's own 128 KiB it is zero, because the ceiling never
/// engages. The wall-clock numbers are printed on every run rather than
/// asserted, because absolute times on a shared machine are not a property of
/// this crate.
#[tokio::test(flavor = "multi_thread")]
async fn a_raised_window_on_a_dedicated_thread_stops_holding_every_frame() {
    // The echo is heavy enough that the 3 s idle timeout the spin tests want
    // is the wrong instrument: a driver that spends a second draining a
    // backlog is not an idle one. This is the crate's own default order.
    let idle_timeout = Duration::from_secs(30);

    let baseline = echo_round_trip(CarrierConfig {
        idle_timeout,
        ..carrier_config(Backpressure::default())
    })
    .await;

    // The ceiling-off case is str0m's own MAX_BUFFERED_ACROSS_STREAMS (128 KiB),
    // not the shipped dedicated default (64 KiB, which keeps this crate's ceiling
    // engaged on purpose — see DEDICATED_SCTP_WINDOW_BYTES). Name it directly so
    // this test proves the mechanism regardless of what the default is set to.
    const STR0M_CEILING_BYTES: usize = 128 * 1024;
    let raised = echo_round_trip(CarrierConfig {
        open_timeout: HANDSHAKE_BUDGET,
        idle_timeout,
        ..CarrierConfig::dedicated(DEDICATED_DRIVER_STACK_BYTES, STR0M_CEILING_BYTES)
    })
    .await;

    let bytes = (ECHO_FRAMES * ECHO_PAYLOAD_BYTES) as f64;
    let rate = |elapsed: Duration| bytes / elapsed.as_secs_f64() / (1024.0 * 1024.0);
    println!(
        "echo {ECHO_FRAMES} x {ECHO_PAYLOAD_BYTES} B, one round trip each:\n  \
         shared runtime,   window {DEFAULT_SCTP_WINDOW_BYTES:>6} B: {:>10.3?} ({:.2} MiB/s), \
         {} window holds\n  \
         dedicated thread, window {STR0M_CEILING_BYTES:>6} B: {:>10.3?} ({:.2} MiB/s), \
         {} window holds",
        baseline.elapsed,
        rate(baseline.elapsed),
        baseline.source.window_holds,
        raised.elapsed,
        rate(raised.elapsed),
        raised.source.window_holds,
    );

    assert_eq!(baseline.source.frames_sent as usize, ECHO_FRAMES);
    assert_eq!(raised.source.frames_sent as usize, ECHO_FRAMES);

    // The pathology, as a number: a window below one frame holds the next frame
    // back over and over, for most of the frames in the transfer.
    assert!(
        baseline.source.window_holds >= ECHO_FRAMES as u64 / 2,
        "a {DEFAULT_SCTP_WINDOW_BYTES}-byte window against {ECHO_PAYLOAD_BYTES}-byte frames \
         held only {} times in {ECHO_FRAMES} frames, so this run did not reproduce stop-and-wait",
        baseline.source.window_holds
    );
    // And gone at str0m's own ceiling: nothing is ever held back, because this
    // crate's ceiling no longer binds.
    assert!(
        raised.source.window_holds < ECHO_FRAMES as u64 / 8,
        "the raised window still held {} times in {ECHO_FRAMES} frames, against {} at the default",
        raised.source.window_holds,
        baseline.source.window_holds
    );
}

// ── Property 8: the frame pump presents a byte stream ────────────────────────

/// The carrier's frames carry a *byte stream*, not a message queue.
///
/// This is what Graphshell's session loop rests on: it writes NDJSON with
/// `write_all` and reads it back with `BufReader::lines`, and neither end knows
/// a frame exists. The property that has to hold is that frame boundaries are
/// invisible — a line longer than one frame is split across several and arrives
/// as one line, and several short lines may share a frame and still arrive as
/// several lines.
///
/// Run over the same real DTLS loopback as everything else in this file, so the
/// chunking under test is the carrier's own, not a mock's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_frame_pump_hides_frame_boundaries_from_a_line_reader() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use webrtc_carrier::native::stream_over_frames;

    let (answerer, offerer) = connect(CarrierConfig::default()).await;
    let (a_read, a_write, _a_control) = answerer.into_parts();
    let (b_read, b_write, _b_control) = offerer.into_parts();
    let (mut host, _host_pump) = stream_over_frames(a_read, a_write);
    let (peer, _peer_pump) = stream_over_frames(b_read, b_write);

    // One line far longer than a single frame, then two short ones. If frames
    // leaked through as message boundaries, the long line would arrive in
    // pieces and the short ones might be merged.
    let long = "x".repeat(MAX_FRAME_PAYLOAD_BYTES * 3 + 17);
    let written = format!("{long}\n{{\"id\":1}}\n{{\"id\":2}}\n");
    host.write_all(written.as_bytes())
        .await
        .expect("the host writes its lines");
    host.flush().await.expect("the host flushes");

    let mut lines = BufReader::new(peer).lines();
    let first = lines
        .next_line()
        .await
        .expect("the peer reads")
        .expect("a first line");
    assert_eq!(
        first.len(),
        long.len(),
        "a line spanning several frames must arrive as one line"
    );
    assert_eq!(first, long, "and byte-for-byte the same line");

    assert_eq!(
        lines.next_line().await.expect("the peer reads").as_deref(),
        Some("{\"id\":1}"),
        "two short lines sharing a frame stay two lines"
    );
    assert_eq!(
        lines.next_line().await.expect("the peer reads").as_deref(),
        Some("{\"id\":2}"),
    );
}

/// Dropping the stream ends the pump, and it says why.
///
/// A session that ends has to stop the task carrying it; a pump left spinning
/// on a carrier nobody reads is the "without a hot loop" failure this crate
/// already tests for elsewhere, arriving by a new route.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_stream_ends_its_pump() {
    use webrtc_carrier::native::stream_over_frames;

    let (answerer, offerer) = connect(CarrierConfig::default()).await;
    let (a_read, a_write, _a_control) = answerer.into_parts();
    let (b_read, b_write, _b_control) = offerer.into_parts();
    let (host, host_pump) = stream_over_frames(a_read, a_write);
    let (_peer, _peer_pump) = stream_over_frames(b_read, b_write);

    drop(host);

    let end = tokio::time::timeout(Duration::from_secs(5), host_pump)
        .await
        .expect("the pump ends promptly rather than spinning")
        .expect("the pump task does not panic");
    assert!(
        end.is_clean(),
        "dropping the stream is an ordinary end, got {end}"
    );
}
