//! Carrier pumps: one framed byte stream per interface, both directions in one task.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::iface::hdlc::{Deframer, frame};

use super::interface::{InterfaceId, open_frame};
use super::queue::OutboundPackets;
use super::shared::Shared;

/// A write that makes no progress for this long ends the pump, as Backbone's egress dead time
/// does (`BackboneInterface.py` 63-69, 476-499): a peer that stops reading must not keep its
/// interface, and the routes through it, alive.
pub(super) const DEAD_TIME: Duration = Duration::from_secs(12);

/// How packets are framed on a stream (`TCPInterface.py` 55-67, 322-388).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Framing {
    /// HDLC, as RNS's TCP, Local and Pipe interfaces.
    #[default]
    Hdlc,
    /// KISS data frames, as a TCP client with `kiss_framing = yes`. Seam: carried as HDLC
    /// until unit V4 (C5) implements it.
    Kiss,
}

/// Why a pump ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PumpEnd {
    /// The peer closed the stream.
    Eof,
    /// A write made no progress for [`DEAD_TIME`].
    WriteStall,
    /// The stream failed.
    Error,
    /// The endpoint closed the interface or stopped routing.
    Closed,
}

/// Carry interface `id` over one byte stream until either direction ends, which ends both: a
/// writer that fails must not leave the reader holding a half-dead interface.
///
/// Inbound frames are IFAC-opened and decoded; a full router queue is awaited rather than
/// dropped, so TCP flow control slows a flooding peer. A queued packet that cannot be encoded
/// is dropped and counted, and the pump carries on.
pub(super) async fn run<R, W>(
    shared: &Shared,
    id: InterfaceId,
    out: &mut OutboundPackets,
    framing: Framing,
    mut reader: R,
    mut writer: W,
) -> PumpEnd
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let _ = framing;
    let ifac = out.ifac.clone();
    let (frame_limit, stats) = (out.frame_limit.clone(), out.stats.clone());
    let write = async {
        while let Some(pkt) = out.recv().await {
            let Ok(wire) = out.encode(&pkt) else {
                shared.with_iface(id, |iface| iface.unsendable.fetch_add(1, Ordering::Relaxed));
                continue;
            };
            let sent = async {
                writer.write_all(&frame(&wire)).await?;
                writer.flush().await
            };
            match tokio::time::timeout(DEAD_TIME, sent).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return PumpEnd::Error,
                Err(_) => return PumpEnd::WriteStall,
            }
        }
        PumpEnd::Closed
    };
    let read = async {
        let mut deframer = Deframer::with_limit(frame_limit.load(Ordering::Acquire));
        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf).await {
                Ok(0) => return PumpEnd::Eof,
                Ok(n) => n,
                Err(_) => return PumpEnd::Error,
            };
            for raw in deframer.push(&buf[..n]) {
                let limit = frame_limit.load(Ordering::Acquire);
                if let Ok(pkt) = open_frame(&raw, ifac.as_ref(), limit, &stats)
                    && !shared.inbound_queue.send(id, pkt).await
                {
                    return PumpEnd::Closed;
                }
            }
        }
    };
    tokio::select! {
        end = write => end,
        end = read => end,
    }
}

/// Carry one TCP connection: online while it lasts, then offline with the backlog discarded,
/// since RNS sends nothing on an interface that is down (`Transport.py` 1449).
///
/// Only an offline-to-online transition discards on entry: an interface registered online
/// already holds what callers queued since attaching (an announce sent right after
/// `attach_tcp_client`), and that must go out.
pub(super) async fn carry_tcp(
    shared: &Shared,
    id: InterfaceId,
    online: &AtomicBool,
    out: &mut OutboundPackets,
    stream: TcpStream,
    framing: Framing,
) -> PumpEnd {
    if !online.swap(true, Ordering::AcqRel) {
        out.discard();
    }
    let (reader, writer) = stream.into_split();
    let end = run(shared, id, out, framing, reader, writer).await;
    online.store(false, Ordering::Release);
    out.discard();
    end
}
