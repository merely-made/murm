// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

mod board;
mod campaign;
mod carrier;
mod output;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "serve") {
        anyhow::ensure!(arguments.len() == 2, "serve requires fixture directory");
        return carrier::serve(std::path::Path::new(&arguments[1])).await;
    }
    anyhow::ensure!(
        arguments.is_empty() || (arguments.len() == 2 && arguments[0] == "--output"),
        "usage: iroh-control-probe [--output receipt.json]"
    );
    let mut output = if arguments.len() == 2 {
        Some(output::reserve(std::path::Path::new(&arguments[1]))?)
    } else {
        None
    };
    let receipt = campaign::run().await?;
    let rendered = serde_json::to_string_pretty(&receipt)?;
    if let Some(output) = &mut output {
        use std::io::Write;
        output.write_all(rendered.as_bytes())?;
    }
    println!("{rendered}");
    Ok(())
}
