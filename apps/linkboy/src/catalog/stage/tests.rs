use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::package::PackageError;

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

fn repository_index() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../firmware/packages/index.toml")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("linkboy-stage-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn packages(&self) -> PathBuf {
        self.0.join("firmware/packages")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn synthetic_payloads() -> BTreeMap<String, StagedPayload> {
    let uf2 = crate::uf2::encode_application(&[3; 2048], 0x26000, crate::uf2::NRF52840_FAMILY_ID)
        .unwrap();
    BTreeMap::from([
        (
            "retinue.heltec-v4".to_string(),
            StagedPayload {
                bytes: vec![0x5a; 8192],
                write_bytes: Some(4096),
            },
        ),
        (
            "retinue.t114".to_string(),
            StagedPayload {
                bytes: uf2,
                write_bytes: None,
            },
        ),
    ])
}

#[test]
fn every_index_entry_is_a_template_or_a_full_manifest_that_exists() {
    let index = PackageIndex::load(repository_index()).unwrap();
    let dir = repository_index().parent().unwrap().to_path_buf();
    for entry in &index.packages {
        let template = template_path(&dir, &entry.manifest);
        assert!(
            template.is_file() || dir.join(&entry.manifest).is_file(),
            "{} names {} with neither a template nor a manifest",
            entry.package_id,
            entry.manifest
        );
        assert!(
            !(template.is_file() && dir.join(&entry.manifest).exists()),
            "{} has both a template and a tracked manifest",
            entry.package_id
        );
    }
}

#[test]
fn the_repository_index_stages_into_a_catalog_that_verifies() {
    let scratch = Scratch::new("verifies");
    let staged = stage_catalog(
        repository_index(),
        scratch.packages(),
        REVISION,
        synthetic_payloads(),
    )
    .expect("stage verifies");
    assert_eq!(staged.packages.len(), 3);
    let v4 = FlashPackage::load(scratch.packages().join("heltec-v4-current.toml")).unwrap();
    assert_eq!(v4.manifest().source_revision, REVISION);
    assert!(
        scratch
            .0
            .join("firmware/heltec-v4-phy/tulle-heltec-v4-phy")
            .is_file()
    );
    assert!(
        scratch
            .0
            .join("firmware/t114-phy/tulle-t114-phy-current.uf2")
            .is_file()
    );
}

#[test]
fn a_staged_package_with_one_flipped_payload_byte_is_refused() {
    let scratch = Scratch::new("flipped");
    stage_catalog(
        repository_index(),
        scratch.packages(),
        REVISION,
        synthetic_payloads(),
    )
    .unwrap();
    let image = scratch.0.join("firmware/heltec-v4-phy/tulle-heltec-v4-phy");
    let mut bytes = fs::read(&image).unwrap();
    bytes[100] ^= 0x01;
    fs::write(&image, bytes).unwrap();
    assert!(matches!(
        FlashPackage::load(scratch.packages().join("heltec-v4-current.toml")),
        Err(PackageError::HashMismatch { .. })
    ));
    let index = scratch.packages().join("index.toml");
    assert!(
        PackageIndex::load(&index)
            .unwrap()
            .verify_packages(&index)
            .is_err()
    );
}

#[test]
fn staging_refuses_a_missing_or_unexpected_payload_and_a_used_destination() {
    let mut missing = synthetic_payloads();
    missing.remove("retinue.t114");
    let scratch = Scratch::new("missing");
    assert!(stage_catalog(repository_index(), scratch.packages(), REVISION, missing).is_err());

    let mut extra = synthetic_payloads();
    extra.insert(
        "not.in.index".into(),
        StagedPayload {
            bytes: vec![1],
            write_bytes: None,
        },
    );
    let scratch = Scratch::new("extra");
    assert!(stage_catalog(repository_index(), scratch.packages(), REVISION, extra).is_err());

    let scratch = Scratch::new("used");
    fs::create_dir_all(scratch.packages()).unwrap();
    fs::write(scratch.packages().join("left-over"), b"x").unwrap();
    assert!(
        stage_catalog(
            repository_index(),
            scratch.packages(),
            REVISION,
            synthetic_payloads()
        )
        .is_err()
    );
}

#[test]
fn a_payload_path_cannot_leave_the_firmware_root() {
    let root = Path::new("stage/firmware/packages");
    assert!(contained(root, "../heltec-v4-phy/image").is_ok());
    assert!(contained(root, "vendor/image.uf2").is_ok());
    assert!(contained(root, "../../outside").is_err());
    assert!(contained(root, "/absolute").is_err());
}
