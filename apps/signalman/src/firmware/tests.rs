mod download;

use linkboy::OwnerStage;

use super::*;

#[test]
fn installer_starts_with_the_first_owner_page() {
    let installer = FirmwareInstaller::new();
    let view = installer.view();
    assert_eq!(view.stage, OwnerStage::ChooseDevice);
    assert_eq!(view.title, "Choose device");
    assert_eq!(view.device, None);
    assert_eq!(view.review, None);
    assert_eq!(view.result, None);
}

/// The repository's index staged with synthetic images: the checkout holds templates, not
/// built firmware, so the catalog under test is staged the way a release stage is.
fn synthetic_catalog() -> (tempfile::TempDir, std::path::PathBuf) {
    let index = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    let stage = tempfile::tempdir().expect("stage directory");
    let packages = stage.path().join("firmware/packages");
    linkboy::stage_catalog(&index, &packages, &"ab".repeat(20), payloads).expect("stage verifies");
    (stage, packages.join("index.toml"))
}

#[test]
fn catalog_loads_and_resolves_verified_packages() {
    let (_stage, index_path) = synthetic_catalog();
    let catalog = FirmwareCatalog::load(index_path).expect("package catalog should load");
    assert!(
        catalog.packages().len() >= 2,
        "the Retinue V4 and T114 packages remain present alongside any added packages"
    );
    assert_eq!(
        catalog
            .package("retinue.heltec-v4")
            .expect("V4 should be catalogued")
            .state,
        linkboy::CatalogState::ProvenRecipe
    );
    let package = catalog
        .load_package("retinue.t114")
        .expect("T114 manifest should resolve");
    assert_eq!(package.manifest().package_id, "retinue.t114");
}

#[test]
fn an_owner_confirmed_t114_dfu_port_keeps_loader_evidence_and_transport_state() {
    let snapshot = linkboy::T114LoaderSnapshot {
        schema: linkboy::discovery::T114_LOADER_SNAPSHOT_SCHEMA,
        model: "HT-n5262".into(),
        uf2_bootloader: "0.9.0".into(),
        softdevice: "S140 6.1.1".into(),
        processor: linkboy::ProcessorKind::Nrf52840,
        flash_size: 1024 * 1024,
    };

    let observation = observe_t114_serial_dfu_port("COM10", "2.x".into(), &snapshot);

    assert_eq!(
        observation.transport,
        linkboy::DeviceTransport::SerialDfuPort("COM10".into())
    );
    assert_eq!(
        observation.firmware,
        linkboy::device::FirmwareState::Bootloader
    );
    assert_eq!(
        observation.hardware.loader_route.as_deref(),
        Some("captured-t114-loader-snapshot")
    );
    assert_eq!(
        observation
            .selected_board
            .as_ref()
            .map(|board| &board.family),
        Some(&linkboy::BoardFamily::T114)
    );
}

#[test]
fn a_radio_on_the_listed_meshtastic_release_stands_listed_and_others_do_not() {
    let (_stage, index_path) = synthetic_catalog();
    let catalog = FirmwareCatalog::load(index_path).expect("package catalog should load");
    let radio = |version: &str| RadioFirmware {
        vendor: FirmwareVendor::Meshtastic,
        version: version.into(),
        hw_model: Some(69),
        model: None,
    };
    match catalog.standing(Some(&radio("2.7.26.54e0d8d")), None) {
        FirmwareStanding::Listed { current, newer } => {
            assert_eq!(current.package_id, "meshtastic.heltec-mesh-node-t114");
            assert_eq!(current.publisher, "Meshtastic");
            assert_eq!(current.byte_length, 1_467_392);
            assert_eq!(newer, None);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        catalog.standing(Some(&radio("2.8.2.d5f1619")), None),
        FirmwareStanding::NotListed {
            reported: "2.8.2.d5f1619".into()
        }
    );
    let unknown_board = RadioFirmware {
        hw_model: None,
        ..radio("2.7.26.54e0d8d")
    };
    assert!(matches!(
        catalog.standing(Some(&unknown_board), None),
        FirmwareStanding::Unknown { .. }
    ));
    assert!(matches!(
        catalog.standing(Some(&unknown_board), Some(linkboy::BoardFamily::T114)),
        FirmwareStanding::Listed { .. }
    ));
    assert!(matches!(
        catalog.standing(None, None),
        FirmwareStanding::Unknown { .. }
    ));
}

#[test]
fn the_same_selection_names_our_newest_staged_image() {
    let (_stage, index_path) = synthetic_catalog();
    let catalog = FirmwareCatalog::load(index_path).expect("package catalog should load");
    for (board, package_id) in [
        (linkboy::BoardFamily::T114, "retinue.t114"),
        (linkboy::BoardFamily::HeltecV4, "retinue.heltec-v4"),
    ] {
        let newest = linkboy::catalog::newest::newest(catalog.listings(), "Merely Made", &board)
            .expect("staged versions order")
            .expect("a staged image is listed");
        assert_eq!(newest.entry.package_id, package_id);
    }
}
