//! Run a separately installed Ratspeak backend with explicit operator-owned profiles.
use std::{error::Error, ffi::OsString, process::Command};

pub fn run(mut args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn Error>> {
    let usage = "signalman ratspeak PROGRAM LXMF_DIR RNS_DIR relay | receive | send DEST BODY_FILE";
    let program = args.next().ok_or(usage)?;
    let lxmf = args.next().ok_or(usage)?;
    let rns = args.next().ok_or(usage)?;
    let mut command = Command::new(program);
    command
        .arg("--config")
        .arg(lxmf)
        .arg("--rnsconfig")
        .arg(rns);
    match args.next().as_deref().and_then(|s| s.to_str()) {
        Some("receive") => {} // lxmd retrieves from the configured outbound node while running.
        Some("relay") => {
            command.arg("--propagation-node");
        }
        Some("send") => {
            let dest = args.next().ok_or(usage)?;
            let addr = dest.to_str().ok_or(usage)?;
            if addr.len() != 32 || !addr.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("expected a 16-byte LXMF destination in hex".into());
            }
            command
                .arg("--send")
                .arg(dest)
                .arg("--send-file")
                .arg(args.next().ok_or(usage)?)
                .args(["--send-method", "propagated"]);
        }
        _ => return Err(usage.into()),
    }
    if args.next().is_some() {
        return Err(usage.into());
    }
    let status = command.status()?;
    if !status.success() {
        return Err(format!("Ratspeak backend exited with {status}").into());
    }
    Ok(())
}
