// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

#[test]
fn real_endpoints_and_fresh_server_processes_preserve_control_authority() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-control-probe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["summary"]["failed"], 0);
    assert_eq!(receipt["summary"]["passed"], 16);
}
