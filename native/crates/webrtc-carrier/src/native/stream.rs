// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A byte stream over the carrier's frames.
//!
//! The carrier moves bounded frames. Notochord's handshake and Graphshell's
//! session loop both want an `AsyncRead + AsyncWrite`, and neither should learn
//! what a data channel is to get one. [`stream_over_frames`] is the seam: it
//! hands back a stream and runs a pump that carries frame payloads onto it and
//! whatever is written to it back out as frames.
//!
//! ## Why a pump and a duplex rather than an `AsyncRead` impl
//!
//! Writing `poll_read`/`poll_write` directly over the frame channels means
//! reimplementing the backpressure policy in poll form — the high/low water
//! wait is an `.await` on a watch channel, and there is no honest way to spell
//! that as a `Poll::Pending` without a waker the channel does not expose. A
//! pump gets it for free by simply awaiting [`FrameWriter::send_frame`], which
//! is the policy already written and already tested.
//!
//! This is also the shape `graphshell::browser_carrier` already uses for the
//! same reason on the native-messaging lane: keep the framing at the edge, put
//! a private duplex between it and everything that wants a stream.
//!
//! ## Where the boundary sits
//!
//! Frames are a chunking of the byte stream, not a message boundary the stream
//! preserves. A reader gets a continuous stream and reassembles whatever
//! framing it has of its own — NDJSON lines, in Graphshell's case. Outbound
//! bytes are cut at [`MAX_FRAME_PAYLOAD_BYTES`], so a long line becomes several
//! frames and arrives as one line again.

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;

use crate::MAX_FRAME_PAYLOAD_BYTES;
use crate::native::error::NativeError;
use crate::native::session::{FrameReader, FrameWriter};

/// How much either direction may buffer before the pump stops reading on.
///
/// Not a protocol limit: the carrier's own high and low water marks are what
/// bound the outbound queue, and this only bounds the handoff between the pump
/// and whatever holds the other end. Sized to a few maximum frames so a peer
/// sending flat out does not stall on the duplex before the carrier's own
/// backpressure has a chance to speak.
pub const STREAM_BUFFER_BYTES: usize = 4 * MAX_FRAME_PAYLOAD_BYTES;

/// Why a pump stopped.
#[derive(Debug)]
pub enum PumpEnd {
    /// The peer closed the carrier. The ordinary end.
    PeerClosed,
    /// Whatever held the stream dropped it or shut it down. Also ordinary.
    StreamClosed,
    /// The carrier failed.
    Carrier(NativeError),
    /// The stream failed.
    Stream(std::io::Error),
}

impl std::fmt::Display for PumpEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PumpEnd::PeerClosed => write!(f, "the peer closed the carrier"),
            PumpEnd::StreamClosed => write!(f, "the stream was closed locally"),
            PumpEnd::Carrier(error) => write!(f, "carrier failed: {error}"),
            PumpEnd::Stream(error) => write!(f, "stream failed: {error}"),
        }
    }
}

impl PumpEnd {
    /// Whether this end is the ordinary one rather than a failure.
    pub fn is_clean(&self) -> bool {
        matches!(self, PumpEnd::PeerClosed | PumpEnd::StreamClosed)
    }
}

/// Present one carrier's frames as a byte stream.
///
/// Returns the stream to hand to whatever wants an `AsyncRead + AsyncWrite`,
/// and the pump's handle. Dropping the stream ends the pump; so does the peer
/// closing the carrier.
pub fn stream_over_frames(
    reader: FrameReader,
    writer: FrameWriter,
) -> (DuplexStream, JoinHandle<PumpEnd>) {
    stream_over_frames_with(reader, writer, STREAM_BUFFER_BYTES)
}

/// [`stream_over_frames`] with an explicit handoff buffer, for tests that want
/// to provoke the boundary.
pub fn stream_over_frames_with(
    mut reader: FrameReader,
    writer: FrameWriter,
    buffer_bytes: usize,
) -> (DuplexStream, JoinHandle<PumpEnd>) {
    let (near, far) = tokio::io::duplex(buffer_bytes);
    let (mut far_read, mut far_write) = tokio::io::split(far);

    let pump = tokio::spawn(async move {
        let mut outbound = vec![0u8; MAX_FRAME_PAYLOAD_BYTES];
        loop {
            tokio::select! {
                // Frames arriving from the peer become bytes on the stream.
                frame = reader.recv_frame() => match frame {
                    Ok(Some(payload)) => {
                        if let Err(error) = far_write.write_all(&payload).await {
                            // The holder dropped its end mid-frame. Ordinary on
                            // a session that ended, so it is not a failure.
                            return stream_end(error);
                        }
                    }
                    Ok(None) => return PumpEnd::PeerClosed,
                    Err(error) => return PumpEnd::Carrier(error),
                },
                // Bytes written to the stream become frames to the peer.
                read = far_read.read(&mut outbound) => match read {
                    Ok(0) => return PumpEnd::StreamClosed,
                    Ok(n) => {
                        if let Err(error) = writer.send_frame(&outbound[..n]).await {
                            return PumpEnd::Carrier(error);
                        }
                    }
                    Err(error) => return stream_end(error),
                },
            }
        }
    });

    (near, pump)
}

/// A broken pipe on the handoff is the holder having gone away, which is a
/// clean end rather than a fault.
fn stream_end(error: std::io::Error) -> PumpEnd {
    match error.kind() {
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof => PumpEnd::StreamClosed,
        _ => PumpEnd::Stream(error),
    }
}
