//! Attended standalone-firmware probe. Public config, isolated Djinn bench wallet.
//! No private material is written. Firmware setup remains volatile across reboot.
use station_custody::DjinnStationAuthority;
use radio_hand::{
    resident_command::Command,
    resident_wire::{ResidentSetup, SennetKey},
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroize;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
struct PublicConfig {
    device_id: pandect::DeviceId,
    realm: String,
    port: PathBuf,
    #[serde(default, rename = "dtr")]
    usb_dtr: bool,
}
fn save(path: &Path, value: &serde_json::Value) -> Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
async fn collect(port: &mut serial2_tokio::SerialPort, duration: Duration) -> Result<String> {
    let until = tokio::time::Instant::now() + duration;
    let mut data = Vec::new();
    let mut bytes = [0; 1024];
    while tokio::time::Instant::now() < until {
        match tokio::time::timeout_at(until, port.read(&mut bytes)).await {
            Ok(Ok(0)) => return Err("firmware port closed".into()),
            Ok(Ok(count)) => data.extend_from_slice(&bytes[..count]),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        }
        if data.len() > 65536 {
            return Err("firmware transcript bound".into());
        }
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}
async fn command(port: &mut serial2_tokio::SerialPort, value: Command) -> Result<()> {
    let wire = value.encode().ok_or("invalid firmware command")?;
    port.write_all(&wire).await?;
    port.flush().await?;
    Ok(())
}
async fn run(args: &[String]) -> Result<()> {
    if args.len() != 7 {
        return Err("usage: tucket_firmware_contacts <public-config> <fresh-output> <peer-full-key> <signalman-ui-tag> <expected-cache-count> <min-stack-bytes> <min-free-heap-percent>".into());
    }
    let config_path = Path::new(&args[0]);
    let output = Path::new(&args[1]);
    if output.exists() {
        return Err("preserve previous trial".into());
    }
    let settings = signalman_desktop::resident::djinn::load(config_path)?;
    let config: PublicConfig = serde_json::from_slice(&std::fs::read(config_path)?)?;
    if !config.realm.starts_with("bench-") || !args[3].starts_with("signalman-ui-") {
        return Err("explicit isolated bench scope required".into());
    }
    let peer: [u8; 32] = hex::decode(&args[2])?
        .try_into()
        .map_err(|_| "full peer key required")?;
    let expected: usize = args[4].parse()?;
    let min_stack: usize = args[5].parse()?;
    let min_heap: usize = args[6].parse()?;
    if min_heap > 100 {
        return Err("invalid heap acceptance percentage".into());
    }
    let released = DjinnStationAuthority::default().open_tucket(config.device_id, &config.realm)?;
    let public = tucket::identity::LocalIdentity::from_seed(*released.material.seed_bytes())
        .identity()
        .pub_key;
    let p2 = settings.profile;
    let mut p0 = selvage::PhyProfile::meshcore(p2.frequency_hz, 250000, 11, 5);
    p0.tx_power_dbm = 0;
    let mut p1 = selvage::PhyProfile::meshtastic_long_fast(p2.frequency_hz);
    p1.tx_power_dbm = 0;
    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|e| format!("OS randomness unavailable: {e}"))?;
    let mut setup = ResidentSetup {
        sennet_source: 0x31415926,
        sennet_channel: 0,
        sennet_key: SennetKey::Aes256(random),
        tucket_identity_seed: *released.material.seed_bytes(),
        retinue_name_hash: [0x29; 10],
        profiles: [p0, p1, p2],
        home: 2,
        pin: None,
        require_coverage: false,
        max_excursion_ms: 10000,
        return_budget_ms: 1500,
        max_defer_ms: 250,
        transition_timeout_ms: 2000,
        tx_budget_ms: 2000,
        frame_ttl_ms: 15000,
        packet_lease_count: 16,
    };
    std::fs::create_dir(output)?;
    let mut port = signalman::radio_comms::companion::open_port(&config.port).await?;
    port.set_rts(false)?;
    port.set_dtr(config.usb_dtr)?;
    // Select already-permitted ordinary service on an uncommissioned board
    // before the volatile setup frame. This does not mint owner authority.
    port.write_all(b"status\n").await?;
    port.flush().await?;
    let mut transcript = vec![collect(&mut port, Duration::from_millis(800)).await?];
    let mut wire = setup.encode();
    released.lease.check()?;
    port.write_all(&wire).await?;
    port.flush().await?;
    wire.as_mut_slice().zeroize();
    setup.tucket_identity_seed.zeroize();
    random.zeroize();
    // The V4's diagnostic carrier retires on an undrained reply. Keep a read
    // outstanding while setup finishes rather than sleeping through readiness.
    transcript.push(collect(&mut port, Duration::from_secs(8)).await?);
    command(&mut port, Command::Status).await?;
    let status = collect(&mut port, Duration::from_millis(900)).await?;
    if !status.contains("state=Home")
        || !status.contains("instance: PersonalityId(2)")
        || !status.contains(&format!(
            "resident contacts persisted={expected} installed={expected}"
        ))
    {
        save(
            &output.join("failed-status.json"),
            &serde_json::json!({"status":status,"before_setup":transcript}),
        )?;
        return Err("firmware contact scope/activation did not match before peer work".into());
    }
    transcript.push(status.clone());
    save(
        &output.join("configured.json"),
        &serde_json::json!({"public_key":hex::encode(public),"cached_before_peer_work":expected,"status":status}),
    )?;
    let until = Instant::now() + Duration::from_secs(30);
    while !output.join("ready").exists() {
        if Instant::now() >= until {
            return Err("attended peer readiness deadline".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    released.lease.check()?;
    // Allow the initial verified advert's quiet save before the first send.
    transcript.push(collect(&mut port, Duration::from_millis(1500)).await?);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?;
    command(
        &mut port,
        Command::TucketText {
            to: peer[0],
            timestamp,
            ttl_ms: 15000,
            attempts: 3,
            flood_last: true,
            text: args[3].as_str().try_into()?,
        },
    )
    .await?;
    transcript.push(collect(&mut port, Duration::from_secs(16)).await?);
    command(&mut port, Command::Status).await?;
    let final_status = collect(&mut port, Duration::from_millis(900)).await?;
    let number = |field: &str| -> Result<usize> {
        Ok(final_status
            .split_whitespace()
            .find_map(|word| word.strip_prefix(field))
            .ok_or_else(|| format!("missing firmware measurement {field}"))?
            .parse()?)
    };
    let stack_free = number("free=")?;
    let heap_capacity = number("heap_capacity=")?;
    let heap_peak = number("heap_allocator_used_peak=")?;
    let reserves = stack_free >= min_stack
        && heap_peak <= heap_capacity
        && (heap_capacity - heap_peak) * 100 >= heap_capacity * min_heap;
    transcript.push(final_status);
    released.lease.check()?;
    let combined = transcript.join("\n");
    let matched = combined.contains("TucketAcknowledged");
    let received = combined.contains(&format!("{}-return", args[3]));
    save(
        &output.join("receipt.json"),
        &serde_json::json!({"passed":matched&&received&&reserves,"authority":"isolated Djinn wallet, volatile USB firmware setup",
        "public_key":hex::encode(public),"peer_key":hex::encode(peer),"expected_cache_before_peer_work":expected,
        "protocol_acknowledged":matched,"received_exact_return_text":received,
        "memory":{"stack_free":stack_free,"heap_capacity":heap_capacity,"heap_peak":heap_peak,
        "min_stack_bytes":min_stack,"min_free_heap_percent":min_heap,"passed":reserves},"transcript":transcript}),
    )?;
    if !matched || !received || !reserves {
        return Err("standalone firmware text/ACK exchange failed".into());
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run(&args))
}
