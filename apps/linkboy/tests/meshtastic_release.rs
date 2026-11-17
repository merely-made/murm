//! The qualified Meshtastic T114 release: a tracked manifest pinning the vendor's file, which
//! is downloaded on demand and is no longer in the repository (protocol_parity D5).

use std::path::PathBuf;

use linkboy::package::FlashPackageManifest;
use linkboy::{FlashRoute, PartSource};

fn package_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../firmware/packages/meshtastic-t114-2.7.26.54e0d8d.toml")
}

#[test]
fn official_t114_uf2_is_pinned_by_url_hash_and_address_map() {
    let manifest = FlashPackageManifest::load(package_path()).expect("qualified Meshtastic entry");
    let part = &manifest.declared_parts()[0];
    assert_eq!(
        part.source(),
        Some(PartSource::Download(
            "https://release.meshtastic.org/2.7.26.54e0d8d/firmware-heltec-mesh-node-t114-2.7.26.54e0d8d.uf2"
        ))
    );
    assert_eq!(part.byte_length, 1_467_392);
    let qualification = manifest
        .qualification
        .as_ref()
        .expect("qualification record");
    assert!(!qualification.companion_checked);
    assert_eq!(
        manifest.source_revision,
        "54e0d8d0ab2ff56b3a9ce967e53f79e49af560fb"
    );
    assert_eq!(
        manifest.source_url,
        "https://github.com/meshtastic/firmware/tree/54e0d8d0ab2ff56b3a9ce967e53f79e49af560fb"
    );
    assert_eq!(manifest.targets[0].route, FlashRoute::Uf2MassStorage);
    assert_eq!(manifest.write_ranges()[0].start, 0x26000);
    assert_eq!(manifest.write_ranges()[0].length, 0xb3200);
    assert_eq!(part.write_bytes, 733696);
    assert_eq!(
        part.sha256,
        "e038b1067b6fa12ed77853337e0e416425f30a7589a907f8ac1600f21b8d02b9"
    );
}
