//! Staging a catalog: tracked templates plus built images become verifiable packages.
//!
//! For each index entry with a `<name>.template.toml` beside the index, the caller supplies the
//! built payload and staging writes the payload and its completed manifest. Entries without a
//! template are full manifests and are copied with their verified parts; a qualified vendor
//! download is copied as its manifest alone.
//! The staged index is then verified as a whole, so a stage that Linkboy would refuse fails here.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use super::{CatalogError, PackageIndex};
use crate::package::{BuildFacts, FlashPackage, FlashPackageManifest, PackageTemplate};

/// A built image for one templated package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedPayload {
    pub bytes: Vec<u8>,
    /// Required for an espflash ELF; `None` for a UF2, whose blocks give it.
    pub write_bytes: Option<u64>,
}

/// The template that stands in the source tree for the manifest an index names.
pub fn template_path(index_dir: &Path, manifest: &str) -> PathBuf {
    let stem = manifest.strip_suffix(".toml").unwrap_or(manifest);
    index_dir.join(format!("{stem}.template.toml"))
}

/// Stage `index_path` into the empty or absent `destination`, which becomes the packages
/// directory. Payload paths may climb one level, to the firmware root, and no further.
pub fn stage_catalog(
    index_path: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    source_revision: &str,
    mut payloads: BTreeMap<String, StagedPayload>,
) -> Result<PackageIndex, CatalogError> {
    let index_path = index_path.as_ref();
    let destination = destination.as_ref();
    let index = PackageIndex::load(index_path)?;
    index.validate()?;
    if destination.exists() && fs::read_dir(destination)?.next().is_some() {
        return Err(CatalogError::Invalid(format!(
            "stage destination {} is not empty",
            destination.display()
        )));
    }
    fs::create_dir_all(destination)?;
    let index_dir = index_path.parent().unwrap_or_else(|| Path::new("."));
    let package_error = |package_id: &str| {
        let package_id = package_id.to_string();
        move |source| CatalogError::Package { package_id, source }
    };

    for entry in &index.packages {
        let template_file = template_path(index_dir, &entry.manifest);
        let manifest_out = contained(destination, &entry.manifest)?;
        if template_file.is_file() {
            let template =
                PackageTemplate::load(&template_file).map_err(package_error(&entry.package_id))?;
            if template.package_id != entry.package_id {
                return Err(CatalogError::Invalid(format!(
                    "template {} names package {:?}, the index {:?}",
                    template_file.display(),
                    template.package_id,
                    entry.package_id
                )));
            }
            let payload = payloads.remove(&entry.package_id).ok_or_else(|| {
                CatalogError::Invalid(format!("no built payload for {:?}", entry.package_id))
            })?;
            let facts = BuildFacts {
                source_revision: source_revision.to_string(),
                write_bytes: payload.write_bytes,
            };
            let manifest = template
                .complete(&payload.bytes, &facts)
                .map_err(package_error(&entry.package_id))?;
            write_new(
                &contained(destination, &template.payload.path)?,
                &payload.bytes,
            )?;
            write_new(&manifest_out, toml::to_string(&manifest)?.as_bytes())?;
        } else {
            let source = index_dir.join(&entry.manifest);
            let manifest =
                FlashPackageManifest::load(&source).map_err(package_error(&entry.package_id))?;
            // A qualified download is complete as tracked: the stage carries its manifest and
            // no bytes, which are fetched on demand (protocol_parity D5).
            if !manifest.has_downloads() {
                let package =
                    FlashPackage::load(&source).map_err(package_error(&entry.package_id))?;
                for part in package.parts() {
                    write_new(
                        &contained(destination, part.declaration().label())?,
                        part.bytes(),
                    )?;
                }
            }
            write_new(&manifest_out, &fs::read(&source)?)?;
        }
    }
    if let Some(package_id) = payloads.keys().next() {
        return Err(CatalogError::Invalid(format!(
            "built payload for {package_id:?}, which the index does not template"
        )));
    }

    let staged_index = destination.join("index.toml");
    write_new(&staged_index, &fs::read(index_path)?)?;
    let staged = PackageIndex::load(&staged_index)?;
    staged.verify_packages(&staged_index)?;
    Ok(staged)
}

/// Join a manifest-relative path, refusing anything that leaves the firmware root.
fn contained(packages: &Path, relative: &str) -> Result<PathBuf, CatalogError> {
    let mut depth = 0_i32;
    let mut climbed = false;
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir if !climbed => climbed = true,
            Component::CurDir => {}
            _ => {
                return Err(CatalogError::Invalid(format!(
                    "staged path {relative:?} leaves the firmware root"
                )));
            }
        }
    }
    Ok(packages.join(relative))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CatalogError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests;
