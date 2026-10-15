//! Carrier pumps: one framed byte stream per interface, both directions in one task.

use alloc::vec::Vec;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::iface::{hdlc, kiss};

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
    /// KISS data frames on port 0, as a TCP client with `kiss_framing = yes`.
    Kiss,
}

impl Framing {
    /// One packet on the wire: `FEND CMD_DATA escape(data) FEND` for KISS (`TCPInterface.py`
    /// 322-327), a flag pair per frame for HDLC, which stock Pipe's strict parser needs
    /// (`PipeInterface.py` 140-157).
    fn frame(self, wire: &[u8]) -> Vec<u8> {
        match self {
            Self::Hdlc => hdlc::frame(wire),
            Self::Kiss => kiss::encode(kiss::cmd::DATA, wire),
        }
    }

    fn deframer(self, limit: usize) -> Deframer {
        match self {
            Self::Hdlc => Deframer::Hdlc(hdlc::Deframer::with_limit(limit)),
            Self::Kiss => Deframer::Kiss(KissData::new(limit)),
        }
    }
}

enum Deframer {
    Hdlc(hdlc::Deframer),
    Kiss(KissData),
}

impl Deframer {
    fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        match self {
            Self::Hdlc(d) => d.push(chunk),
            Self::Kiss(d) => chunk.iter().filter_map(|&b| d.push(b)).collect(),
        }
    }
}

/// KISS data frames off a stream (`TCPInterface.py` 361-388): the command's port nibble is
/// masked and only DATA is kept. Unlike stock, a frame over `limit` or with a bad escape is
/// dropped whole rather than truncated or passed through, as is one closed mid-escape, and
/// every FEND starts a clean frame, so no escape state leaks into the next one. A FEND that
/// closes one frame also opens the next, where stock needs a FEND pair.
struct KissData {
    buf: Vec<u8>,
    limit: usize,
    command: Option<u8>,
    in_frame: bool,
    escaped: bool,
    poisoned: bool,
}

impl KissData {
    fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
            command: None,
            in_frame: false,
            escaped: false,
            poisoned: false,
        }
    }

    fn push(&mut self, byte: u8) -> Option<Vec<u8>> {
        if byte == kiss::FEND {
            let done = self.in_frame
                && !(self.poisoned || self.escaped)
                && self.command == Some(kiss::cmd::DATA);
            let frame = core::mem::take(&mut self.buf);
            *self = Self {
                in_frame: true,
                ..Self::new(self.limit)
            };
            return (done && !frame.is_empty()).then_some(frame);
        }
        if !self.in_frame || self.poisoned {
            return None;
        }
        let Some(command) = self.command else {
            self.command = Some(byte & 0x0F);
            return None;
        };
        if command != kiss::cmd::DATA {
            return None;
        }
        let byte = match (core::mem::take(&mut self.escaped), byte) {
            (false, kiss::FESC) => {
                self.escaped = true;
                return None;
            }
            (false, b) => b,
            (true, kiss::TFEND) => kiss::FEND,
            (true, kiss::TFESC) => kiss::FESC,
            (true, _) => {
                self.poisoned = true;
                return None;
            }
        };
        if self.buf.len() == self.limit {
            self.poisoned = true;
            self.buf = Vec::new();
        } else {
            self.buf.push(byte);
        }
        None
    }
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
    let ifac = out.ifac.clone();
    let (frame_limit, stats) = (out.frame_limit.clone(), out.stats.clone());
    let write = async {
        while let Some(pkt) = out.recv().await {
            let Ok(wire) = out.encode(&pkt) else {
                shared.with_iface(id, |iface| iface.unsendable.fetch_add(1, Ordering::Relaxed));
                continue;
            };
            let sent = async {
                writer.write_all(&framing.frame(&wire)).await?;
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
        let mut deframer = framing.deframer(frame_limit.load(Ordering::Acquire));
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
pub(super) async fn carry_tcp(
    shared: &Shared,
    id: InterfaceId,
    online: &AtomicBool,
    out: &mut OutboundPackets,
    stream: TcpStream,
    framing: Framing,
) -> PumpEnd {
    let (reader, writer) = stream.into_split();
    carry(shared, id, online, out, framing, reader, writer).await
}

/// Carry one connection of a carrier that outlives it, as [`carry_tcp`] does.
///
/// Only an offline-to-online transition discards on entry: an interface registered online
/// already holds what callers queued since attaching (an announce sent right after
/// `attach_tcp_client`), and that must go out.
pub(super) async fn carry<R, W>(
    shared: &Shared,
    id: InterfaceId,
    online: &AtomicBool,
    out: &mut OutboundPackets,
    framing: Framing,
    reader: R,
    writer: W,
) -> PumpEnd
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if !online.swap(true, Ordering::AcqRel) {
        out.discard();
    }
    let end = run(shared, id, out, framing, reader, writer).await;
    online.store(false, Ordering::Release);
    out.discard();
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn deframe(limit: usize, wire: &[u8]) -> Vec<Vec<u8>> {
        Framing::Kiss.deframer(limit).push(wire)
    }

    #[test]
    fn kiss_frames_round_trip_with_escapes() {
        let data = [kiss::FEND, 1, kiss::FESC, kiss::TFEND, kiss::TFESC];
        let wire = Framing::Kiss.frame(&data);
        assert_eq!(&wire[..2], &[kiss::FEND, kiss::cmd::DATA]);
        assert_eq!(&wire[2..4], &[kiss::FESC, kiss::TFEND]);
        let mut twice = wire.clone();
        twice.extend(&wire);
        assert_eq!(deframe(16, &twice), [data.to_vec(), data.to_vec()]);
    }

    #[test]
    fn kiss_masks_the_port_and_keeps_only_data() {
        let mut wire = kiss::encode(0x50, b"port five");
        wire.extend(kiss::encode(kiss::cmd::TXDELAY, b"x"));
        wire.extend([kiss::FEND, kiss::FEND]);
        assert_eq!(deframe(64, &wire), [b"port five".to_vec()]);
    }

    #[test]
    fn kiss_drops_oversize_and_broken_frames_whole() {
        let mut d = Framing::Kiss.deframer(4);
        assert!(d.push(&kiss::encode(0, &[7; 5])).is_empty());
        assert!(
            d.push(&[kiss::FEND, 0, 1, kiss::FESC, 9, 2, kiss::FEND])
                .is_empty()
        );
        // A dangling escape does not reach the next frame.
        assert!(d.push(&[kiss::FEND, 0, 1, kiss::FESC]).is_empty());
        assert_eq!(
            d.push(&kiss::encode(0, &[kiss::TFEND; 4])),
            [vec![kiss::TFEND; 4]]
        );
    }
}
