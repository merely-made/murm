//! `station-claim lifecycle`: apply a candidate built from arguments, read Status as evidence,
//! then commit, revert, or wait out the board's deadline, all in this one invocation (D17,
//! D18). The commit token never leaves this process's memory; the run record carries no
//! secret.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use postilion::control::verified::{UsbControlConfig, UsbControlTransport};
use radio_hand::control::{
    ManagementCarrierSet, NodeId, PublicConfigurationV1, ReticulumTransportPolicy,
};
use station_custody::lifecycle::{
    FileControllerState, InMemoryTokenHolder, LifecyclePlan, Then, run_lifecycle,
};

pub const USAGE: &str = "  station-claim lifecycle --authority-root PATH --port PORT --node NODE_HEX --region REGION --frequency-hz HZ --bandwidth-hz HZ --spreading-factor 5..12 --coding-rate 5..8 --preamble-symbols N --sync-word 0xNN --tx-power-dbm DBM --relay-announces on|off --relay-packets on|off --max-hops N --lifetime-ms MS --then commit|revert|expire --record PATH [--expiry-margin-ms MS]";

pub struct LifecycleArgs {
    authority_root: PathBuf,
    port: String,
    record: PathBuf,
    plan: LifecyclePlan,
}

fn on_off(value: &str, name: &str) -> Result<bool, Box<dyn Error>> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{name} takes on or off, got {value:?}").into()),
    }
}

fn sync_word(value: &str) -> Result<u8, Box<dyn Error>> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    u8::from_str_radix(digits, 16)
        .map_err(|error| format!("sync word {value:?} is not one hex byte: {error}").into())
}

pub fn parse(values: &mut BTreeMap<String, String>) -> Result<LifecycleArgs, Box<dyn Error>> {
    let mut take = |name: &str| super::required(values, name);
    let authority_root = PathBuf::from(take("--authority-root")?);
    let port = take("--port")?;
    let node: NodeId = super::parse_node(&take("--node")?)?;
    let region = super::parse_region(&take("--region")?)?;
    // Explicit header, CRC on, IQ not inverted (postilion's profile); every other radio field
    // is an argument.
    let frequency_hz: u32 = take("--frequency-hz")?.parse()?;
    let mut phy = postilion::profile(take("--bandwidth-hz")?.parse()?);
    phy.frequency_hz = frequency_hz;
    phy.spreading_factor = take("--spreading-factor")?.parse()?;
    phy.coding_rate_denominator = take("--coding-rate")?.parse()?;
    phy.preamble_symbols = take("--preamble-symbols")?.parse()?;
    phy.sync_word = sync_word(&take("--sync-word")?)?;
    phy.tx_power_dbm = take("--tx-power-dbm")?.parse()?;
    let relay_announces = on_off(&take("--relay-announces")?, "--relay-announces")?;
    let relay_packets = on_off(&take("--relay-packets")?, "--relay-packets")?;
    let max_hops: u8 = take("--max-hops")?.parse()?;
    let lifetime_ms: u64 = take("--lifetime-ms")?.parse()?;
    let then = match take("--then")?.as_str() {
        "commit" => Then::Commit,
        "revert" => Then::Revert,
        "expire" => Then::Expire,
        other => {
            return Err(format!("--then takes commit, revert or expire, got {other:?}").into());
        }
    };
    let record = PathBuf::from(take("--record")?);
    let margin_ms: u64 = match values.remove("--expiry-margin-ms") {
        Some(value) => value.parse()?,
        None => 3_000,
    };
    if let Some(flag) = values.keys().next() {
        return Err(format!("unknown argument {flag}").into());
    }
    let transport = ReticulumTransportPolicy::new(relay_announces, relay_packets, max_hops)
        .map_err(|error| format!("relay flags and max hops: {error:?}"))?;
    // The V4 accepts only its literal USB management carrier.
    let carriers = ManagementCarrierSet::from_mask(1).map_err(|error| format!("{error:?}"))?;
    let candidate = PublicConfigurationV1::new(region, phy, transport, carriers)
        .map_err(|error| format!("candidate configuration: {error:?}"))?;
    Ok(LifecycleArgs {
        authority_root,
        port,
        record,
        plan: LifecyclePlan {
            node,
            candidate,
            lifetime_ms,
            then,
            expiry_margin: Duration::from_millis(margin_ms),
        },
    })
}

pub async fn run(args: LifecycleArgs) -> Result<(), Box<dyn Error>> {
    // Refuse to overwrite an earlier record before anything touches the board.
    let mut record_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.record)?;
    let controller_scope = super::load_controller_scope(&args.authority_root)?;
    let provider = super::released_controller(controller_scope.as_bytes())?;
    let transport = UsbControlTransport::open(&args.port, UsbControlConfig::default())?;
    let mut state = FileControllerState::new(&args.authority_root);
    let mut holder = InMemoryTokenHolder::new();
    let outcome = run_lifecycle(
        &provider,
        controller_scope.as_bytes(),
        transport,
        &args.plan,
        &mut state,
        &mut holder,
        &mut |line| println!("{line}"),
    )
    .await;
    let (record, result) = match outcome {
        Ok(record) => (record, Ok(())),
        Err(failure) => {
            let message = failure.to_string();
            (*failure.record, Err(message))
        }
    };
    record_file.write_all(&serde_json::to_vec_pretty(&record)?)?;
    record_file.sync_all()?;
    println!("record={}", args.record.display());
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut map: BTreeMap<String, String> = [
            ("--authority-root", "root"),
            ("--port", "PORT"),
            ("--node", "a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4"),
            ("--region", "us915"),
            ("--frequency-hz", "904375000"),
            ("--bandwidth-hz", "125000"),
            ("--spreading-factor", "9"),
            ("--coding-rate", "6"),
            ("--preamble-symbols", "12"),
            ("--sync-word", "0x12"),
            ("--tx-power-dbm", "14"),
            ("--relay-announces", "on"),
            ("--relay-packets", "on"),
            ("--max-hops", "8"),
            ("--lifetime-ms", "60000"),
            ("--then", "commit"),
            ("--record", "run.json"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        for (key, value) in extra {
            map.insert(key.to_string(), value.to_string());
        }
        map
    }

    #[test]
    fn every_radio_field_and_both_relay_flags_come_from_arguments() {
        let args = parse(&mut values(&[])).unwrap();
        let phy = args.plan.candidate.requested_reticulum_phy();
        assert_eq!(
            (phy.frequency_hz, phy.bandwidth_hz, phy.spreading_factor),
            (904_375_000, 125_000, 9)
        );
        assert_eq!((phy.coding_rate_denominator, phy.preamble_symbols), (6, 12));
        assert_eq!((phy.sync_word, phy.tx_power_dbm), (0x12, 14));
        let transport = args.plan.candidate.reticulum_transport();
        assert!(transport.relay_announces && transport.relay_packets);
        assert_eq!(transport.max_hops, 8);
        assert_eq!(args.plan.then, Then::Commit);
    }

    #[test]
    fn inconsistent_or_unknown_arguments_are_refused() {
        assert!(
            parse(&mut values(&[
                ("--relay-announces", "off"),
                ("--relay-packets", "off")
            ]))
            .is_err()
        );
        assert!(parse(&mut values(&[("--then", "later")])).is_err());
        assert!(parse(&mut values(&[("--sync-word", "0x123")])).is_err());
        assert!(parse(&mut values(&[("--extra", "1")])).is_err());
    }
}
