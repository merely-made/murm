//! The package catalog, and where a connected radio's firmware stands against it.
//!
//! A catalog lists qualified vendor releases, fetched on demand, beside our own staged images
//! (protocol_parity D5). Loading it never fetches. The companion client reads the radio's
//! firmware version at connect; [`FirmwareCatalog::standing`] compares it with the list and
//! returns one of three states as data. Nothing installs from a standing: an install still
//! goes through the owner flow's single review and approval.

use std::path::{Path, PathBuf};

use linkboy::catalog::newest::{self, Standing};
use linkboy::{
    BoardFamily, CatalogError, CatalogPackage, Fetch, FirmwareCache, FlashPackage, PackageIndex,
    PackageListing, PartSource,
};

/// A stock firmware vendor the companion client speaks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareVendor {
    Meshtastic,
    MeshCore,
}

impl FirmwareVendor {
    /// The `publisher` a qualified entry for this vendor names.
    pub fn publisher(self) -> &'static str {
        match self {
            Self::Meshtastic => "Meshtastic",
            Self::MeshCore => "MeshCore",
        }
    }
}

/// What a radio reported about its own firmware at connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioFirmware {
    pub vendor: FirmwareVendor,
    pub version: String,
    /// Meshtastic's `DeviceMetadata.hw_model`, when the radio sent it.
    pub hw_model: Option<u32>,
    /// MeshCore's device-info model string.
    pub model: Option<String>,
}

impl RadioFirmware {
    /// The board, where the radio's own report names it. Meshtastic's hardware models are
    /// from the vendor's per-board release manifests: 69 `HELTEC_MESH_NODE_T114`, 110
    /// `HELTEC_V4`. MeshCore's model strings are not mapped; the owner names the board.
    pub fn board(&self) -> Option<BoardFamily> {
        match (self.vendor, self.hw_model) {
            (FirmwareVendor::Meshtastic, Some(69)) => Some(BoardFamily::T114),
            (FirmwareVendor::Meshtastic, Some(110)) => Some(BoardFamily::HeltecV4),
            _ => None,
        }
    }
}

/// A listed release as the confirmation screen shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareOffer {
    pub package_id: String,
    pub publisher: String,
    pub version: String,
    /// Total bytes of every part.
    pub byte_length: u64,
    /// Each part's sha256, in write order.
    pub sha256: Vec<String>,
    /// Where each downloaded part is fetched from; empty for a package beside its manifest.
    pub download_urls: Vec<String>,
    /// The package erases all of flash before writing.
    pub full_erase: bool,
}

impl FirmwareOffer {
    fn of(listing: &PackageListing) -> Self {
        let parts = listing.manifest.declared_parts();
        Self {
            package_id: listing.entry.package_id.clone(),
            publisher: listing.manifest.publisher.clone(),
            version: listing.manifest.version.clone(),
            byte_length: parts.iter().map(|part| part.byte_length).sum(),
            sha256: parts.iter().map(|part| part.sha256.clone()).collect(),
            download_urls: parts
                .iter()
                .filter_map(|part| match part.source() {
                    Some(PartSource::Download(url)) => Some(url.to_string()),
                    _ => None,
                })
                .collect(),
            full_erase: listing.manifest.full_erase,
        }
    }
}

/// Where a radio's firmware stands against the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirmwareStanding {
    /// It runs a listed release; `newer` is offered when the list has a newer one.
    Listed {
        current: FirmwareOffer,
        newer: Option<FirmwareOffer>,
    },
    /// It runs a version the list does not have. Nothing is offered automatically.
    NotListed { reported: String },
    /// The version or board was not read, or the list cannot be ordered.
    Unknown { reason: String },
}

pub struct FirmwareCatalog {
    index_path: PathBuf,
    index: PackageIndex,
    listings: Vec<PackageListing>,
    cache: FirmwareCache,
    fetcher: Option<Box<dyn Fetch>>,
}

impl FirmwareCatalog {
    /// Load and validate the catalog, verifying every local payload. Fetches nothing.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let index_path = path.as_ref().to_path_buf();
        let index = PackageIndex::load(&index_path)?;
        let listings = index.listings(&index_path)?;
        Ok(Self {
            index_path,
            index,
            listings,
            cache: FirmwareCache::new(FirmwareCache::default_dir()),
            fetcher: None,
        })
    }

    /// Keep downloaded releases in `cache` instead of the default directory.
    pub fn with_cache(mut self, cache: FirmwareCache) -> Self {
        self.cache = cache;
        self
    }

    /// Fetch a qualified release with `fetcher` when the owner asks to install it. Without
    /// one, only a release already in the cache can be installed.
    pub fn with_fetcher(mut self, fetcher: Box<dyn Fetch>) -> Self {
        self.fetcher = Some(fetcher);
        self
    }

    pub fn packages(&self) -> &[CatalogPackage] {
        &self.index.packages
    }

    pub fn package(&self, package_id: &str) -> Option<&CatalogPackage> {
        self.index.package(package_id)
    }

    pub fn listings(&self) -> &[PackageListing] {
        &self.listings
    }

    /// The confirmation facts for one listed package.
    pub fn offer(&self, package_id: &str) -> Option<FirmwareOffer> {
        self.listings
            .iter()
            .find(|listing| listing.entry.package_id == package_id)
            .map(FirmwareOffer::of)
    }

    /// Load a package for the owner flow, fetching a qualified release if it is not cached and
    /// this catalog has a fetcher. Every byte verifies against the tracked manifest.
    pub fn load_package(&self, package_id: &str) -> Result<FlashPackage, CatalogError> {
        self.index.load_package_with(
            &self.index_path,
            package_id,
            &self.cache,
            self.fetcher.as_deref(),
        )
    }

    /// Compare a radio's reported firmware with the list. `board` overrides the board the
    /// radio's own report names.
    pub fn standing(
        &self,
        radio: Option<&RadioFirmware>,
        board: Option<BoardFamily>,
    ) -> FirmwareStanding {
        let Some(radio) = radio else {
            return FirmwareStanding::Unknown {
                reason: "the radio's firmware version was not read".into(),
            };
        };
        let board = board.or_else(|| radio.board());
        match newest::standing(
            &self.listings,
            radio.vendor.publisher(),
            board.as_ref(),
            Some(&radio.version),
        ) {
            Standing::Listed { current, newer } => FirmwareStanding::Listed {
                current: FirmwareOffer::of(current),
                newer: newer.map(FirmwareOffer::of),
            },
            Standing::NotListed { reported } => FirmwareStanding::NotListed { reported },
            Standing::Unknown { reason } => FirmwareStanding::Unknown { reason },
        }
    }
}
