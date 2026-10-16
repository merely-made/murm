//! Fresh private material for the explicitly isolated desktop Tucket bench.
//! Prints only its public key, and never opens a radio or replaces a file.
use std::{fs::OpenOptions, io::Write, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let export = args.first().is_some_and(|v| v == "--public-card");
    if export {
        args.remove(0);
    }
    if args.len() != 2 {
        return Err(
            "usage: tucket_bench_identity [--public-card] <absolute-private-file> <new-public-receipt>".into(),
        );
    }
    let private = PathBuf::from(&args[0]);
    let receipt = PathBuf::from(&args[1]);
    if !private.is_absolute()
        || (!export && private.exists())
        || receipt.exists()
        || private == receipt
    {
        return Err("require a new absolute private file and a distinct new receipt".into());
    }
    let mut material = zeroize::Zeroizing::new([0u8; 116]);
    if export {
        let bytes = zeroize::Zeroizing::new(std::fs::read(&private)?);
        if bytes.len() != 116 {
            return Err("invalid isolated bench material length".into());
        }
        material.copy_from_slice(&bytes);
    } else {
        getrandom::fill(material.as_mut())
            .map_err(|e| format!("OS randomness unavailable: {e}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(private)?;
        file.write_all(material.as_ref())?;
        file.sync_all()?;
    }
    let mut node = tucket::node::Node::new(
        tucket::identity::LocalIdentity::from_seed(material[64..96].try_into().unwrap()),
        false,
    );
    let public = node.identity().pub_key;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?;
    let card = node
        .advert_frame_data(now, &tucket::advert::AdvertData::chat("Tucket bench"))
        .ok_or("invalid bench advert")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt)?;
    let public = hex::encode(public);
    serde_json::to_writer_pretty(
        &mut output,
        &serde_json::json!({"schema":1,"bench_only":true,"public_key":public,"advert_hex":hex::encode(card)}),
    )?;
    output.sync_all()?;
    println!("{public}");
    Ok(())
}
