// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The native runtime adapter: a str0m answerer over a tokio UDP socket.
//!
//! Feature-gated on `native`, and off by default. The crate's default build is
//! the shared core and must keep compiling for `wasm32-unknown-unknown`
//! untouched; everything in this module — a socket, a runtime, an OS clock — is
//! precisely what that build refuses to contain.
//!
//! ## What str0m is, and what that costs
//!
//! [`str0m`] is Sans-IO. An `Rtc` instance owns no socket, no thread, no async
//! task, and no timer: it is a state machine whose only motion comes from calls
//! into it. Everything a WebRTC stack usually hides — reading datagrams,
//! writing them, firing retransmit timers, deciding when to wake — is the
//! adapter's, and [`driver`](self) is that adapter.
//!
//! That is the right shape for this codebase. Notochord's handshake core and
//! this carrier's own core are both deliberately I/O-free, so the carrier's
//! WebRTC engine being I/O-free too means there is exactly one place in the
//! native stack that knows what a socket is, and it is a file you can read.
//!
//! The three decisions Sans-IO forces, recorded here because they are the
//! interesting part of C1 rather than an implementation detail:
//!
//! - **What drives the loop.** Four inputs, one per turn, taken by `select!`:
//!   the control command (biased first, so cancellation is never starved), a
//!   datagram, a frame from the writer, and the timer. Everything else is
//!   output.
//! - **Timer granularity.** str0m hands back an absolute deadline, sometimes
//!   already past. The wait is clamped to a 1 ms floor and a 1 s ceiling, with
//!   a 5 ms retry cadence while a frame is held. Without the floor, an engine
//!   that keeps asking for "now" is the hot loop C1 names as a failure.
//! - **How cancellation unwinds.** [`CarrierControl`] holds the only command
//!   sender; dropping it cancels. The driver returns
//!   [`NativeError::Cancelled`], publishes that as the session's terminal
//!   reason, and *then* drops its channel ends — so a reader or writer waking
//!   on a closed channel finds the reason already recorded rather than racing
//!   for it.
//!
//! ## Where the loop runs
//!
//! Sans-IO buys one more thing, and [`DriverPlacement`] spends it: a loop that
//! owns no thread can be *put* on one. The driver spawns nothing and needs only
//! tokio's io and time drivers, so hosting it on a thread of this crate's own
//! is a placement decision rather than a rewrite.
//!
//! The reason to take it is the stack. str0m's `do_poll_output` recurses once
//! per queued SCTP packet, so the driver's stack is what bounds
//! [`CarrierConfig::sctp_window_bytes`], and on a shared tokio worker that
//! stack is 2 MiB — which caps the window at 16 KiB, below one maximum frame,
//! which turns a pipelined transfer into a stop-and-wait one. Measured on a
//! LAN: 116.6 s against 1.5 s for 200 x 16 KiB frames.
//! [`CarrierConfig::dedicated`] is the pairing that undoes it, and
//! [`DriverPlacement`] carries both the stack table and the loopback
//! measurements that say when it is worth taking — the win is proportional to
//! the round trip, and on loopback there is none to win.
//!
//! ## Shape
//!
//! ```text
//! Answerer::bind        socket + Rtc + host candidate
//!    |  answer(offer) -> answer
//!    v
//! serve(rtc, socket)    spawns the driver, waits for ChannelOpen
//!    |
//!    +-- FrameReader     recv_frame  -> Option<Vec<u8>>
//!    +-- FrameWriter     send_frame  (defers) / try_send_frame (refuses)
//!    `-- CarrierControl  cancel / close / join / stats
//! ```
//!
//! ## What this half does not do
//!
//! It does not derive a [`crate::LinkChallenge`]. It exposes the two
//! role-tagged fingerprints the transcript needs
//! ([`Answerer::local_fingerprint`], [`Answerer::remote_fingerprint`]) and
//! stops there, because the nonces and the invite id are C2's and the core
//! generates neither. It also reports no authenticated initiator: WebRTC
//! authenticates a DTLS connection, not a subject, and pretending otherwise is
//! the one thing the plan's boundaries forbid outright.

mod answerer;
mod driver;
mod error;
mod loopback;
mod session;
mod stream;

pub use crate::native::answerer::{Answerer, AnswererConfig, fingerprint_from_str0m};
pub use crate::native::error::NativeError;
pub use crate::native::loopback::{LoopbackOfferer, loopback_pair};
pub use crate::native::session::{
    Carrier, CarrierConfig, CarrierControl, CarrierStats, DEDICATED_DRIVER_STACK_BYTES,
    DEDICATED_SCTP_WINDOW_BYTES, DEFAULT_CHANNEL_LABEL, DEFAULT_IDLE_TIMEOUT,
    DEFAULT_INBOUND_QUEUE_FRAMES, DEFAULT_OPEN_TIMEOUT, DEFAULT_OUTBOUND_QUEUE_FRAMES,
    DEFAULT_SCTP_WINDOW_BYTES, DriverPlacement, FrameReader, FrameWriter,
    MIN_DEDICATED_DRIVER_STACK_BYTES, SessionFingerprints, serve, serve_advertised,
};
pub use crate::native::stream::{
    PumpEnd, STREAM_BUFFER_BYTES, stream_over_frames, stream_over_frames_with,
};

/// The WebRTC engine this adapter drives, re-exported.
///
/// Pinned here rather than left to consumers so a test peer, a probe, or a
/// future offerer builds its `Rtc` from the same version this crate compiled
/// against. Two str0m copies in one process is a class of bug worth making
/// impossible to reach by accident.
pub use str0m;
