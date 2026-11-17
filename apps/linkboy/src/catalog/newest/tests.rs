use std::path::PathBuf;

use super::*;
use crate::catalog::{Availability, CatalogPackage, CatalogState};
use crate::package::FlashPackageManifest;

/// The tracked Meshtastic T114 manifest, renamed and re-versioned.
fn listing(publisher: &str, package_id: &str, version: &str, board: BoardFamily) -> PackageListing {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../firmware/packages/meshtastic-t114-2.7.26.54e0d8d.toml");
    let mut manifest = FlashPackageManifest::load(path).unwrap();
    manifest.package_id = package_id.into();
    manifest.publisher = publisher.into();
    manifest.version = version.into();
    manifest.targets[0].family = board;
    PackageListing {
        entry: CatalogPackage {
            package_id: package_id.into(),
            release_sequence: 1,
            manifest: format!("{package_id}.toml"),
            firmware_publisher: publisher.into(),
            state: CatalogState::Partial,
            instructions_url: "https://example.invalid/readme".into(),
            recovery_url: "https://example.invalid/recovery".into(),
            installer_receipts: Vec::new(),
            recovery_receipts: Vec::new(),
            receipt_hosts: Vec::new(),
            purchase_url: None,
        },
        manifest,
        availability: Availability::OnDemand,
    }
}

#[test]
fn vendor_version_strings_parse_to_a_triple_and_a_commit() {
    for (text, triple, build) in [
        ("2.7.26.54e0d8d", (2, 7, 26), Some("54e0d8d")),
        ("2.8.2.d5f1619", (2, 8, 2), Some("d5f1619")),
        ("v1.17.1-d929643", (1, 17, 1), Some("d929643")),
        ("1.18.0-81dbae5", (1, 18, 0), Some("81dbae5")),
        ("v1.17.1", (1, 17, 1), None),
        ("0.1.0-711816a", (0, 1, 0), Some("711816a")),
    ] {
        let version = ReleaseVersion::parse(text).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(version.triple, triple, "{text}");
        assert_eq!(version.build.as_deref(), build, "{text}");
    }
    for text in [
        "",
        "2.7",
        "2.7.26.54e0d8d.1",
        "2.7.26.54e0d8d-abcdef0",
        "v1.17.1-beta",
        "v1.17.1-d92964",
        "1.17.1-d929643-dirty",
        "0.0.1-v47",
    ] {
        assert_eq!(ReleaseVersion::parse(text), None, "{text:?}");
    }
}

#[test]
fn a_device_build_matches_the_listed_release() {
    let listed = ReleaseVersion::parse("1.17.1-d929643").unwrap();
    assert!(
        ReleaseVersion::parse("v1.17.1-d929643")
            .unwrap()
            .same_release(&listed)
    );
    assert!(
        ReleaseVersion::parse("v1.17.1")
            .unwrap()
            .same_release(&listed)
    );
    assert!(
        !ReleaseVersion::parse("v1.17.1-1234567")
            .unwrap()
            .same_release(&listed)
    );
    assert!(
        !ReleaseVersion::parse("v1.17.10-d929643")
            .unwrap()
            .same_release(&listed)
    );
}

#[test]
fn the_newest_listing_is_chosen_per_publisher_and_board() {
    let listings = [
        listing(
            "Meshtastic",
            "m.t114.old",
            "2.7.15.567b8ea",
            BoardFamily::T114,
        ),
        listing("Meshtastic", "m.t114", "2.7.26.54e0d8d", BoardFamily::T114),
        listing("Meshtastic", "m.v4", "2.8.2.d5f1619", BoardFamily::HeltecV4),
        listing("MeshCore", "c.t114", "1.18.0-81dbae5", BoardFamily::T114),
    ];
    let pick = |publisher, board| {
        newest(&listings, publisher, &board)
            .unwrap()
            .map(|l| l.entry.package_id.as_str())
    };
    assert_eq!(pick("Meshtastic", BoardFamily::T114), Some("m.t114"));
    assert_eq!(pick("Meshtastic", BoardFamily::HeltecV4), Some("m.v4"));
    assert_eq!(pick("MeshCore", BoardFamily::T114), Some("c.t114"));
    assert_eq!(pick("MeshCore", BoardFamily::HeltecV4), None);
}

#[test]
fn one_triple_listed_twice_is_ambiguous_and_offers_nothing() {
    let listings = [
        listing("Meshtastic", "a", "2.7.26.54e0d8d", BoardFamily::T114),
        listing("Meshtastic", "b", "2.7.26.1234567", BoardFamily::T114),
    ];
    assert!(matches!(
        newest(&listings, "Meshtastic", &BoardFamily::T114),
        Err(NewestError::Ambiguous { .. })
    ));
    assert!(matches!(
        standing(
            &listings,
            "Meshtastic",
            Some(&BoardFamily::T114),
            Some("2.7.26.54e0d8d")
        ),
        Standing::Unknown { .. }
    ));
}

#[test]
fn a_radio_stands_on_a_listed_release_off_the_list_or_unknown() {
    let listings = [
        listing("Meshtastic", "m.old", "2.7.15.567b8ea", BoardFamily::T114),
        listing("Meshtastic", "m.new", "2.7.26.54e0d8d", BoardFamily::T114),
        listing("MeshCore", "c.v4", "1.17.1-d929643", BoardFamily::HeltecV4),
    ];
    let t114 = Some(&BoardFamily::T114);
    match standing(&listings, "Meshtastic", t114, Some("2.7.15.567b8ea")) {
        Standing::Listed { current, newer } => {
            assert_eq!(current.entry.package_id, "m.old");
            assert_eq!(newer.map(|l| l.entry.package_id.as_str()), Some("m.new"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        standing(&listings, "Meshtastic", t114, Some("2.7.26.54e0d8d")),
        Standing::Listed { newer: None, .. }
    ));
    // A newer release than any listed, an alpha, and an unparseable string are all off the list.
    for reported in ["2.8.2.d5f1619", "2.7.20.aaaaaaa", "custom-build"] {
        assert_eq!(
            standing(&listings, "Meshtastic", t114, Some(reported)),
            Standing::NotListed {
                reported: reported.into()
            }
        );
    }
    assert!(matches!(
        standing(
            &listings,
            "MeshCore",
            Some(&BoardFamily::HeltecV4),
            Some("v1.17.1-d929643")
        ),
        Standing::Listed { newer: None, .. }
    ));
    assert!(matches!(
        standing(&listings, "Meshtastic", None, Some("2.7.26.54e0d8d")),
        Standing::Unknown { .. }
    ));
    assert!(matches!(
        standing(&listings, "Meshtastic", t114, None),
        Standing::Unknown { .. }
    ));
}
