//! Receive-only channel witness: configure a Tulle direct-PHY board and print every frame it
//! hears, with arrival time and the Reticulum header decoded. Never transmits.
//!
//! ```text
//! rf_sniff PORT [SECONDS] [BANDWIDTH_HZ]
//! ```
//!
//! The profile is Postilion's trunk profile (906.875 MHz, SF8, CR 4/5, sync 0x12, 16-symbol
//! preamble), so it hears both RNode-personality boards and Retinue stations.

use std::time::{Duration, Instant};

use tulle::PhyProfile;
use tulle::airtime::AirtimeBudget;
use tulle::direct_phy_serial::{DirectPhySerialConfig, DirectPhySerialLink};

const PACKET_TYPES: [&str; 4] = ["data", "announce", "linkreq", "proof"];
const DEST_TYPES: [&str; 4] = ["single", "group", "plain", "link"];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The RNS header fields that say which exchange a frame belongs to.
fn describe(frame: &[u8]) -> String {
    let Some(&flags) = frame.first() else {
        return "empty".into();
    };
    let header_2 = flags & 0x40 != 0;
    let (dest, context) = if header_2 {
        (frame.get(18..34), frame.get(34))
    } else {
        (frame.get(2..18), frame.get(18))
    };
    format!(
        "{} {} {}dest={} ctx={} hops={}",
        PACKET_TYPES[(flags & 0x03) as usize],
        DEST_TYPES[((flags >> 2) & 0x03) as usize],
        if header_2 { "via-transport " } else { "" },
        dest.map(hex).unwrap_or_else(|| "?".into()),
        context
            .map(|c| format!("{c:#04x}"))
            .unwrap_or_else(|| "?".into()),
        frame.get(1).copied().unwrap_or_default(),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port = args
        .get(1)
        .expect("usage: rf_sniff PORT [SECONDS] [BANDWIDTH_HZ]");
    let seconds: u64 = args.get(2).map_or(600, |s| s.parse().expect("seconds"));
    let bandwidth_hz: u32 = args
        .get(3)
        .map_or(250_000, |s| s.parse().expect("bandwidth"));
    let profile = PhyProfile {
        frequency_hz: 906_875_000,
        bandwidth_hz,
        spreading_factor: 8,
        coding_rate_denominator: 5,
        preamble_symbols: 16,
        sync_word: 0x12,
        explicit_header: true,
        crc: true,
        invert_iq: false,
        tx_power_dbm: 2,
    };
    let budget = AirtimeBudget::new(60_000, 0);
    let mut radio =
        DirectPhySerialLink::open(port, profile, budget, DirectPhySerialConfig::default())
            .expect("open direct-PHY serial port");
    radio.wait_online().await.expect("direct-PHY online");
    let start = Instant::now();
    println!("sniffing {port} at {bandwidth_hz} Hz for {seconds} s");
    let deadline = start + Duration::from_secs(seconds);
    while let Ok(Some(received)) = tokio::time::timeout_at(deadline.into(), radio.recv()).await {
        println!(
            "{:>9.3}s rssi={} snr={:.1} len={} {}",
            start.elapsed().as_secs_f64(),
            received.rssi_dbm,
            received.snr_db,
            received.frame.len(),
            describe(&received.frame)
        );
    }
    radio.shutdown().await.expect("close direct-PHY link");
}
