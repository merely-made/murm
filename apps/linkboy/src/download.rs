//! Qualified vendor releases fetched on demand and verified against the tracked manifest.
//!
//! The repository pins each release's length and sha256 (protocol_parity D5); neither vendor
//! signs its files, so the pinned hash is what vouches for them, and a file replaced upstream
//! is refused. Bytes are fetched only when an owner asks to install, never at catalog load.
//!
//! A fetch is hashed while it is read, capped at the declared length, written under a
//! temporary name and renamed into the cache only after it verifies. The cache is keyed by
//! sha256, and a cached file is verified again before every use. The HTTPS client sits behind
//! [`Fetch`], so tests and embedders supply their own.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::package::{
    FlashPackage, FlashPackageManifest, PackageError, PackagePart, PartSource, sha256_hex,
};

/// One response to an https GET.
pub struct FetchResponse {
    /// The URL that answered, after any redirects. It must still be https.
    pub final_url: String,
    /// The length the server declared, when it declared one.
    pub content_length: Option<u64>,
    pub body: Box<dyn Read>,
}

/// An HTTPS client. Implementations follow redirects only to https URLs.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("{url} is not an https URL; only https downloads are fetched")]
    NotHttps { url: String },
    #[error("{url} redirected to {final_url}, which is not https")]
    RedirectNotHttps { url: String, final_url: String },
    #[error("{url} has not been downloaded, and this caller fetches nothing")]
    NotFetched { url: String },
    #[error("{url} sent more than the {expected} bytes the manifest pins")]
    Oversize { url: String, expected: u64 },
    #[error("{url} is {actual} bytes; the manifest pins {expected}")]
    LengthMismatch {
        url: String,
        expected: u64,
        actual: u64,
    },
    #[error("{url} hashes to {actual}; the manifest pins {expected}")]
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("fetching {url} failed: {source}")]
    Fetch {
        url: String,
        #[source]
        source: io::Error,
    },
    #[error("firmware cache: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Package(#[from] PackageError),
}

/// Where downloaded releases are kept: `<root>/<sha256>/<file name>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareCache {
    root: PathBuf,
}

impl FirmwareCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Signalman's data directory convention: `SIGNALMAN_FIRMWARE_CACHE`, else
    /// `%LOCALAPPDATA%\Merely\Signalman\firmware`, else the temporary directory.
    pub fn default_dir() -> PathBuf {
        if let Some(path) = std::env::var_os("SIGNALMAN_FIRMWARE_CACHE") {
            return path.into();
        }
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Merely")
            .join("Signalman")
            .join("firmware")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The cached file for a downloaded part.
    pub fn entry_path(&self, part: &PackagePart) -> Result<PathBuf, DownloadError> {
        let url = download_url(part)?;
        Ok(self
            .root
            .join(part.sha256.to_ascii_lowercase())
            .join(file_name(url)))
    }

    /// The verified cached bytes, if present. A cached file that no longer verifies is deleted.
    pub fn cached(&self, part: &PackagePart) -> Result<Option<(PathBuf, Vec<u8>)>, DownloadError> {
        let path = self.entry_path(part)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() as u64 == part.byte_length
            && sha256_hex(&bytes).eq_ignore_ascii_case(&part.sha256)
        {
            Ok(Some((path, bytes)))
        } else {
            fs::remove_file(&path)?;
            Ok(None)
        }
    }

    /// The cached bytes, or a fresh fetch when `fetcher` is given. Never fetches otherwise.
    pub fn obtain(
        &self,
        part: &PackagePart,
        fetcher: Option<&dyn Fetch>,
    ) -> Result<(PathBuf, Vec<u8>), DownloadError> {
        if let Some(found) = self.cached(part)? {
            return Ok(found);
        }
        match fetcher {
            Some(fetcher) => self.fetch(part, fetcher),
            None => Err(DownloadError::NotFetched {
                url: download_url(part)?.to_string(),
            }),
        }
    }

    /// Fetch, verify and cache one downloaded part. Nothing is left in the cache on refusal.
    pub fn fetch(
        &self,
        part: &PackagePart,
        fetcher: &dyn Fetch,
    ) -> Result<(PathBuf, Vec<u8>), DownloadError> {
        let url = download_url(part)?;
        let path = self.entry_path(part)?;
        let mut fetched = fetch_capped(fetcher, url, part.byte_length)?;
        if fetched.bytes.len() as u64 != part.byte_length {
            return Err(DownloadError::LengthMismatch {
                url: url.into(),
                expected: part.byte_length,
                actual: fetched.bytes.len() as u64,
            });
        }
        if !fetched.sha256.eq_ignore_ascii_case(&part.sha256) {
            return Err(DownloadError::HashMismatch {
                url: url.into(),
                expected: part.sha256.clone(),
                actual: fetched.sha256,
            });
        }
        let bytes = std::mem::take(&mut fetched.bytes);
        store(&path, &bytes)?;
        Ok((path, bytes))
    }

    /// Keep bytes a person qualified by hand under `<sha256 of bytes>/<url's file name>`.
    /// Nothing trusts the entry for that: every load verifies it against a manifest again.
    pub fn insert(&self, url: &str, bytes: &[u8]) -> Result<PathBuf, DownloadError> {
        if !crate::package::is_download_url(url) {
            return Err(DownloadError::NotHttps { url: url.into() });
        }
        let path = self.root.join(sha256_hex(bytes)).join(file_name(url));
        store(&path, bytes)?;
        Ok(path)
    }

    fn forget(&self, part: &PackagePart) {
        if let Ok(path) = self.entry_path(part) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Bytes read from one fetch, with their sha256, never more than `limit`.
#[derive(Debug)]
pub struct Fetched {
    pub final_url: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// GET `url` over https only, hashing while reading and refusing a body longer than `limit`.
pub fn fetch_capped(fetcher: &dyn Fetch, url: &str, limit: u64) -> Result<Fetched, DownloadError> {
    if !url.starts_with("https://") {
        return Err(DownloadError::NotHttps { url: url.into() });
    }
    let as_fetch = |source| DownloadError::Fetch {
        url: url.into(),
        source,
    };
    let response = fetcher.get(url).map_err(as_fetch)?;
    if !response.final_url.starts_with("https://") {
        return Err(DownloadError::RedirectNotHttps {
            url: url.into(),
            final_url: response.final_url,
        });
    }
    if response.content_length.is_some_and(|length| length > limit) {
        return Err(DownloadError::Oversize {
            url: url.into(),
            expected: limit,
        });
    }
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(usize::try_from(limit.min(1 << 24)).unwrap_or(0));
    let mut body = response.body.take(limit.saturating_add(1));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = body.read(&mut buffer).map_err(as_fetch)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() as u64 > limit {
            return Err(DownloadError::Oversize {
                url: url.into(),
                expected: limit,
            });
        }
    }
    Ok(Fetched {
        final_url: response.final_url,
        sha256: hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        bytes,
    })
}

/// Load a package, reading local parts beside the manifest and downloaded parts through the
/// cache. The whole package then verifies exactly as a local one does (length, sha256, layout);
/// if it does not, its downloaded parts are removed from the cache.
pub fn load_package(
    manifest_path: impl AsRef<Path>,
    cache: &FirmwareCache,
    fetcher: Option<&dyn Fetch>,
) -> Result<FlashPackage, DownloadError> {
    let manifest_path = manifest_path.as_ref();
    let manifest = FlashPackageManifest::load(manifest_path)?;
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let declared = manifest.declared_parts();
    let parts = declared
        .iter()
        .map(|part| match part.source() {
            Some(PartSource::Local(relative)) => {
                let path = parent.join(relative);
                Ok(fs::read(&path).map(|bytes| (path, bytes))?)
            }
            _ => cache.obtain(part, fetcher),
        })
        .collect::<Result<Vec<_>, _>>()?;
    FlashPackage::from_verified_parts(manifest, manifest_path, parts).map_err(|error| {
        for part in &declared {
            if matches!(part.source(), Some(PartSource::Download(_))) {
                cache.forget(part);
            }
        }
        error.into()
    })
}

fn download_url(part: &PackagePart) -> Result<&str, DownloadError> {
    match part.source() {
        Some(PartSource::Download(url)) => Ok(url),
        _ => Err(PackageError::InvalidField("part is not a download".into()).into()),
    }
}

/// A validated download URL's last segment, which validation restricts to a plain name.
fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// Write under a temporary name beside `path`, then rename into place.
fn store(path: &Path, bytes: &[u8]) -> Result<(), DownloadError> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("cache entry has no directory"))?;
    fs::create_dir_all(directory)?;
    let mut partial = path.as_os_str().to_os_string();
    partial.push(format!(".partial-{}", std::process::id()));
    let partial = PathBuf::from(partial);
    let written = fs::File::create(&partial)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| {
            if path.exists() {
                fs::remove_file(path)?;
            }
            fs::rename(&partial, path)
        });
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    Ok(written?)
}

mod https;
#[cfg(test)]
mod tests;

pub use https::{HttpsFetcher, MAX_REDIRECTS};
