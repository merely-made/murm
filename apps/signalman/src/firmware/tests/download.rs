//! A qualified release is fetched and verified on a worker thread, through an injected fetcher.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use linkboy::{Fetch, FetchResponse, FirmwareCache};

use super::*;

const URL: &str = "https://release.example/2.0.0/firmware-board-2.0.0.uf2";

struct Served(Vec<u8>);

impl Fetch for Served {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error> {
        assert_eq!(url, URL);
        Ok(FetchResponse {
            final_url: url.into(),
            content_length: Some(self.0.len() as u64),
            body: Box::new(io::Cursor::new(self.0.clone())),
        })
    }
}

/// An index naming one qualified download of `bytes`, a UF2 spanning `0x26000+0x800`.
fn qualified_catalog(dir: &std::path::Path, bytes: &[u8]) -> std::path::PathBuf {
    let manifest = format!(
        r#"schema = 2
package_id = "vendor.board"
display_name = "Vendor firmware"
version = "2.0.0.abcdef0"
publisher = "Vendor"
regions = []
channel_capabilities = []
state_impact = "unknown"
license = "GPL-3.0"
notices = "External firmware."
source_revision = "abcdef0"
source_url = "https://example.invalid/source"
origin_url = "https://example.invalid/release"

[[helpers]]
route = "uf2-mass-storage"
program = "linkboy UF2 volume writer"
version = "0.0.1"
license = "MPL-2.0"
source_url = "https://example.invalid/linkboy"
notice = "Built in."

[payload]
url = "{URL}"
format = "uf2"
byte_length = {}
sha256 = "{}"
write_bytes = 2048

[[targets]]
family = "t114"
revision = "2.x"
processor = "nrf52840"
flash_size = 1048576
bootloader = "adafruit-uf2-0.9.0"
route = "uf2-mass-storage"

[[write_ranges]]
start = 0x26000
length = 0x800

[[preserved_ranges]]
start = 0xec000
length = 0x14000

[expected_application]
board = "t114"
version = "2.0.0.abcdef0"
manual_check = "Use the vendor's own interface."

[recovery]
before_write = "Enter the UF2 bootloader."
after_failure = "Re-enter the UF2 bootloader and retry."

[qualification]
hash_source = "Computed from the vendor file."
hash_recorded_on = "2026-11-14"
structure_checks = "UF2 layout check."
companion_checked = false
"#,
        bytes.len(),
        linkboy::package::sha256_hex(bytes)
    );
    std::fs::write(dir.join("vendor-board.toml"), manifest).unwrap();
    std::fs::write(
        dir.join("index.toml"),
        r#"schema = "retinue.package-index/v1"
publisher = "Merely Made"
version = "1"

[[packages]]
package_id = "vendor.board"
release_sequence = 2000000
manifest = "vendor-board.toml"
firmware_publisher = "Vendor"
state = "partial"
instructions_url = "https://example.invalid/readme"
recovery_url = "https://example.invalid/recovery"
installer_receipts = []
recovery_receipts = []
"#,
    )
    .unwrap();
    dir.join("index.toml")
}

#[test]
fn a_qualified_release_downloads_and_verifies_on_a_worker_thread() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = linkboy::encode_application(&[9; 2048], 0x26000, linkboy::NRF52840_FAMILY_ID)
        .expect("synthetic UF2");
    let index = qualified_catalog(dir.path(), &bytes);
    let cache = FirmwareCache::new(dir.path().join("cache"));

    let offline = FirmwareCatalog::load(&index)
        .unwrap()
        .with_cache(cache.clone());
    assert!(offline.needs_download("vendor.board"));
    let wake: InstallerWake = Arc::new(|| {});
    assert!(offline.start_download("vendor.board", wake).is_err());

    let catalog = offline.with_fetcher(Arc::new(Served(bytes.clone())));
    let (tx, rx) = std::sync::mpsc::channel();
    let wake: InstallerWake = Arc::new(move || {
        let _ = tx.send(());
    });
    let mut worker = catalog.start_download("vendor.board", wake).unwrap();
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the worker wakes the host");
    while !worker.finished() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let package = worker.take().unwrap().expect("verified package");
    assert_eq!(package.parts()[0].bytes(), bytes.as_slice());
    assert!(worker.take().is_none());
    assert!(!catalog.needs_download("vendor.board"));
    catalog
        .load_package("vendor.board")
        .expect("now served from the cache");
}
