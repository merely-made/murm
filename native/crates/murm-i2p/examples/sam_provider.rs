// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! A bounded qualification host for the existing embedded provider. No Retinue
//! endpoint or application identity is created. Send a line on stdin to stop.
use murm_i2p::{EmbeddedRouter, RouterConfig, Transit};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let storage = std::env::var_os("MURM_RELAY_PROVIDER_STORAGE")
        .ok_or_else(|| std::io::Error::other("set MURM_RELAY_PROVIDER_STORAGE"))?;
    let router = tokio::time::timeout(
        Duration::from_secs(180),
        EmbeddedRouter::start(RouterConfig::new(storage, Transit::Off, false)),
    )
    .await
    .map_err(|_| std::io::Error::other("provider startup expired"))??;
    println!("MURM_POOL_SAM {}", router.bridge().addr());
    println!(
        "MURM_POOL_BOOTSTRAP stored={} reseeded={:?}",
        router.report().stored_routers,
        router.report().reseeded
    );
    let stop_file = std::env::var_os("MURM_RELAY_PROVIDER_STOP_FILE");
    let stopped = async {
        if let Some(path) = stop_file {
            let path = std::path::PathBuf::from(path);
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        } else {
            let mut input = tokio::io::BufReader::new(tokio::io::stdin());
            let mut stop = String::new();
            let _ = input.read_line(&mut stop).await;
        }
    };
    tokio::select! {
        _ = stopped => {},
        _ = tokio::time::sleep(Duration::from_secs(3600)) => {},
        end = router.stopped() => return Err(std::io::Error::other(format!("provider terminated: {end:?}"))),
    }
    let end = router.shutdown().await;
    println!("MURM_POOL_PROVIDER_END {end:?}");
    Ok(())
}
