//! What a catalog offers, without fetching anything.
//!
//! A package beside its manifest is verified when the catalog loads, as before. A qualified
//! vendor download is listed from its tracked manifest alone and verified when it is fetched
//! (protocol_parity D5), so loading a catalog never touches the network.

use std::path::Path;

use super::{CatalogError, CatalogPackage, PackageIndex};
use crate::download::{Fetch, FirmwareCache};
use crate::package::{FlashPackage, FlashPackageManifest};

/// Whether a listed package's bytes have been checked yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Every part was read beside the manifest and verified at load.
    Verified,
    /// Fetched and verified only when the owner asks to install it.
    OnDemand,
}

/// One index entry with its validated manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageListing {
    pub entry: CatalogPackage,
    pub manifest: FlashPackageManifest,
    pub availability: Availability,
}

impl PackageIndex {
    /// Validate the index and every manifest it names; verify every local payload.
    pub fn listings(
        &self,
        index_path: impl AsRef<Path>,
    ) -> Result<Vec<PackageListing>, CatalogError> {
        self.validate()?;
        let parent = index_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        self.packages
            .iter()
            .map(|entry| {
                let manifest_path = parent.join(&entry.manifest);
                let package_error = |source| CatalogError::Package {
                    package_id: entry.package_id.clone(),
                    source,
                };
                let manifest = FlashPackageManifest::load(&manifest_path).map_err(package_error)?;
                let availability = if manifest.has_downloads() {
                    Availability::OnDemand
                } else {
                    FlashPackage::load(&manifest_path).map_err(package_error)?;
                    Availability::Verified
                };
                if manifest.package_id != entry.package_id {
                    return Err(CatalogError::Invalid(format!(
                        "catalog package {:?} names manifest package {:?}",
                        entry.package_id, manifest.package_id
                    )));
                }
                if manifest.publisher != entry.firmware_publisher {
                    return Err(CatalogError::Invalid(format!(
                        "catalog package {:?} names firmware publisher {:?}, but its manifest says {:?}",
                        entry.package_id, entry.firmware_publisher, manifest.publisher
                    )));
                }
                Ok(PackageListing {
                    entry: entry.clone(),
                    manifest,
                    availability,
                })
            })
            .collect()
    }

    /// Load one package, reading a downloaded part from `cache` and fetching it with `fetcher`
    /// when it is absent. Without a fetcher, an uncached download is refused, never fetched.
    pub fn load_package_with(
        &self,
        index_path: impl AsRef<Path>,
        package_id: &str,
        cache: &FirmwareCache,
        fetcher: Option<&dyn Fetch>,
    ) -> Result<FlashPackage, CatalogError> {
        let index_path = index_path.as_ref();
        self.listings(index_path)?;
        let entry = self.package(package_id).ok_or_else(|| {
            CatalogError::Invalid(format!("package {package_id:?} is not in the index"))
        })?;
        let parent = index_path.parent().unwrap_or_else(|| Path::new("."));
        crate::download::load_package(parent.join(&entry.manifest), cache, fetcher).map_err(
            |source| CatalogError::Download {
                package_id: entry.package_id.clone(),
                source,
            },
        )
    }
}
