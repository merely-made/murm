//! `linkboy qualify URL [--file PATH] [--out DRAFT]`.

use linkboy::package::sha256_hex;
use linkboy::qualify::{draft, today};
use linkboy::{Error, FirmwareCache};

use super::bad_usage;

/// Draft an entry for a vendor release: fetch `URL` once (or read `--file`, a copy the person
/// downloaded), record the URL, length, sha256 and detected layout, and keep the bytes in the
/// cache so the finished entry can be inspected and flashed without a second download.
pub(super) fn run(
    mut args: impl Iterator<Item = String>,
    cache: &FirmwareCache,
) -> Result<(), Error> {
    let url = args
        .next()
        .ok_or_else(|| bad_usage("qualify needs a URL"))?;
    let (mut file, mut out) = (None, None);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| bad_usage(&format!("{flag} needs a PATH")))?;
        match flag.as_str() {
            "--file" => file = Some(value),
            "--out" => out = Some(value),
            _ => return Err(bad_usage("qualify accepts URL [--file PATH] [--out DRAFT]")),
        }
    }
    if !linkboy::package::is_download_url(&url) {
        return Err(bad_usage(
            "qualify needs an https URL ending in a plain file name",
        ));
    }
    // Without --file this is the one network step a person runs by hand: one capped GET.
    let bytes = match file {
        Some(path) => std::fs::read(path)?,
        None => {
            linkboy::download::fetch_capped(
                &linkboy::download::HttpsFetcher::new(),
                &url,
                linkboy::qualify::QUALIFY_LIMIT,
            )?
            .bytes
        }
    };
    let entry = draft(&url, &bytes, &sha256_hex(&bytes), &today());
    println!("url: {}", entry.url);
    println!("byte_length: {}", entry.byte_length);
    println!("sha256: {}", entry.sha256);
    println!("layout: {}", entry.layout.describe());
    println!("cached: {}", cache.insert(&url, &bytes)?.display());
    match out {
        Some(path) => {
            use std::io::Write as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?
                .write_all(entry.toml.as_bytes())?;
            println!("draft written to {path}");
        }
        None => print!("{}", entry.toml),
    }
    Ok(())
}
