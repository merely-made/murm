//! The package catalog, and where a connected radio's firmware stands against it.
//!
//! A catalog lists qualified vendor releases, fetched on demand, beside our own staged images
//! (protocol_parity D5). Loading it never fetches. The companion client reads the radio's
//! firmware version at connect; [`FirmwareCatalog::standing`] compares it with the list and
//! returns one of three states as data. Nothing installs from a standing: an install still
//! goes through the owner flow's single review and approval.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use linkboy::catalog::newest::{self, Standing};
use linkboy::{
    Availability, BoardFamily, CatalogError, CatalogPackage, Fetch, FirmwareCache, FlashPackage,
    PackageIndex, PackageListing, PartSource,
};

use super::InstallerWake;

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
    fetcher: Option<Arc<dyn Fetch>>,
}

/// One qualified release being fetched and verified off the UI thread.
///
/// The worker owns copies of the index, cache and fetcher, and calls the host's wake when it
/// ends; the face then polls [`finished`] and [`take`]s the verified package, as for an install.
///
/// [`finished`]: FirmwareDownloadWorker::finished
/// [`take`]: FirmwareDownloadWorker::take
pub struct FirmwareDownloadWorker {
    package_id: String,
    handle: Option<JoinHandle<Result<FlashPackage, String>>>,
}

impl FirmwareDownloadWorker {
    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    /// Whether the fetch and verification have ended. Never blocks.
    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The verified package or the refusal, once finished; `None` while running or after.
    pub fn take(&mut self) -> Option<Result<FlashPackage, String>> {
        if !self.finished() {
            return None;
        }
        let handle = self.handle.take()?;
        Some(
            handle
                .join()
                .unwrap_or_else(|_| Err("the download worker panicked".into())),
        )
    }
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
    pub fn with_fetcher(mut self, fetcher: Arc<dyn Fetch>) -> Self {
        self.fetcher = Some(fetcher);
        self
    }

    /// Fetch qualified releases over https (ureq on rustls), for an installed application.
    pub fn with_https(self) -> Self {
        self.with_fetcher(Arc::new(linkboy::download::HttpsFetcher::new()))
    }

    /// Whether installing `package_id` needs a download first: an on-demand package with a
    /// part not validly in the cache. Reads the cache; never fetches.
    pub fn needs_download(&self, package_id: &str) -> bool {
        self.listings
            .iter()
            .find(|listing| listing.entry.package_id == package_id)
            .is_some_and(|listing| {
                listing.availability == Availability::OnDemand
                    && listing.manifest.declared_parts().iter().any(|part| {
                        matches!(part.source(), Some(PartSource::Download(_)))
                            && !matches!(self.cache.cached(part), Ok(Some(_)))
                    })
            })
    }

    /// Fetch and verify `package_id` on a worker thread, calling `wake` when it ends. Refused
    /// without a fetcher. The result is the package [`load_package`] would return.
    ///
    /// [`load_package`]: FirmwareCatalog::load_package
    pub fn start_download(
        &self,
        package_id: &str,
        wake: InstallerWake,
    ) -> Result<FirmwareDownloadWorker, String> {
        let fetcher = self
            .fetcher
            .clone()
            .ok_or_else(|| "this catalog has no HTTPS client to download with".to_string())?;
        let (index, index_path, cache) = (
            self.index.clone(),
            self.index_path.clone(),
            self.cache.clone(),
        );
        let id = package_id.to_string();
        let handle = std::thread::Builder::new()
            .name("signalman-firmware-download".into())
            .spawn(move || {
                let result = index
                    .load_package_with(&index_path, &id, &cache, Some(fetcher.as_ref()))
                    .map_err(|error| error.to_string());
                (wake)();
                result
            })
            .map_err(|error| error.to_string())?;
        Ok(FirmwareDownloadWorker {
            package_id: package_id.to_string(),
            handle: Some(handle),
        })
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
