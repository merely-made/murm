// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The native answerer's failure vocabulary.
//!
//! C1 requires that close, cancellation, and write failure each arrive as a
//! *distinct, matchable* condition rather than as one opaque "the session
//! ended". That is why [`NativeError::Closed`], [`NativeError::Cancelled`] and
//! [`NativeError::Write`] are three variants and not three strings inside one:
//! a caller deciding whether to retry, reconnect, or give up is asking exactly
//! which of the three happened, and a carrier that answers "an error occurred"
//! has not propagated anything.
//!
//! The core's [`FrameError`] and [`FingerprintError`] are carried through
//! rather than re-spelled, so a peer that puts an oversize length prefix on the
//! wire surfaces the *same* value the offline decoder produces.

use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;

use crate::error::{FingerprintError, FrameError};

/// Why the native carrier refused, stopped, or never started.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NativeError {
    /// The UDP socket could not be bound.
    #[error("binding the carrier socket to {addr} failed: {source}")]
    Bind {
        /// The address that was asked for.
        addr: SocketAddr,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// The UDP socket failed while the session was running.
    #[error("the carrier socket failed: {0}")]
    Socket(#[source] std::io::Error),

    /// No usable local ICE candidate could be formed.
    ///
    /// This crate does no interface discovery — str0m does none either, and
    /// saying so in an error is better than guessing an address. Supply
    /// `advertise` addresses when binding to an unspecified IP.
    #[error("no usable local ICE candidate: {0}")]
    NoCandidate(String),

    /// An SDP offer could not be parsed, or an answer could not be produced.
    #[error("signaling: {0}")]
    Signaling(String),

    /// str0m refused an operation, or its state machine failed.
    #[error("webrtc engine: {0}")]
    Engine(String),

    /// The data channel is gone: the peer closed it, or this end already did.
    ///
    /// Distinct from [`Cancelled`](Self::Cancelled): the session ended the way
    /// a session is supposed to end.
    #[error("the data channel is closed")]
    Closed,

    /// The carrier task was cancelled, or its control handle was dropped.
    ///
    /// Distinct from [`Closed`](Self::Closed): nothing was negotiated with the
    /// peer, the local side simply stopped.
    #[error("the carrier task was cancelled")]
    Cancelled,

    /// A write was refused because the outbound queue is at or above the
    /// high-water mark.
    ///
    /// Only [`FrameWriter::try_send_frame`](super::FrameWriter::try_send_frame)
    /// produces this; the awaiting form defers instead.
    #[error("write refused: {queued} bytes queued, high water is {high_water}")]
    WouldBlock {
        /// Bytes queued when the write was attempted.
        queued: usize,
        /// The configured high-water mark.
        high_water: usize,
    },

    /// Handing a frame to SCTP failed.
    ///
    /// Distinct from [`Closed`](Self::Closed): the channel was open and the
    /// write itself was rejected.
    #[error("writing to the data channel failed: {0}")]
    Write(String),

    /// A framing rule was broken, on this end or by the peer.
    ///
    /// [`FrameError::Oversize`] arriving here means a peer declared a payload
    /// past the ceiling. It is raised from the four-byte prefix, so no buffer
    /// was reserved for the payload it announced.
    #[error("frame: {0}")]
    Frame(#[from] FrameError),

    /// A DTLS fingerprint could not be canonicalized for the link challenge.
    #[error("fingerprint: {0}")]
    Fingerprint(#[from] FingerprintError),

    /// No matching data channel opened before the configured deadline.
    #[error("no `{label}` data channel opened within {timeout:?}")]
    OpenTimeout {
        /// The label that was waited for.
        label: String,
        /// The deadline that expired.
        timeout: Duration,
    },

    /// The peer opened a channel that is not ordered and reliable.
    ///
    /// The carrier refuses rather than downgrading: an application that framed
    /// its protocol for an ordered reliable channel is not served by a lossy
    /// one that merely connects.
    #[error(
        "the peer's `{label}` channel is not ordered and reliable (ordered={ordered}, reliability={reliability})"
    )]
    UnreliableChannel {
        /// The channel's label.
        label: String,
        /// Whether the peer asked for ordered delivery.
        ordered: bool,
        /// The peer's reliability setting, rendered.
        reliability: String,
    },

    /// The receive buffer grew past its ceiling without yielding a frame.
    ///
    /// A structural impossibility if both ends frame correctly, and therefore
    /// an assertion rather than an expected condition.
    #[error("the receive buffer reached {len} bytes without a complete frame (ceiling {max})")]
    ReceiveOverflow {
        /// Bytes buffered.
        len: usize,
        /// The ceiling.
        max: usize,
    },

    /// The carrier task ended for a reason a handle cannot restate precisely.
    #[error("the carrier task ended: {0}")]
    Driver(String),

    /// The dedicated driver thread, or the runtime on it, could not be started.
    ///
    /// Only [`DriverPlacement::DedicatedThread`](super::DriverPlacement) can
    /// produce this: the OS refused a thread at the requested stack size, or
    /// tokio could not build a current-thread runtime on it. The shared-runtime
    /// placement borrows a runtime that already exists and has no equivalent.
    #[error("the carrier driver thread could not be started: {0}")]
    DriverThread(#[source] std::io::Error),

    /// The carrier task panicked or was aborted out from under its handle.
    #[error("the carrier task did not finish: {0}")]
    Join(String),
}
