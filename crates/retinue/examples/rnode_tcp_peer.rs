//! Retinue's side of `oracle/interop_rnode_tcp.py`: an RNode reached over TCP.
//!
//! ```text
//! rnode_tcp_peer PORT [--tcp-port N] [--sf N] [--bw HZ]
//! ```
//!
//! `PORT` is an RNS port value (`tcp://127.0.0.1`); `--tcp-port` replaces RNS's 7633 so
//! gates can run in parallel. Runs the shared serve script, printing each pump status and
//! each telemetry change as `TELEMETRY {json}` under `get_interface_stats()`'s key names.
//! When stdin closes it shuts down.

#[allow(dead_code, reason = "the burst mode belongs to serial_peer")]
#[path = "serial_peer/script.rs"]
mod script;

use std::time::Duration;

use retinue::iface::tulle::attach_rnode;
use serde_json::{Value, json};
use tulle::airtime::AirtimeBudget;
use tulle::lora::{CodingRate, LoRaParams};
use tulle::rnode::{RNodeConfig, Telemetry};
use tulle::serial::{RNodePort, SerialPumpConfig};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let spec = args.next().ok_or("usage: rnode_tcp_peer PORT [flags]")?;
    let mut port = RNodePort::parse(&spec).ok_or("unsupported port")?;
    let mut params = LoRaParams {
        spreading_factor: 7,
        bandwidth_hz: 500_000,
        coding_rate: CodingRate::Cr45,
        frequency_hz: 867_200_000,
        tx_power_dbm: 14,
        preamble_syms: 8,
        explicit_header: true,
        crc: true,
    };
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        let number: u32 = value.parse().map_err(|e| format!("{e}"))?;
        match (flag.as_str(), &mut port) {
            ("--tcp-port", RNodePort::Tcp { port, .. }) => *port = number as u16,
            ("--sf", _) => params.spreading_factor = number as u8,
            ("--bw", _) => params.bandwidth_hz = number,
            (other, _) => return Err(format!("unexpected flag {other}")),
        }
    }

    let endpoint = script::endpoint();
    let budget = AirtimeBudget::new(60_000, 60_000);
    let pump = SerialPumpConfig::tcp();
    let carrier = attach_rnode(
        &endpoint,
        &port,
        RNodeConfig::new(params),
        budget,
        pump,
        None,
        None,
    )
    .map_err(|e| format!("attach: {e}"))?;
    let (mut status, mut telemetry) = (carrier.status, carrier.telemetry);
    tokio::spawn(async move {
        loop {
            println!("STATUS {:?}", status.borrow_and_update().clone());
            if status.changed().await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while telemetry.changed().await.is_ok() {
            println!("TELEMETRY {}", stats(&telemetry.borrow_and_update()));
        }
    });
    let driver = tokio::spawn(carrier.run);
    script::serve(endpoint.clone(), None).await;
    endpoint.shutdown(Duration::from_secs(2)).await;
    if let Ok(Err(error)) = driver.await {
        println!("DRIVE_ERR {error}");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("DONE");
    Ok(())
}

/// The telemetry as RNS's `get_interface_stats()` names it (`Reticulum.py` 1538-1567), with
/// PHYPRM and CSMA under the interface's own `r_*` names.
fn stats(t: &Telemetry) -> Value {
    let mut out = json!({ "bitrate": t.bitrate, "cpu_temp": t.temperature_c });
    if let Some(c) = t.channel {
        out["airtime_short"] = json!(c.airtime_short);
        out["airtime_long"] = json!(c.airtime_long);
        out["channel_load_short"] = json!(c.channel_load_short);
        out["channel_load_long"] = json!(c.channel_load_long);
        out["noise_floor"] = json!(c.noise_floor_dbm);
        out["interference"] = json!(c.interference_dbm);
        out["r_current_rssi"] = json!(c.current_rssi_dbm);
    }
    if let Some(b) = t.battery {
        if b.state != 0 {
            out["battery_state"] = json!(b.state_name());
        }
        out["battery_percent"] = json!(b.percent);
    }
    if let Some(p) = t.phy {
        out["r_symbol_time_ms"] = json!(p.symbol_time_ms);
        out["r_symbol_rate"] = json!(p.symbol_rate);
        out["r_preamble_symbols"] = json!(p.preamble_symbols);
        out["r_premable_time_ms"] = json!(p.preamble_ms);
        out["r_csma_slot_time_ms"] = json!(p.csma_slot_ms);
        out["r_csma_difs_ms"] = json!(p.csma_difs_ms);
    }
    if let Some(c) = t.csma {
        out["r_csma_cw_band"] = json!(c.band);
        out["r_csma_cw_min"] = json!(c.cw_min);
        out["r_csma_cw_max"] = json!(c.cw_max);
    }
    out
}
