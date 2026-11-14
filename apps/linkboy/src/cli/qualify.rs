//! `linkboy qualify URL --file PATH [--out DRAFT]`.

use linkboy::Error;
use linkboy::package::sha256_hex;
use linkboy::qualify::{draft, today};

use super::bad_usage;

/// Draft an entry for a vendor release. This build carries no HTTPS client yet (the client
/// choice is open in protocol_parity D5), so the person downloads the file from `URL` and
/// passes it with `--file`; Linkboy records the URL, length, sha256 and detected layout.
pub(super) fn run(mut args: impl Iterator<Item = String>) -> Result<(), Error> {
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
            _ => return Err(bad_usage("qualify accepts URL --file PATH [--out DRAFT]")),
        }
    }
    if !linkboy::package::is_download_url(&url) {
        return Err(bad_usage(
            "qualify needs an https URL ending in a plain file name",
        ));
    }
    let file = file.ok_or_else(|| Error::ToolFailed {
        tool: "linkboy",
        message: "this build has no HTTPS client (protocol_parity D5 leaves its choice open); \
                  download URL yourself and pass it with --file PATH"
            .into(),
    })?;
    let bytes = std::fs::read(&file)?;
    let entry = draft(&url, &bytes, &sha256_hex(&bytes), &today());
    println!("url: {}", entry.url);
    println!("byte_length: {}", entry.byte_length);
    println!("sha256: {}", entry.sha256);
    println!("layout: {}", entry.layout.describe());
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
