//! A verified catalog for tests that need one.
//!
//! The checkout holds package templates, not built firmware (protocol_parity
//! D4). Tests stage the repository's own index with small synthetic images
//! through Linkboy's staging code, so the flow sees a catalog that passed the
//! same verification an installed stage passes, with no firmware toolchain.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The staged `index.toml`, staged once per test binary under Cargo's
/// temporary directory for integration tests.
pub fn synthetic_catalog() -> PathBuf {
    static INDEX: OnceLock<PathBuf> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            let index = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../firmware/packages/index.toml");
            let uf2 = linkboy::encode_application(&[3; 2048], 0x26000, linkboy::NRF52840_FAMILY_ID)
                .expect("synthetic UF2");
            let payloads = [
                ("retinue.heltec-v4", vec![0x5a; 8192], Some(4096)),
                ("retinue.t114", uf2, None),
            ]
            .into_iter()
            .map(|(id, bytes, write_bytes)| {
                (
                    id.to_string(),
                    linkboy::StagedPayload { bytes, write_bytes },
                )
            })
            .collect();
            let stage = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join(concat!("synthetic-catalog-", env!("CARGO_CRATE_NAME")));
            let _ = std::fs::remove_dir_all(&stage);
            let packages = stage.join("firmware/packages");
            linkboy::stage_catalog(&index, &packages, &"ab".repeat(20), payloads)
                .expect("stage verifies");
            packages.join("index.toml")
        })
        .clone()
}
