//! Explicit public-only Sennet commissioning through Djinn. No radio access.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || !matches!(args[2].as_str(), "inspect" | "initialize-packet-ids") {
        return Err("usage: sennet_station <public-config> <new-public-output> <inspect|initialize-packet-ids>".into());
    }
    if std::path::Path::new(&args[1]).exists() {
        return Err("preserve earlier station receipt".into());
    }
    let value = signalman_desktop::resident::sennet_station::station_card(
        std::path::Path::new(&args[0]),
        args[2] == "initialize-packet-ids",
    )?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    file.write_all(&serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
