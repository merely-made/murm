#![forbid(unsafe_code)]
// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Explicit USB first-owner claim through an existing Signalman credential.
//!
//! This command has three explicit lifecycle actions: `init` creates a controller scope,
//! `claim` requires that existing scope, and `status` asks an already-claimed board for its
//! control status under the same controller identity. None of them opens or bootstraps a
//! wallet: the controller's scoped keys are released by djinn (dramatis repo plan, D11, D17),
//! and with djinn absent or Locked the controller is pending (D12).
//!
//! `status` keeps the controller's outer replay counter in a small record beside the scope.
//! The next counter is written atomically before the command is sent: a board may accept and
//! journal a command whose reply is then lost, and a counter this controller has already
//! spent must never be offered again.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use station_custody::{
    DjinnStationAuthority, claim_first_owner, first_owner_controller_fingerprint,
    first_owner_status,
};
use personae::RetainedKeys;
use postilion::control::first_owner::{
    ClaimOutcome, FirstOwnerController, UsbFirstOwnerConfig, UsbFirstOwnerTransport,
    v4_usb_claim_plan,
};
use postilion::control::verified::{UsbControlConfig, UsbControlTransport};
use radio_hand::control::{ControlStatusBootFact, ControlStatusEvidence, NodeId};
use radio_hand::region::Region;

enum Command {
    Init { authority_root: PathBuf },
    Claim(ClaimArgs),
    Status(StatusArgs),
}

struct StatusArgs {
    authority_root: PathBuf,
    port: String,
    node: NodeId,
}

/// The controller's outer replay counter, persisted beside the scope record.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
struct ControllerCounter {
    /// The highest counter this controller has ever offered a board under this scope.
    last_used: u64,
}

struct ClaimArgs {
    authority_root: PathBuf,
    port: String,
    region: Region,
    frequency_hz: u32,
    bandwidth_hz: u32,
    tx_power_dbm: i8,
}

fn main() -> Result<(), Box<dyn Error>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    match parse_command()? {
        Command::Init { authority_root } => initialize_controller_scope(&authority_root),
        Command::Claim(args) => claim(args).await,
        Command::Status(args) => status(args).await,
    }
}

async fn status(args: StatusArgs) -> Result<(), Box<dyn Error>> {
    let controller_scope = load_controller_scope(&args.authority_root)?;
    let provider = released_controller(controller_scope.as_bytes())?;

    // Spend the counter durably before the board can see it.
    let counter = reserve_next_counter(&args.authority_root)?;
    let transport = UsbControlTransport::open(&args.port, UsbControlConfig::default())?;
    let verified = first_owner_status(
        &provider,
        controller_scope.as_bytes(),
        transport,
        args.node,
        counter,
    )
    .await?;
    let status = verified.status;
    println!(
        "auth=verified-controller carrier=usb node={} counter={} transaction={} control={} pending={} boot={} known-good-generation={} generation-watermark={}",
        hex_text(&status.node().0),
        verified.counter,
        hex_text(&verified.transaction.0),
        evidence_name(status.control()),
        evidence_name(status.pending()),
        boot_name(status.boot()),
        status.known_good_generation().0,
        status.generation_watermark().0,
    );
    Ok(())
}

const fn evidence_name(value: ControlStatusEvidence) -> &'static str {
    match value {
        ControlStatusEvidence::Blank => "blank",
        ControlStatusEvidence::Valid => "valid",
        ControlStatusEvidence::Corrupt => "corrupt",
    }
}

const fn boot_name(value: ControlStatusBootFact) -> &'static str {
    match value {
        ControlStatusBootFact::KnownGoodApplied => "known-good-applied",
        ControlStatusBootFact::RecoveredRollback => "recovered-rollback",
    }
}

fn hex_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_node(value: &str) -> Result<NodeId, Box<dyn Error>> {
    let digits = value.as_bytes();
    if digits.len() != 32 {
        return Err(format!("node id must be 32 hex digits, got {}", digits.len()).into());
    }
    let mut node = [0_u8; 16];
    for (index, pair) in digits.chunks(2).enumerate() {
        let text = std::str::from_utf8(pair)?;
        node[index] = u8::from_str_radix(text, 16)
            .map_err(|error| format!("invalid node id hex {text:?}: {error}"))?;
    }
    Ok(NodeId(node))
}

/// Reads the counter record, advances it, and makes the advance durable before returning.
///
/// A missing record starts at one, which is the first counter a freshly claimed board's
/// zeroed grant accepts. The write goes to a sibling temporary file, is synced, and is then
/// renamed over the record, so an interruption leaves either the old record or the new one.
fn reserve_next_counter(authority_root: &Path) -> Result<u64, Box<dyn Error>> {
    let path = controller_counter_path(authority_root);
    let last_used = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let record: ControllerCounter = serde_json::from_str(&text).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "invalid first-owner controller counter at {}: {error}",
                        path.display()
                    ),
                )
            })?;
            record.last_used
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    let next = last_used
        .checked_add(1)
        .ok_or("first-owner controller counter is exhausted")?;
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec(&ControllerCounter { last_used: next })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    file.write_all(&contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, &path)?;
    Ok(next)
}

fn controller_counter_path(authority_root: &Path) -> PathBuf {
    authority_root.join("first-owner-controller-counter.json")
}

async fn claim(args: ClaimArgs) -> Result<(), Box<dyn Error>> {
    let controller_scope = load_controller_scope(&args.authority_root)?;
    let provider = released_controller(controller_scope.as_bytes())?;

    let mut phy = postilion::profile(args.bandwidth_hz);
    phy.frequency_hz = args.frequency_hz;
    phy.tx_power_dbm = args.tx_power_dbm;
    let plan = v4_usb_claim_plan(args.region, phy)?;
    let transport = UsbFirstOwnerTransport::open(&args.port, UsbFirstOwnerConfig::default())?;
    let mut controller = FirstOwnerController::new(transport);

    match claim_first_owner(
        &provider,
        controller_scope.as_bytes(),
        &mut controller,
        plan,
    )
    .await?
    {
        ClaimOutcome::Committed => println!("claim outcome=committed"),
        ClaimOutcome::CommittedCleanupPending => {
            println!("claim outcome=committed-cleanup-pending")
        },
    }
    Ok(())
}

fn initialize_controller_scope(authority_root: &Path) -> Result<(), Box<dyn Error>> {
    initialize_controller_scope_with(authority_root, released_controller)
}

/// `init`, with the release supplied: djinn's in production, a test's own.
fn initialize_controller_scope_with(
    authority_root: &Path,
    release: impl FnOnce(&[u8]) -> Result<RetainedKeys, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let scope = uuid::Uuid::new_v4();
    let provider = release(scope.as_bytes())?;
    let fingerprint = first_owner_controller_fingerprint(&provider, scope.as_bytes())?;
    let path = controller_scope_path(authority_root);
    let contents = serde_json::to_vec(&scope)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&contents)?;
    file.sync_all()?;

    println!("first-owner controller scope initialized: scope={scope} fingerprint={fingerprint}");
    Ok(())
}

/// The controller's keys for `scope`, released by djinn from the wallet.
fn released_controller(scope: &[u8]) -> Result<RetainedKeys, Box<dyn Error>> {
    DjinnStationAuthority::default()
        .release_controller(scope)
        .map_err(|error| match error.is_pending() {
            true => format!("the Signalman controller is pending: {error}").into(),
            false => error.into(),
        })
}

fn load_controller_scope(authority_root: &Path) -> Result<uuid::Uuid, Box<dyn Error>> {
    let path = controller_scope_path(authority_root);
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid existing first-owner controller scope at {}: {error}",
                path.display()
            ),
        )
        .into()
    })
}

fn controller_scope_path(authority_root: &Path) -> PathBuf {
    authority_root.join("first-owner-controller-id.json")
}

fn parse_command() -> Result<Command, Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments
        .next()
        .ok_or("missing command; use init or claim")?;
    if command == "--help" || command == "-h" {
        print_usage();
        std::process::exit(0);
    }
    let mut values = parse_values(arguments)?;
    match command.as_str() {
        "init" => {
            let authority_root = PathBuf::from(required(&mut values, "--authority-root")?);
            if let Some(flag) = values.keys().next() {
                return Err(format!("unknown argument {flag}").into());
            }
            Ok(Command::Init { authority_root })
        },
        "claim" => parse_claim_args(&mut values).map(Command::Claim),
        "status" => parse_status_args(&mut values).map(Command::Status),
        _ => Err(format!("unknown command {command:?}; use init, claim, or status").into()),
    }
}

fn parse_status_args(values: &mut BTreeMap<String, String>) -> Result<StatusArgs, Box<dyn Error>> {
    let authority_root = PathBuf::from(required(values, "--authority-root")?);
    let port = required(values, "--port")?;
    let node = parse_node(&required(values, "--node")?)?;
    if let Some(flag) = values.keys().next() {
        return Err(format!("unknown argument {flag}").into());
    }
    Ok(StatusArgs {
        authority_root,
        port,
        node,
    })
}

fn parse_values(
    arguments: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut values = BTreeMap::new();
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        if !flag.starts_with("--") {
            return Err(format!("unexpected argument {flag:?}").into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if values.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate argument {flag}").into());
        }
    }
    Ok(values)
}

fn parse_claim_args(values: &mut BTreeMap<String, String>) -> Result<ClaimArgs, Box<dyn Error>> {
    let region = parse_region(&required(values, "--region")?)?;
    let frequency_hz = required(values, "--frequency-hz")?.parse()?;
    let bandwidth_hz = required(values, "--bandwidth-hz")?.parse()?;
    let tx_power_dbm = required(values, "--tx-power-dbm")?.parse()?;
    let authority_root = PathBuf::from(required(values, "--authority-root")?);
    let port = required(values, "--port")?;
    if let Some(flag) = values.keys().next() {
        return Err(format!("unknown argument {flag}").into());
    }
    Ok(ClaimArgs {
        authority_root,
        port,
        region,
        frequency_hz,
        bandwidth_hz,
        tx_power_dbm,
    })
}

fn required(values: &mut BTreeMap<String, String>, name: &str) -> Result<String, Box<dyn Error>> {
    values
        .remove(name)
        .ok_or_else(|| format!("missing required argument {name}").into())
}

fn parse_region(value: &str) -> Result<Region, Box<dyn Error>> {
    match value {
        "us915" => Ok(Region::Us915),
        "eu868" => Ok(Region::Eu868),
        "eu433" => Ok(Region::Eu433),
        "anz915" => Ok(Region::Anz915),
        "jp920" => Ok(Region::Jp920),
        _ => Err(format!(
            "unsupported region {value:?}; use us915, eu868, eu433, anz915, or jp920"
        )
        .into()),
    }
}

fn print_usage() {
    println!(
        "usage:\n  station-claim init --authority-root PATH\n  station-claim claim --authority-root PATH --port PORT --region us915|eu868|eu433|anz915|jp920 --frequency-hz HZ --bandwidth-hz HZ --tx-power-dbm DBM\n  station-claim status --authority-root PATH --port PORT --node NODE_HEX"
    );
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn init_creates_one_scope_from_released_controller_keys() {
        let root = tempdir().unwrap();
        let wallet = personae::InMemoryProvider::from_seed([0x4a; 32]);
        let release = |scope: &[u8]| -> Result<RetainedKeys, Box<dyn Error>> {
            Ok(RetainedKeys::capture(
                &wallet,
                &station_custody::controller_salts(scope),
            )?)
        };

        initialize_controller_scope_with(root.path(), release).unwrap();

        let scope_path = controller_scope_path(root.path());
        let scope: uuid::Uuid = serde_json::from_slice(&fs::read(&scope_path).unwrap()).unwrap();
        assert_ne!(scope, uuid::Uuid::nil());

        let error = initialize_controller_scope_with(root.path(), release).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn counter_is_spent_durably_and_never_reoffered() {
        let root = tempdir().unwrap();
        assert_eq!(reserve_next_counter(root.path()).unwrap(), 1);
        assert_eq!(reserve_next_counter(root.path()).unwrap(), 2);
        let record: ControllerCounter =
            serde_json::from_slice(&fs::read(controller_counter_path(root.path())).unwrap())
                .unwrap();
        assert_eq!(record.last_used, 2);
        assert!(
            !controller_counter_path(root.path())
                .with_extension("json.tmp")
                .exists()
        );

        fs::write(controller_counter_path(root.path()), b"not json").unwrap();
        assert!(reserve_next_counter(root.path()).is_err());
    }

    #[test]
    fn node_id_parses_exactly_sixteen_bytes() {
        assert_eq!(parse_node(&"a4".repeat(16)).unwrap(), NodeId([0xa4; 16]));
        assert!(parse_node(&"a4".repeat(15)).is_err());
        assert!(parse_node(&"zz".repeat(16)).is_err());
    }

    #[test]
    fn claim_scope_load_refuses_missing_scope_without_creating_one() {
        let root = tempdir().unwrap();
        let path = controller_scope_path(root.path());

        assert!(load_controller_scope(root.path()).is_err());
        assert!(!path.exists());
    }
}
