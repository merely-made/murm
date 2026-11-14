//! A qualified release is fetched off the UI thread for review, and a connected radio's
//! firmware is compared with the list. No test touches the network.

#[path = "support/mod.rs"]
mod support;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use linkboy::{Fetch, FetchResponse, FirmwareCache};
use signalman::comms_adapter::comms::ProtocolKind;
use signalman::{FirmwareCatalog, FirmwareStanding, FirmwareVendor, RadioFirmware};
use signalman_desktop::state::{DesktopState, Request};

const MESHTASTIC: &str = "meshtastic.heltec-mesh-node-t114";

fn state(cache: &std::path::Path) -> DesktopState {
    let mut state = DesktopState::new(&support::synthetic_catalog());
    state.catalog = state
        .catalog
        .take()
        .map(|catalog| catalog.with_cache(FirmwareCache::new(cache)));
    state
}

/// Answers with the right length and the wrong bytes, as a replaced upstream file would.
struct Replaced;

impl Fetch for Replaced {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error> {
        Ok(FetchResponse {
            final_url: url.into(),
            content_length: Some(1_467_392),
            body: Box::new(io::Cursor::new(vec![0; 1_467_392])),
        })
    }
}

fn confirm(state: &mut DesktopState) {
    let mut worker = None;
    signalman_desktop::flow::perform(
        state,
        Request::ConfirmFirmware,
        &mut worker,
        Arc::new(|| {}),
    );
}

#[test]
fn without_a_client_the_qualified_release_is_refused_not_fetched() {
    let cache = tempfile_dir("no-client");
    let mut state = state(&cache);
    state.choose_package_id(MESHTASTIC);
    confirm(&mut state);
    assert!(state.firmware_download.is_none());
    assert!(
        state
            .refusal
            .iter()
            .any(|line| line.contains("no HTTPS client")),
        "{:?}",
        state.refusal
    );
}

#[test]
fn a_replaced_upstream_file_is_refused_after_an_off_thread_download() {
    let cache = tempfile_dir("replaced");
    let mut state = state(&cache);
    state.catalog = state
        .catalog
        .take()
        .map(|catalog: FirmwareCatalog| catalog.with_fetcher(Arc::new(Replaced)));
    state.choose_package_id(MESHTASTIC);
    confirm(&mut state);
    let note = state.firmware_download_note.clone().expect("download note");
    assert!(note.contains("1467392 bytes"), "{note}");
    assert!(note.contains("https://release.meshtastic.org/"), "{note}");
    let started = std::time::Instant::now();
    while !state.firmware_download_finished() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    state.finish_firmware_download();
    assert!(state.firmware_download.is_none());
    assert!(
        state.refusal.iter().any(|line| line.contains("pins")),
        "{:?}",
        state.refusal
    );
    assert!(state.view().review.is_none());
}

#[test]
fn a_connected_radio_stands_against_the_qualified_list() {
    let cache = tempfile_dir("standing");
    let mut state = state(&cache);
    assert_eq!(state.companion_standing(ProtocolKind::Meshtastic), None);
    let radio = |version: &str| RadioFirmware {
        vendor: FirmwareVendor::Meshtastic,
        version: version.into(),
        hw_model: Some(69),
        model: None,
    };
    state
        .companion_firmware
        .push((ProtocolKind::Meshtastic, radio("2.7.26.54e0d8d")));
    assert!(matches!(
        state.companion_standing(ProtocolKind::Meshtastic),
        Some(FirmwareStanding::Listed { newer: None, .. })
    ));
    state.companion_firmware = vec![(ProtocolKind::Meshtastic, radio("2.8.2.d5f1619"))];
    assert_eq!(
        state.companion_standing(ProtocolKind::Meshtastic),
        Some(FirmwareStanding::NotListed {
            reported: "2.8.2.d5f1619".into()
        })
    );
}

fn tempfile_dir(name: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("firmware-download-{name}"));
    let _ = std::fs::remove_dir_all(&path);
    path
}
