//! The strict on-disk manifest shape and its helper requirements.

use serde::{Deserialize, Serialize};

use super::{
    BoardFamily, FirmwarePartKind, FlashRange, FlashRoute, NODE_TIMEBASE_PRESERVED_RANGE,
    PERSISTENT_STATE_SCHEMA, PayloadFormat, ProcessorKind, StateImpact,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperArtifact {
    /// Linkboy's installed helper directory name, for example `windows-x86_64`.
    pub platform: String,
    /// Digest of the executable after extracting the upstream release archive.
    pub binary_sha256: String,
    /// Digest published for the retained upstream release archive.
    pub archive_sha256: String,
    pub archive_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperRequirement {
    pub route: FlashRoute,
    pub program: String,
    pub version: String,
    /// Legacy single-platform custody. New public packages use `artifacts`.
    #[serde(default)]
    pub binary_sha256: Option<String>,
    /// Official release artifacts admitted on each supported host platform.
    #[serde(default)]
    pub artifacts: Vec<HelperArtifact>,
    pub license: String,
    pub source_url: String,
    pub notice: String,
}

impl HelperRequirement {
    pub fn artifact_for_current_platform(&self) -> Option<&HelperArtifact> {
        let platform = helper_platform();
        self.artifacts
            .iter()
            .find(|artifact| artifact.platform == platform)
    }

    pub fn expected_binary_sha256(&self) -> Option<&str> {
        self.artifact_for_current_platform()
            .map(|artifact| artifact.binary_sha256.as_str())
            .or(self.binary_sha256.as_deref())
    }
}

pub fn helper_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePayload {
    /// A file beside the manifest. Exactly one of `path` and `url` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A vendor release fetched on demand and verified against `byte_length` and `sha256`
    /// (protocol_parity D5). The tracked manifest is the only thing that vouches for the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub format: PayloadFormat,
    pub byte_length: u64,
    pub sha256: String,
    /// Bytes represented by the application image in a container format. This is distinct from
    /// byte_length, which is always the exact file length that was hashed.
    pub write_bytes: u64,
}

/// One immutable file in an ordered firmware package.
///
/// ESP sparse packages state a concrete flash offset per part. Container formats, such as the
/// nRF DFU ZIP and a self-contained ESP ELF, keep their address layout inside the container and
/// therefore have no outer offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePart {
    pub kind: FirmwarePartKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub format: PayloadFormat,
    pub offset: Option<u32>,
    pub byte_length: u64,
    pub sha256: String,
    pub write_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PublisherSignatureFormat {
    Minisign,
}

/// Evidence retained from an upstream publisher. It is deliberately evidence, not a Linkboy
/// trust root: the signed Merely package index still decides which network package is admitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherSignature {
    pub format: PublisherSignatureFormat,
    pub key_id: String,
    pub signed_manifest_url: String,
    pub signed_manifest_sha256: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageTarget {
    pub family: BoardFamily,
    pub revision: String,
    pub processor: ProcessorKind,
    pub flash_size: u32,
    pub bootloader: String,
    pub route: FlashRoute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedApplication {
    pub board: BoardFamily,
    pub version: String,
    /// External firmware can require a human to exercise its own interface after the helper has
    /// verified every written part. This prevents a Retinue-only serial probe from claiming a
    /// foreign application is broken.
    #[serde(default)]
    pub manual_check: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryInstructions {
    pub before_write: String,
    pub after_failure: String,
}

/// Evidence that an image can safely continue a durable native-node announce sequence after a
/// reset. Absent means no such claim; it stays small and additive so older Linkboy builds can
/// still read schema-2 manifests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistentStateCompatibility {
    pub schema: u32,
    pub native_node_guard: bool,
    pub preserved_range: FlashRange,
}

impl PersistentStateCompatibility {
    pub fn supports_native_node_guard(&self) -> bool {
        self.schema == PERSISTENT_STATE_SCHEMA
            && self.native_node_guard
            && self.preserved_range == NODE_TIMEBASE_PRESERVED_RANGE
    }
}

/// The strict on-disk shape. Unknown keys are rejected so a typo cannot silently weaken a plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlashPackageManifest {
    pub schema: u32,
    pub package_id: String,
    pub display_name: String,
    pub version: String,
    pub publisher: String,
    pub helpers: Vec<HelperRequirement>,
    /// The legacy one-file package form. Schema v2 keeps it for the current Retinue routes;
    /// new sparse packages use `parts` instead.
    #[serde(default)]
    pub payload: Option<PackagePayload>,
    /// Ordered immutable parts for a sparse package. Exactly one of `payload` and `parts` is
    /// present, so an old opaque container cannot quietly become an incomplete sparse write.
    #[serde(default)]
    pub parts: Vec<PackagePart>,
    pub targets: Vec<PackageTarget>,
    /// Explicit ranges for an opaque one-file container. Sparse packages derive their ranges
    /// from the verified part offsets and must not carry a second, contradictory range list.
    #[serde(default)]
    pub write_ranges: Vec<FlashRange>,
    pub preserved_ranges: Vec<FlashRange>,
    pub regions: Vec<String>,
    pub channel_capabilities: Vec<String>,
    pub state_impact: StateImpact,
    pub expected_application: ExpectedApplication,
    pub license: String,
    pub notices: String,
    pub source_revision: String,
    pub source_url: String,
    pub origin_url: String,
    pub publisher_signature: Option<PublisherSignature>,
    pub recovery: RecoveryInstructions,
    /// Optional additive declaration. Foreign and legacy packages omit it and therefore never
    /// accidentally claim support for the guarded Retinue native-node state.
    #[serde(default)]
    pub persistent_state: Option<PersistentStateCompatibility>,
    /// The package erases all of flash before writing, preserved state included. Only a vendor
    /// merged image states this, with no preserved ranges; the confirmation screen shows it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full_erase: bool,
    /// Required for a package with a downloaded part: what was checked before it was listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualification: Option<Qualification>,
}

/// What was checked before a vendor release was listed (protocol_parity D5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    /// Where the pinned length and sha256 came from.
    pub hash_source: String,
    /// The date (`YYYY-MM-DD`) the hash was recorded.
    pub hash_recorded_on: String,
    /// The structure checks the bytes passed.
    pub structure_checks: String,
    /// Whether Signalman's stock-companion client was checked against this release on a real
    /// board. True needs the date and a receipt URL.
    pub companion_checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_checked_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_receipt: Option<String>,
}

/// Where a part's bytes come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartSource<'a> {
    /// A path relative to the manifest.
    Local(&'a str),
    /// An https URL, fetched on demand.
    Download(&'a str),
}

impl PackagePart {
    /// The declared source, once validation has established that exactly one is present.
    pub fn source(&self) -> Option<PartSource<'_>> {
        match (&self.path, &self.url) {
            (Some(path), None) => Some(PartSource::Local(path)),
            (None, Some(url)) => Some(PartSource::Download(url)),
            _ => None,
        }
    }

    /// The name a part keeps on disk: a local path, or a URL's last segment.
    pub fn label(&self) -> &str {
        match self.source() {
            Some(PartSource::Local(path)) => path,
            Some(PartSource::Download(url)) => url.rsplit('/').next().unwrap_or(url),
            None => "",
        }
    }
}

impl FlashPackageManifest {
    /// Parse and validate a manifest without reading any part.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, super::PackageError> {
        let manifest: Self = toml::from_str(&std::fs::read_to_string(path)?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The structural checks every package passes before any byte is read.
    pub fn validate(&self) -> Result<(), super::PackageError> {
        super::validate::validate_manifest(self)
    }

    /// Whether any part is fetched on demand rather than read beside the manifest.
    pub fn has_downloads(&self) -> bool {
        self.declared_parts()
            .iter()
            .any(|part| matches!(part.source(), Some(PartSource::Download(_))))
    }

    pub fn helper_for(&self, route: &FlashRoute) -> Option<&HelperRequirement> {
        self.helpers.iter().find(|helper| &helper.route == route)
    }

    pub fn write_ranges(&self) -> Vec<FlashRange> {
        if self.parts.is_empty() {
            self.write_ranges.clone()
        } else {
            self.parts
                .iter()
                .filter_map(|part| {
                    Some(FlashRange {
                        start: part.offset?,
                        length: u32::try_from(part.write_bytes).ok()?,
                    })
                })
                .collect()
        }
    }

    pub fn declared_parts(&self) -> Vec<PackagePart> {
        match &self.payload {
            Some(payload) => vec![PackagePart {
                kind: FirmwarePartKind::Application,
                path: payload.path.clone(),
                url: payload.url.clone(),
                format: payload.format.clone(),
                offset: None,
                byte_length: payload.byte_length,
                sha256: payload.sha256.clone(),
                write_bytes: payload.write_bytes,
            }],
            None => self.parts.clone(),
        }
    }
}
