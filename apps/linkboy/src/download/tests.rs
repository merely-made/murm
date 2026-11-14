//! Downloads are tested through an injected fetcher; no test touches the network.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;
use crate::catalog::{Availability, PackageIndex, StagedPayload, stage_catalog};

const URL: &str = "https://release.example/2.0.0/firmware-board-2.0.0.uf2";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("linkboy-download-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn cache(&self) -> FirmwareCache {
        FirmwareCache::new(self.0.join("cache"))
    }

    /// Every file under the cache, temporary names included.
    fn cached_files(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    walk(&entry.path(), found);
                } else {
                    found.push(entry.path());
                }
            }
        }
        let mut found = Vec::new();
        walk(&self.0.join("cache"), &mut found);
        found
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Serves fixed responses by URL and counts requests. An unknown URL fails the test.
#[derive(Default)]
struct Served {
    responses: BTreeMap<String, (String, Option<u64>, Vec<u8>)>,
    requests: Mutex<Vec<String>>,
}

impl Served {
    fn with(url: &str, bytes: Vec<u8>) -> Self {
        let mut served = Self::default();
        let length = bytes.len() as u64;
        served
            .responses
            .insert(url.into(), (url.into(), Some(length), bytes));
        served
    }

    fn requests(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Fetch for Served {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error> {
        self.requests.lock().unwrap().push(url.into());
        let (final_url, content_length, bytes) = self
            .responses
            .get(url)
            .unwrap_or_else(|| panic!("unexpected request for {url}"))
            .clone();
        Ok(FetchResponse {
            final_url,
            content_length,
            body: Box::new(io::Cursor::new(bytes)),
        })
    }
}

/// A fetcher that must never be called.
struct Offline;

impl Fetch for Offline {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error> {
        panic!("a fetch of {url} was attempted")
    }
}

fn uf2() -> Vec<u8> {
    crate::uf2::encode_application(&[7; 2048], 0x26000, crate::uf2::NRF52840_FAMILY_ID).unwrap()
}

/// A complete download manifest for `bytes`, whose UF2 blocks span `0x26000+0x800`.
fn manifest_text(source: &str, bytes: &[u8], qualification: &str) -> String {
    format!(
        r#"schema = 2
package_id = "vendor.board"
display_name = "Vendor firmware for the board"
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
{source}
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
{qualification}"#,
        bytes.len(),
        sha256_hex(bytes),
    )
}

const QUALIFIED: &str = r#"
[qualification]
hash_source = "Computed from the vendor file."
hash_recorded_on = "2026-11-14"
structure_checks = "Linkboy's UF2 layout check."
companion_checked = false
"#;

fn download_manifest(bytes: &[u8]) -> String {
    manifest_text(&format!("url = \"{URL}\""), bytes, QUALIFIED)
}

fn parse(text: &str) -> Result<FlashPackageManifest, PackageError> {
    let manifest: FlashPackageManifest = toml::from_str(text)?;
    manifest.validate()?;
    Ok(manifest)
}

/// A one-entry catalog naming the download manifest.
fn catalog(scratch: &Scratch, manifest: &str) -> PathBuf {
    let dir = scratch.0.join("packages");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("vendor-board.toml"), manifest).unwrap();
    fs::write(
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

fn load(
    scratch: &Scratch,
    index: &Path,
    fetcher: Option<&dyn Fetch>,
) -> Result<FlashPackage, crate::catalog::CatalogError> {
    PackageIndex::load(index).unwrap().load_package_with(
        index,
        "vendor.board",
        &scratch.cache(),
        fetcher,
    )
}

#[test]
fn a_download_payload_parses_and_names_its_source() {
    let manifest = parse(&download_manifest(&uf2())).unwrap();
    assert!(manifest.has_downloads());
    let part = &manifest.declared_parts()[0];
    assert_eq!(part.source(), Some(PartSource::Download(URL)));
    assert_eq!(part.label(), "firmware-board-2.0.0.uf2");
}

#[test]
fn a_payload_needs_exactly_one_https_source() {
    let bytes = uf2();
    for source in [
        format!("path = \"a.uf2\"\nurl = \"{URL}\""),
        String::new(),
        "url = \"http://release.example/a.uf2\"".into(),
        "url = \"https://release.example/a.uf2?token=1\"".into(),
        "url = \"https://release.example/\"".into(),
        "url = \"https://release.example/..\"".into(),
    ] {
        assert!(
            parse(&manifest_text(&source, &bytes, QUALIFIED)).is_err(),
            "{source:?} should be refused"
        );
    }
    let unknown =
        download_manifest(&bytes).replace("format = \"uf2\"", "format = \"uf2\"\nmirror = \"x\"");
    assert!(parse(&unknown).is_err(), "unknown fields stay refused");
}

#[test]
fn a_download_needs_a_truthful_qualification_record() {
    let bytes = uf2();
    let url = format!("url = \"{URL}\"");
    assert!(parse(&manifest_text(&url, &bytes, "")).is_err());
    assert!(parse(&manifest_text("path = \"a.uf2\"", &bytes, QUALIFIED)).is_err());
    let claimed = QUALIFIED.replace("companion_checked = false", "companion_checked = true");
    assert!(parse(&manifest_text(&url, &bytes, &claimed)).is_err());
    let receipted = format!(
        "{claimed}companion_checked_on = \"2026-11-14\"\ncompanion_receipt = \"https://example.invalid/receipt\"\n"
    );
    assert!(parse(&manifest_text(&url, &bytes, &receipted)).is_ok());
    let undated = QUALIFIED.replace("2026-11-14", "14 November");
    assert!(parse(&manifest_text(&url, &bytes, &undated)).is_err());
}

#[test]
fn the_catalog_lists_a_download_without_fetching_it() {
    let scratch = Scratch::new("list");
    let index = catalog(&scratch, &download_manifest(&uf2()));
    let listings = PackageIndex::load(&index)
        .unwrap()
        .listings(&index)
        .unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].availability, Availability::OnDemand);
    // Loading without a fetcher refuses rather than reaching for the network.
    let error = load(&scratch, &index, None).unwrap_err().to_string();
    assert!(error.contains("has not been downloaded"), "{error}");
    assert!(scratch.cached_files().is_empty());
}

#[test]
fn a_fetch_verifies_caches_and_is_reverified_before_reuse() {
    let scratch = Scratch::new("cache");
    let bytes = uf2();
    let index = catalog(&scratch, &download_manifest(&bytes));
    let served = Served::with(URL, bytes.clone());
    let package = load(&scratch, &index, Some(&served)).unwrap();
    assert_eq!(package.parts()[0].bytes(), bytes.as_slice());
    let cached = scratch
        .0
        .join("cache")
        .join(sha256_hex(&bytes))
        .join("firmware-board-2.0.0.uf2");
    assert_eq!(package.parts()[0].path(), cached.as_path());
    assert_eq!(scratch.cached_files(), vec![cached.clone()]);

    // A second load reads the cache and fetches nothing.
    load(&scratch, &index, None).unwrap();
    assert_eq!(served.requests(), 1);

    // A cached file that no longer matches is deleted, not used.
    fs::write(&cached, b"replaced").unwrap();
    assert!(load(&scratch, &index, None).is_err());
    assert!(scratch.cached_files().is_empty());
    load(&scratch, &index, Some(&served)).unwrap();
    assert_eq!(served.requests(), 2);
}

/// A named response and the refusal it must produce.
type Case = (&'static str, Served, fn(&DownloadError) -> bool);

#[test]
fn bad_responses_are_refused_and_leave_nothing_cached() {
    let bytes = uf2();
    let mut longer = bytes.clone();
    longer.extend_from_slice(&[0; 512]);
    let mut altered = bytes.clone();
    altered[600] ^= 1;
    let cases: [Case; 5] = [
        ("short", Served::with(URL, bytes[..512].to_vec()), |e| {
            matches!(e, DownloadError::LengthMismatch { .. })
        }),
        ("hash", Served::with(URL, altered), |e| {
            matches!(e, DownloadError::HashMismatch { .. })
        }),
        (
            "declared-oversize",
            Served::with(URL, longer.clone()),
            |e| matches!(e, DownloadError::Oversize { .. }),
        ),
        (
            "undeclared-oversize",
            {
                let mut served = Served::default();
                served
                    .responses
                    .insert(URL.into(), (URL.into(), None, longer));
                served
            },
            |e| matches!(e, DownloadError::Oversize { .. }),
        ),
        (
            "redirect",
            {
                let mut served = Served::default();
                let target = "http://mirror.example/firmware-board-2.0.0.uf2";
                served
                    .responses
                    .insert(URL.into(), (target.into(), None, bytes.clone()));
                served
            },
            |e| matches!(e, DownloadError::RedirectNotHttps { .. }),
        ),
    ];
    for (name, served, expected) in cases {
        let scratch = Scratch::new(name);
        let index = catalog(&scratch, &download_manifest(&bytes));
        let manifest =
            FlashPackageManifest::load(index.with_file_name("vendor-board.toml")).unwrap();
        let error = scratch
            .cache()
            .fetch(&manifest.declared_parts()[0], &served)
            .unwrap_err();
        assert!(expected(&error), "{name}: {error}");
        assert!(load(&scratch, &index, Some(&served)).is_err(), "{name}");
        assert!(scratch.cached_files().is_empty(), "{name} left a file");
    }
}

#[test]
fn only_https_is_requested() {
    let error = fetch_capped(&Offline, "http://release.example/a.uf2", 10).unwrap_err();
    assert!(matches!(error, DownloadError::NotHttps { .. }));
}

#[test]
fn bytes_that_hash_correctly_but_fail_the_layout_check_are_removed() {
    let scratch = Scratch::new("layout");
    let bytes = uf2();
    let wrong_range = download_manifest(&bytes).replace("length = 0x800", "length = 0x900");
    let index = catalog(&scratch, &wrong_range);
    let error = load(&scratch, &index, Some(&Served::with(URL, bytes)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("write_ranges"), "{error}");
    assert!(scratch.cached_files().is_empty());
}

#[test]
fn staging_copies_a_download_manifest_without_bytes() {
    let scratch = Scratch::new("stage");
    let manifest = download_manifest(&uf2());
    let index = catalog(&scratch, &manifest);
    let stage = scratch.0.join("stage");
    let staged = stage_catalog(
        &index,
        &stage,
        &"ab".repeat(20),
        BTreeMap::<String, StagedPayload>::new(),
    )
    .unwrap();
    assert_eq!(staged.packages.len(), 1);
    assert_eq!(
        fs::read_to_string(stage.join("vendor-board.toml")).unwrap(),
        manifest
    );
    let mut names = fs::read_dir(&stage)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["index.toml", "vendor-board.toml"]);
}

#[test]
fn qualify_drafts_an_entry_from_fetched_bytes() {
    let bytes = uf2();
    let served = Served::with(URL, bytes.clone());
    let draft = crate::qualify::qualify(&served, URL, "2026-11-14").unwrap();
    assert_eq!(draft.byte_length, bytes.len() as u64);
    assert_eq!(draft.sha256, sha256_hex(&bytes));
    assert!(matches!(
        draft.layout,
        crate::qualify::DetectedLayout::Uf2 {
            nrf52840: true,
            write_bytes: 2048,
            ..
        }
    ));
    let value: toml::Value = toml::from_str(&draft.toml).unwrap();
    assert_eq!(value["payload"]["url"].as_str(), Some(URL));
    assert_eq!(
        value["payload"]["sha256"].as_str(),
        Some(sha256_hex(&bytes).as_str())
    );
    assert_eq!(
        value["write_ranges"][0]["start"].as_integer(),
        Some(0x26000)
    );
    assert_eq!(
        value["qualification"]["companion_checked"].as_bool(),
        Some(false)
    );
    // A draft is not a package until a person fills it in.
    assert!(draft.toml.contains("FILL"));
}
