//! Explicit public contact export. All custody stays in Djinn.
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: tucket_contact_card <public-config> <new-public-card>".into());
    }
    let output = PathBuf::from(&args[1]);
    if output.exists() {
        return Err("preserve earlier contact card".into());
    }
    let card = signalman_desktop::resident::djinn::contact_card(&PathBuf::from(&args[0]))?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&card)?)?;
    file.sync_all()?;
    println!(
        "{}",
        card["public_key"].as_str().ok_or("missing public key")?
    );
    Ok(())
}
