//! A vendor's merged ESP32-S3 image at 0, with additional images at manifest offsets.

use super::*;

const SIXTEEN_MB: u32 = 16 * 1024 * 1024;

/// An ESP image header magic followed by filler, the shape a merged image starts with.
fn image(length: usize, fill: u8) -> Vec<u8> {
    let mut bytes = vec![fill; length];
    bytes[0] = 0xE9;
    bytes
}

/// Meshtastic's 16 MB clean-install shape: merged image, OTA at 0x650000, littlefs at 0xc90000.
fn merged_manifest(parts: &[(&str, FirmwarePartKind, u32, &[u8])]) -> FlashPackageManifest {
    let mut value = sparse_manifest(parts);
    value.publisher_signature = None;
    value.preserved_ranges.clear();
    value.state_impact = StateImpact::Replaced;
    value.full_erase = true;
    value.targets[0].flash_size = SIXTEEN_MB;
    value
}

fn verify(
    manifest: FlashPackageManifest,
    parts: &[(&str, &[u8])],
) -> Result<FlashPackage, PackageError> {
    FlashPackage::from_verified_parts(
        manifest,
        "manifest",
        parts
            .iter()
            .map(|(path, bytes)| ((*path).into(), bytes.to_vec()))
            .collect(),
    )
}

#[test]
fn a_merged_image_and_its_additional_images_verify_at_their_offsets() {
    let (merged, ota, fs) = (image(0x2000, 1), vec![2; 0x1000], vec![3; 0x1000]);
    let manifest = merged_manifest(&[
        ("merged.bin", FirmwarePartKind::MergedImage, 0, &merged),
        ("ota.bin", FirmwarePartKind::AdditionalImage, 0x650000, &ota),
        (
            "littlefs.bin",
            FirmwarePartKind::AdditionalImage,
            0xc90000,
            &fs,
        ),
    ]);
    let package = verify(
        manifest,
        &[
            ("merged.bin", &merged),
            ("ota.bin", &ota),
            ("littlefs.bin", &fs),
        ],
    )
    .expect("a merged image with OTA and filesystem images verifies");
    assert_eq!(
        package.manifest().write_ranges(),
        vec![
            FlashRange {
                start: 0,
                length: 0x2000
            },
            FlashRange {
                start: 0x650000,
                length: 0x1000
            },
            FlashRange {
                start: 0xc90000,
                length: 0x1000
            },
        ]
    );
}

#[test]
fn a_merged_image_must_start_at_zero_and_be_first() {
    let merged = image(0x1000, 1);
    for parts in [
        vec![(
            "merged.bin",
            FirmwarePartKind::MergedImage,
            0x1000,
            merged.as_slice(),
        )],
        vec![
            (
                "extra.bin",
                FirmwarePartKind::AdditionalImage,
                0x10000,
                merged.as_slice(),
            ),
            (
                "merged.bin",
                FirmwarePartKind::MergedImage,
                0,
                merged.as_slice(),
            ),
        ],
        vec![
            (
                "merged.bin",
                FirmwarePartKind::MergedImage,
                0,
                merged.as_slice(),
            ),
            (
                "extra.bin",
                FirmwarePartKind::AdditionalImage,
                0x10800,
                merged.as_slice(),
            ),
        ],
    ] {
        assert!(merged_manifest(&parts).validate().is_err(), "{parts:?}");
    }
}

#[test]
fn merged_parts_cannot_overlap_or_run_past_flash() {
    let (merged, extra) = (image(0x3000, 1), vec![2; 0x1000]);
    let overlapping = merged_manifest(&[
        ("merged.bin", FirmwarePartKind::MergedImage, 0, &merged),
        (
            "extra.bin",
            FirmwarePartKind::AdditionalImage,
            0x2000,
            &extra,
        ),
    ]);
    assert!(matches!(
        overlapping.validate(),
        Err(PackageError::ProtectedRangeOverlap)
    ));
    let past_flash = merged_manifest(&[
        ("merged.bin", FirmwarePartKind::MergedImage, 0, &merged),
        (
            "extra.bin",
            FirmwarePartKind::AdditionalImage,
            SIXTEEN_MB - 0x800,
            &extra,
        ),
    ]);
    assert!(past_flash.validate().is_err());
}

#[test]
fn a_merged_image_that_is_not_an_esp_image_is_refused() {
    let not_esp = vec![0; 0x1000];
    let manifest = merged_manifest(&[("merged.bin", FirmwarePartKind::MergedImage, 0, &not_esp)]);
    let error = verify(manifest, &[("merged.bin", &not_esp)]).unwrap_err();
    assert!(error.to_string().contains("0xE9"), "{error}");
}

#[test]
fn a_full_erase_is_stated_and_only_a_merged_image_states_it() {
    let merged = image(0x1000, 1);
    let parts = [(
        "merged.bin",
        FirmwarePartKind::MergedImage,
        0,
        merged.as_slice(),
    )];
    assert!(merged_manifest(&parts).validate().is_ok());

    let mut keeps_state = merged_manifest(&parts);
    keeps_state.preserved_ranges = vec![FlashRange {
        start: 0xff0000,
        length: 0x10000,
    }];
    assert!(
        keeps_state.validate().is_err(),
        "a full erase cannot claim to preserve"
    );

    let mut unknown = merged_manifest(&parts);
    unknown.state_impact = StateImpact::Unknown;
    assert!(unknown.validate().is_err(), "a full erase replaces state");

    let mut container = manifest(b"elf");
    container.full_erase = true;
    container.preserved_ranges.clear();
    container.state_impact = StateImpact::Replaced;
    assert!(
        container.validate().is_err(),
        "only a merged image may state a full erase"
    );
}

#[test]
fn without_a_full_erase_a_merged_image_honours_preserved_ranges() {
    let merged = image(0x2000, 1);
    let mut manifest =
        merged_manifest(&[("merged.bin", FirmwarePartKind::MergedImage, 0, &merged)]);
    manifest.full_erase = false;
    manifest.state_impact = StateImpact::Unknown;
    manifest.preserved_ranges = vec![FlashRange {
        start: 0x1000,
        length: 0x1000,
    }];
    assert!(matches!(
        manifest.validate(),
        Err(PackageError::ProtectedRangeOverlap)
    ));
    manifest.preserved_ranges = vec![FlashRange {
        start: 0x3f0000,
        length: 0x10000,
    }];
    assert!(manifest.validate().is_ok());
}
