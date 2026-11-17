use std::path::PathBuf;

use super::*;
use crate::package::PackageError;

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

fn packages_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../firmware/packages")
}

fn template(name: &str) -> PackageTemplate {
    PackageTemplate::load(packages_dir().join(name)).expect("repository template parses")
}

fn facts(write_bytes: Option<u64>) -> BuildFacts {
    BuildFacts {
        source_revision: REVISION.into(),
        write_bytes,
    }
}

#[test]
fn the_repository_templates_parse_as_templates_and_linkboy_refuses_them_as_manifests() {
    for name in [
        "heltec-v4-current.template.toml",
        "t114-current.template.toml",
    ] {
        let path = packages_dir().join(name);
        let parsed = PackageTemplate::load(&path).expect("parses as a template");
        assert_eq!(parsed.template, PACKAGE_TEMPLATE_SCHEMA);
        assert!(
            matches!(FlashPackage::load(&path), Err(PackageError::Parse(_))),
            "{name} must never verify as a package"
        );
    }
}

#[test]
fn a_template_carrying_a_build_field_is_refused() {
    let text = std::fs::read_to_string(packages_dir().join("t114-current.template.toml"))
        .unwrap()
        .replace("format = \"uf2\"", "format = \"uf2\"\nsha256 = \"00\"");
    assert!(toml::from_str::<PackageTemplate>(&text).is_err());
}

#[test]
fn a_v4_template_completes_with_the_build_facts_and_verifies() {
    let payload = vec![0x5a; 8192];
    let manifest = template("heltec-v4-current.template.toml")
        .complete(&payload, &facts(Some(4096)))
        .expect("completes");
    assert_eq!(manifest.version, "0.0.1-0123456");
    assert_eq!(manifest.source_revision, REVISION);
    assert_eq!(
        manifest.expected_application.manual_check.as_deref(),
        Some(format!("Status must report build={REVISION} image=v4-modem-usb.").as_str())
    );
    assert_eq!(
        manifest.write_ranges,
        [FlashRange {
            start: 0,
            length: 0x3f0000
        }]
    );
    let payload_fields = manifest.payload.as_ref().unwrap();
    assert_eq!(payload_fields.byte_length, 8192);
    assert_eq!(payload_fields.sha256, sha256_hex(&payload));
    assert_eq!(payload_fields.write_bytes, 4096);
}

#[test]
fn an_espflash_template_needs_the_reported_write_bytes() {
    let result = template("heltec-v4-current.template.toml").complete(&[1; 64], &facts(None));
    assert!(matches!(result, Err(PackageError::InvalidField(_))));
}

#[test]
fn a_uf2_template_takes_its_write_layout_from_the_blocks() {
    let uf2 = crate::uf2::encode_application(&[7; 1000], 0x26000, crate::uf2::NRF52840_FAMILY_ID)
        .unwrap();
    let t114 = template("t114-current.template.toml");
    let manifest = t114.complete(&uf2, &facts(None)).expect("completes");
    assert_eq!(
        manifest.write_ranges,
        [FlashRange {
            start: 0x26000,
            length: 1024
        }]
    );
    assert_eq!(manifest.payload.as_ref().unwrap().write_bytes, 1024);
    assert!(t114.complete(&uf2, &facts(Some(1024))).is_err());
}

#[test]
fn a_revision_that_is_not_a_full_commit_id_is_refused() {
    let mut short = facts(Some(64));
    short.source_revision = "0123456".into();
    assert!(
        template("heltec-v4-current.template.toml")
            .complete(&[1; 64], &short)
            .is_err()
    );
}
