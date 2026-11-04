// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Public-only custody receipt probe. No radio access or identity fallback.
use station_custody::DjinnStationAuthority;
use personae::carry::DeviceId;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: tucket_custody_receipt <endpoint> <device-uuid> <realm> <new-public-output> <pending|unlocked|watch>".into());
    }
    let authority = DjinnStationAuthority::at(&args[0]);
    let device: DeviceId = serde_json::from_str(&format!("\"{}\"", args[1]))?;
    let path = PathBuf::from(&args[3]);
    if path.exists() {
        return Err("preserve earlier receipt".into());
    }
    let mut report = serde_json::json!({"mode": args[4], "device_id": args[1], "realm":args[2], "result":"not_completed"});
    if args[4] == "pending" {
        match authority.open_tucket(device, &args[2]) {
            Err(error) => {
                if !error.is_pending()
                    && !matches!(
                        error,
                        castellan_client::custody_client::CustodyClientError::Refused(
                            castellan_client::custody::CustodyRefusal::NoWallet
                        )
                    )
                {
                    return Err(error.into());
                }
                report["result"] = "refused".into();
                report["detail"] = error.to_string().into();
            },
            Ok(_) => {
                return Err(
                    "forbidden fallback: unavailable custody returned Tucket material".into(),
                );
            },
        }
    } else {
        let released = authority.open_tucket(device, &args[2])?;
        let public = personae::Ed25519Keypair::from_seed(*released.material.seed_bytes())
            .public_key()
            .to_bytes();
        report["public_key"] = serde_json::json!(public);
        if args[4] == "unlocked" {
            use ostiary::app_admission::AppId;
            use castellan_client::custody::{CustodyRefusal, KeySource};
            use castellan_client::custody_client::{BlockingCustodyClient, CustodyClientError};
            let salt = personae::tucket::station_salt(device.as_uuid().as_bytes(), &args[2])?;
            // Another application, granted the whole Signalman wallet
            // namespace: a station's keys still stay (the station floor).
            let broad = ostiary::declaration::AppDeclaration {
                routes: vec![ostiary::declaration::NamePattern::exact("custody")],
                custody: ostiary::declaration::CustodyUse {
                    operations: [
                        ostiary::declaration::CustodyOperation::Release,
                        ostiary::declaration::CustodyOperation::Attest,
                        ostiary::declaration::CustodyOperation::Sign,
                    ]
                    .into(),
                    salts: vec![ostiary::declaration::SaltUse::wallet(
                        ostiary::declaration::SaltPattern::Prefix("mere.signalman.".into()),
                    )],
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut other_app = BlockingCustodyClient::open_declared_at(&args[0], AppId::new("turnstone"), broad)?;
            for refused in [
                other_app
                    .release(KeySource::Wallet, vec![salt.clone()])
                    .map(|_| ()),
                other_app
                    .attest(KeySource::Wallet, salt.clone())
                    .map(|_| ()),
                other_app
                    .sign(KeySource::Wallet, salt, b"custody refusal probe".to_vec())
                    .map(|_| ()),
            ] {
                assert!(matches!(
                    refused,
                    Err(CustodyClientError::Refused(CustodyRefusal::NotReleasable))
                ));
            }
            report["wrong_app_release_sign_attest_refused"] = true.into();
            let again = authority.open_tucket(device, &args[2])?;
            let other = personae::Ed25519Keypair::from_seed(*again.material.seed_bytes())
                .public_key()
                .to_bytes();
            assert!(public == other, "reconnect changed public identity");
            released.lease.check()?;
            again.lease.check()?;
            report["result"] = "same_identity_on_reopen".into();
        } else if args[4] == "watch" {
            std::fs::write(path.with_extension("ready"), b"custody watch ready")?;
            let until = Instant::now() + Duration::from_secs(30);
            while released.lease.check_live().is_ok() {
                if Instant::now() >= until {
                    return Err("custody did not close after requested lock/loss".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(released.lease.check_live().is_err());
            assert!(released.lease.check().is_err());
            report["result"] = "custody_closed".into();
        } else {
            return Err("unknown receipt mode".into());
        }
    }
    std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
