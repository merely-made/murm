#![forbid(unsafe_code)]

// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use station_custody::{
    DjinnStationAuthority, SITED_STATION_CONTROL_TITLE, SitedStationControlResult,
    SitedStationCredential, SitedStationHead, SitedStationHeadError,
};
use personae::carry::DeviceId;
use personae::{IdentityProvider, SealedRecordStorage};

struct ProvisionArgs {
    app_endpoint: Option<String>,
    station_root: PathBuf,
    record: PathBuf,
    label: String,
    expires_hours: u64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let issued_at_ms = unix_time_ms()?;
    let expires_at_ms = issued_at_ms
        .checked_add(
            args.expires_hours
                .checked_mul(60 * 60 * 1_000)
                .ok_or("grant duration is too large")?,
        )
        .ok_or("grant expiry is too large")?;

    // djinn holds the wallet (dramatis repo plan, D5, D17). Absent or Locked,
    // the station cannot be commissioned and nothing is written (D12).
    let djinn = match &args.app_endpoint {
        Some(endpoint) => DjinnStationAuthority::at(endpoint.clone()),
        None => DjinnStationAuthority::default(),
    };
    djinn.status().map_err(pending)?;
    let device_id = DeviceId::new();
    let released = djinn.release_station(device_id).map_err(pending)?;

    // The bench head's record is sealed under a station-scoped key djinn
    // releases for its record path, so a later restore asks for the same.
    let storage_salt =
        personae::reticulum::station_storage_salt(args.record.to_string_lossy().as_bytes());
    let storage_key = djinn
        .release_storage(storage_salt.clone())
        .map_err(pending)?
        .derive_keypair(&storage_salt)?
        .to_seed();
    let storage = SealedRecordStorage::open_with_key(&args.station_root, storage_key);
    refuse_existing_head(storage.clone(), &args.record)?;

    let credential = SitedStationCredential::derive_for_device(&released, device_id)?;
    let grant = credential.issue_remote_auth_grant(
        &djinn,
        device_id,
        &args.label,
        issued_at_ms,
        expires_at_ms,
    )?;

    let head = credential.provision_head(storage.clone(), &args.record)?;
    let control = credential.control_signer().grant(&grant)?;
    let acknowledgement = head
        .receive_delivery(
            SITED_STATION_CONTROL_TITLE,
            &control.to_bytes()?,
            issued_at_ms,
        )?
        .ok_or("grant control did not produce an acknowledgement")?;
    let result = acknowledgement.verify(credential.public_identity(), &control)?;
    if result != &(SitedStationControlResult::GrantInstalled { expires_at_ms }) {
        return Err(format!("unexpected station acknowledgement: {result:?}").into());
    }

    let restored = SitedStationHead::restore(storage, &args.record)?;
    restored.authorize_at(issued_at_ms)?;

    println!("device_id={}", device_id.as_uuid());
    println!(
        "station_address={}",
        outrider::delivery_destination(restored.public_identity())
    );
    println!("station_identity={}", restored.public_identity().hash());
    println!("issued_at_ms={issued_at_ms}");
    println!("expires_at_ms={expires_at_ms}");
    println!("station_root={}", args.station_root.display());
    println!("station_record={}", args.record.display());
    Ok(())
}

fn pending(error: castellan_client::custody_client::CustodyClientError) -> Box<dyn Error> {
    match error.is_pending() {
        true => format!("the Signalman authority is pending: {error}").into(),
        false => error.into(),
    }
}

fn refuse_existing_head(storage: SealedRecordStorage, record: &Path) -> Result<(), Box<dyn Error>> {
    match SitedStationHead::restore(storage, record) {
        Ok(head) => Err(format!(
            "station record already exists for device {} at {}; refusing to replace it",
            head.device_id().as_uuid(),
            record.display()
        )
        .into()),
        Err(SitedStationHeadError::MissingState { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn parse_args() -> Result<ProvisionArgs, Box<dyn Error>> {
    let mut values = BTreeMap::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(flag) = arguments.next() {
        if flag == "--help" || flag == "-h" {
            print_usage();
            std::process::exit(0);
        }
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

    let app_endpoint = values.remove("--app-endpoint");
    let station_root = required_path(&mut values, "--station-root")?;
    let record = required_path(&mut values, "--record")?;
    if record.is_absolute() {
        return Err("--record must be relative to --station-root".into());
    }
    let label = required(&mut values, "--label")?;
    if label.trim().is_empty() {
        return Err("--label must not be empty".into());
    }
    let expires_hours = required(&mut values, "--expires-hours")?.parse::<u64>()?;
    if expires_hours == 0 {
        return Err("--expires-hours must be greater than zero".into());
    }
    if let Some(flag) = values.keys().next() {
        return Err(format!("unknown argument {flag}").into());
    }

    Ok(ProvisionArgs {
        app_endpoint,
        station_root,
        record,
        label,
        expires_hours,
    })
}

fn required(values: &mut BTreeMap<String, String>, name: &str) -> Result<String, Box<dyn Error>> {
    values
        .remove(name)
        .ok_or_else(|| format!("missing required argument {name}").into())
}

fn required_path(
    values: &mut BTreeMap<String, String>,
    name: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(required(values, name)?))
}

fn unix_time_ms() -> Result<u64, std::time::SystemTimeError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(u64::try_from(duration.as_millis()).expect("unix millisecond count fits u64"))
}

fn print_usage() {
    println!(
        "usage: station-provision \\\n  [--app-endpoint ENDPOINT] \\\n  --station-root PATH \\\n  --record RELATIVE_PATH \\\n  --label NAME \\\n  --expires-hours HOURS"
    );
}
