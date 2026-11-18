//! Connect a loopback RNS TCP client to one direct-PHY radio for backend qualification.
//! Usage: radio_bridge SERIAL_PORT TCP_PORT BW_HZ TX_DBM DTR raw|paired
use retinue::hash::full_hash;
use retinue::iface::hdlc::{Deframer, MAX_FRAME, frame};
use std::{error::Error, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tulle::{
    airtime::AirtimeBudget,
    direct_phy_serial::{DirectPhySerialConfig, DirectPhySerialLink},
    radio_io::PacketRadio,
};

// Optional paired-bridge framing. Each frame carries the same eight digest bytes,
// the original packet length, and the fragment index. A receiver retains at most
// one bounded packet. A missing or reordered fragment never emits a partial packet.
const HEADER: usize = 11;

fn fragments(packet: &[u8], capacity: usize) -> impl Iterator<Item = Vec<u8>> + '_ {
    let hash = full_hash(packet);
    packet
        .chunks(capacity - HEADER)
        .enumerate()
        .map(move |(index, chunk)| {
            let mut part = Vec::with_capacity(HEADER + chunk.len());
            part.extend_from_slice(&hash[..8]);
            part.extend_from_slice(&(packet.len() as u16).to_be_bytes());
            part.push(index as u8);
            part.extend_from_slice(chunk);
            part
        })
}

#[derive(Default)]
struct Assembly {
    tag: [u8; 10],
    data: Vec<u8>,
}

impl Assembly {
    fn receive(&mut self, part: &[u8], capacity: usize) -> Option<Vec<u8>> {
        let (header, chunk) = part.split_at_checked(HEADER)?;
        let len = u16::from_be_bytes([header[8], header[9]]) as usize;
        let stride = capacity - HEADER;
        let offset = usize::from(header[10]) * stride;
        if len > MAX_FRAME || offset >= len || chunk.len() != stride.min(len - offset) {
            return None;
        }
        if offset == 0 {
            self.tag.copy_from_slice(&header[..10]);
            self.data.clear();
        }
        if self.tag != header[..10] || offset != self.data.len() {
            return None;
        }
        self.data.extend_from_slice(chunk);
        if self.data.len() == len {
            let packet = std::mem::take(&mut self.data);
            return (full_hash(&packet)[..8] == header[..8]).then_some(packet);
        }
        None
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let serial = args.next().ok_or("missing serial port")?;
    let port: u16 = args.next().ok_or("missing TCP port")?.parse()?;
    let mut profile = postilion::profile(args.next().ok_or("missing bandwidth Hz")?.parse()?);
    profile.tx_power_dbm = args.next().ok_or("missing transmit power")?.parse()?;
    let config = DirectPhySerialConfig {
        dtr: args.next().ok_or("missing DTR setting")?.parse()?,
        ..Default::default()
    };
    let fragment = match args.next().as_deref() {
        Some("raw") if args.next().is_none() => false,
        Some("paired") if args.next().is_none() => true,
        _ => return Err("expected raw or paired".into()),
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let mut radio =
        DirectPhySerialLink::open(&serial, profile, AirtimeBudget::new(60_000, 60_000), config)?;
    tokio::time::timeout(Duration::from_secs(10), radio.wait_online()).await??;
    eprintln!("{serial} online at {}", listener.local_addr()?);
    let (mut stream, _) = listener.accept().await?;
    let capacity = radio.max_frame_len();
    let mut assembly = Assembly::default();
    let mut decoder = Deframer::default();
    let mut buf = [0; 1024];
    loop {
        tokio::select! {
            count = stream.read(&mut buf) => {
                let count = count?;
                if count == 0 { break; }
                for packet in decoder.push(&buf[..count]) {
                    let size = packet.len();
                    eprintln!("{serial} outbound protected_bytes={size} capacity={capacity}");
                    if fragment {
                        for part in fragments(&packet, capacity) {
                            let len = part.len();
                            radio.send(part).await?;
                            if size > capacity { eprintln!("{serial} sent {len}-byte fragment of {size}-byte packet"); }
                        }
                        continue;
                    }
                    radio.send(packet).await.map_err(|e| format!("{serial} rejected {size}-byte frame: {e:?}"))?;
                }
            },
            received = radio.recv() => {
                let Some(packet) = received else { return Err("radio closed".into()) };
                let data = if fragment {
                    let Some(data) = assembly.receive(&packet.frame, capacity) else { continue };
                    if data.len() > capacity { eprintln!("{serial} reassembled {}-byte packet", data.len()); }
                    data
                } else { packet.frame };
                stream.write_all(&frame(&data)).await?;
            },
        }
    }
    radio.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tulle::direct_phy::MAX_FRAME_LEN;

    #[test]
    fn protected_packet_survives_only_complete_consistent_fragments() {
        let packet = vec![0xa5; 395];
        let parts: Vec<_> = fragments(&packet, MAX_FRAME_LEN).collect();
        assert_eq!([parts[0].len(), parts[1].len()], [255, 162]);
        let mut rx = Assembly::default();
        assert_eq!(rx.receive(&parts[1], MAX_FRAME_LEN), None);
        assert_eq!(rx.receive(&parts[0], MAX_FRAME_LEN), None);
        let mut damaged = parts[1].clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert_eq!(rx.receive(&damaged, MAX_FRAME_LEN), None);
        assert_eq!(rx.receive(&parts[0], MAX_FRAME_LEN), None);
        assert_eq!(rx.receive(&parts[1], MAX_FRAME_LEN), Some(packet));
    }
}
