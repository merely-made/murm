//! Package templates: a manifest without the values only a build decides.
//!
//! The repository keeps templates, never built images (protocol_parity D4). A staging step
//! builds the image and completes the template into a full manifest beside it. A template
//! carries `template = 1`, which the strict manifest shape rejects, so a template can never
//! be mistaken for a verifiable package.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::uf2_layout::uf2_extent;
use super::{
    BoardFamily, ExpectedApplication, FlashPackage, FlashPackageManifest, FlashRange,
    HelperRequirement, PackageError, PackagePayload, PackageTarget, PayloadFormat,
    PersistentStateCompatibility, ProcessorKind, RecoveryInstructions, StateImpact,
    verified::sha256_hex,
};

pub const PACKAGE_TEMPLATE_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplatePayload {
    pub path: String,
    pub format: PayloadFormat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateApplication {
    pub board: BoardFamily,
    pub version: String,
    /// The `image=` token the running application reports beside its `build=` revision.
    pub image: String,
}

/// Everything in a manifest except `version`, `source_revision`, `write_ranges`, the payload's
/// `byte_length`, `sha256` and `write_bytes`, and the `build=` check, which staging writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageTemplate {
    pub template: u32,
    pub schema: u32,
    pub package_id: String,
    pub display_name: String,
    pub publisher: String,
    pub regions: Vec<String>,
    pub channel_capabilities: Vec<String>,
    pub state_impact: StateImpact,
    pub license: String,
    pub notices: String,
    pub source_url: String,
    pub origin_url: String,
    pub helpers: Vec<HelperRequirement>,
    pub payload: TemplatePayload,
    pub targets: Vec<PackageTarget>,
    pub preserved_ranges: Vec<FlashRange>,
    pub expected_application: TemplateApplication,
    pub recovery: RecoveryInstructions,
    #[serde(default)]
    pub persistent_state: Option<PersistentStateCompatibility>,
}

/// Facts a build supplies. `write_bytes` is required for an espflash ELF, whose application
/// size only espflash can report; a UF2's is read from its blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFacts {
    pub source_revision: String,
    pub write_bytes: Option<u64>,
}

impl PackageTemplate {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        let template: Self = toml::from_str(&fs::read_to_string(path)?)?;
        if template.template != PACKAGE_TEMPLATE_SCHEMA {
            return Err(PackageError::InvalidField(format!(
                "unsupported template schema {}",
                template.template
            )));
        }
        Ok(template)
    }

    /// Complete this template for `payload` and verify the result exactly as `FlashPackage::load`
    /// would, so a staged manifest that Linkboy would refuse is never written.
    pub fn complete(
        &self,
        payload: &[u8],
        facts: &BuildFacts,
    ) -> Result<FlashPackageManifest, PackageError> {
        let revision = &facts.source_revision;
        if revision.len() != 40
            || !revision
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(PackageError::InvalidField(
                "source_revision must be a full lowercase commit id".into(),
            ));
        }
        let (write_bytes, write_ranges) = match self.payload.format {
            PayloadFormat::Uf2 => {
                let nrf = self
                    .targets
                    .iter()
                    .any(|target| target.processor == ProcessorKind::Nrf52840);
                if facts.write_bytes.is_some() {
                    return Err(PackageError::InvalidField(
                        "a UF2's write_bytes come from its blocks, not the caller".into(),
                    ));
                }
                uf2_extent(payload, nrf)?
            }
            PayloadFormat::EspflashElf => {
                let write_bytes = facts.write_bytes.ok_or_else(|| {
                    PackageError::InvalidField("an espflash ELF needs write_bytes".into())
                })?;
                // espflash writes from flash offset 0 up to the first preserved range.
                let end = self
                    .preserved_ranges
                    .iter()
                    .map(|range| range.start)
                    .min()
                    .ok_or_else(|| {
                        PackageError::InvalidField("an espflash ELF needs a preserved range".into())
                    })?;
                (
                    write_bytes,
                    vec![FlashRange {
                        start: 0,
                        length: end,
                    }],
                )
            }
            ref other => {
                return Err(PackageError::InvalidField(format!(
                    "templates cannot stage {other:?} payloads"
                )));
            }
        };
        let manifest = FlashPackageManifest {
            schema: self.schema,
            package_id: self.package_id.clone(),
            display_name: self.display_name.clone(),
            version: format!("{}-{}", self.expected_application.version, &revision[..7]),
            publisher: self.publisher.clone(),
            helpers: self.helpers.clone(),
            payload: Some(PackagePayload {
                path: self.payload.path.clone(),
                format: self.payload.format.clone(),
                byte_length: payload.len() as u64,
                sha256: sha256_hex(payload),
                write_bytes,
            }),
            parts: Vec::new(),
            targets: self.targets.clone(),
            write_ranges,
            preserved_ranges: self.preserved_ranges.clone(),
            regions: self.regions.clone(),
            channel_capabilities: self.channel_capabilities.clone(),
            state_impact: self.state_impact.clone(),
            expected_application: ExpectedApplication {
                board: self.expected_application.board.clone(),
                version: self.expected_application.version.clone(),
                manual_check: Some(format!(
                    "Status must report build={revision} image={}.",
                    self.expected_application.image
                )),
            },
            license: self.license.clone(),
            notices: self.notices.clone(),
            source_revision: revision.clone(),
            source_url: self.source_url.clone(),
            origin_url: self.origin_url.clone(),
            publisher_signature: None,
            recovery: self.recovery.clone(),
            persistent_state: self.persistent_state.clone(),
        };
        FlashPackage::from_parts(
            manifest.clone(),
            "staged manifest",
            &self.payload.path,
            payload.to_vec(),
        )?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests;
