//! Retinue's side of `oracle/interop_ax25_kiss.py`: an AX.25 KISS carrier on a tty.
//!
//! ```text
//! ax25_peer PORT --callsign CALL --ssid N [--flow-control] [--publish LEN]
//! ```
//!
//! Runs the shared serve script and exits when stdin closes.

#[allow(dead_code, reason = "the burst mode belongs to serial_peer")]
#[path = "serial_peer/script.rs"]
mod script;

use std::time::Duration;

use retinue::iface::ax25::Ax25Header;
use retinue::iface::kiss::TncConfig;
use retinue::iface::serial::{self, Framing, SerialConfig};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let port = args.next().ok_or("usage: ax25_peer PORT [flags]")?;
    let mut config = SerialConfig::new(port);
    config.speed = 115_200;
    let (mut callsign, mut ssid, mut tnc, mut publish) =
        (String::new(), 0, TncConfig::default(), None);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--callsign" => callsign = value()?,
            "--ssid" => ssid = value()?.parse().map_err(|e| format!("{e}"))?,
            "--flow-control" => tnc.flow_control = true,
            "--publish" => publish = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let header = Ax25Header::new(&callsign, ssid).map_err(|e| e.to_string())?;
    let endpoint = script::endpoint();
    let carrier = serial::attach(&endpoint, config, Framing::ax25(header, tnc), None)
        .map_err(|e| format!("attach: {e}"))?;
    println!("ATTACHED {} HEADER {}", carrier.id, hex(header.bytes()));
    let mut status = carrier.status;
    tokio::spawn(async move {
        loop {
            println!("STATUS {:?}", status.borrow_and_update().clone());
            if status.changed().await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(carrier.run);
    script::serve(endpoint.clone(), publish).await;
    endpoint.shutdown(Duration::from_secs(2)).await;
    println!("DONE");
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
