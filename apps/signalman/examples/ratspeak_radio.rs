//! Connect a loopback RNS TCP client to one direct-PHY radio for backend qualification.
//! Usage: ratspeak_radio SERIAL_PORT TCP_PORT BW_HZ TX_DBM DTR
use retinue::iface::hdlc::{Deframer, frame};
use std::{error::Error, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tulle::{
    airtime::AirtimeBudget,
    direct_phy_serial::{DirectPhySerialConfig, DirectPhySerialLink},
};

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
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let mut radio =
        DirectPhySerialLink::open(&serial, profile, AirtimeBudget::new(60_000, 60_000), config)?;
    tokio::time::timeout(Duration::from_secs(10), radio.wait_online()).await??;
    eprintln!("{serial} online at {}", listener.local_addr()?);
    let (mut stream, _) = listener.accept().await?;
    let mut decoder = Deframer::default();
    let mut buf = [0; 1024];
    loop {
        tokio::select! {
            count = stream.read(&mut buf) => {
                let count = count?;
                if count == 0 { break; }
                for packet in decoder.push(&buf[..count]) {
                    let size = packet.len();
                    radio.send(packet).await.map_err(|e| format!("{serial} rejected {size}-byte frame: {e:?}"))?;
                }
            },
            received = radio.recv() => {
                let Some(packet) = received else { return Err("radio closed".into()) };
                stream.write_all(&frame(&packet.frame)).await?;
            },
        }
    }
    radio.shutdown().await?;
    Ok(())
}
