//! Which listed release is newest for a vendor and board, and where a running radio stands.
//!
//! Every listed package is on the list: a qualified vendor download, or one of our images in
//! a stage. Versions order by their numeric `MAJOR.MINOR.PATCH`. Both vendors put the build
//! commit after it (Meshtastic `2.7.26.54e0d8d`, MeshCore `v1.18.0-81dbae5`, ours
//! `0.0.1-711816a`); a commit never orders, so two listings with one triple are ambiguous and
//! neither is offered.

use std::cmp::Ordering;

use super::PackageListing;
use crate::package::BoardFamily;

/// A release version as the vendors write it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub triple: (u32, u32, u32),
    /// The build commit, 7 to 40 hex digits, when the string carries one.
    pub build: Option<String>,
}

impl ReleaseVersion {
    /// `v?MAJOR.MINOR.PATCH`, then optionally `.` or `-` and a hex commit. Nothing else.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let (numbers, build) = match text.split_once('-') {
            Some((numbers, build)) => (numbers, Some(build)),
            None => (text, None),
        };
        let mut pieces = numbers.split('.');
        let mut number = || pieces.next()?.parse::<u32>().ok();
        let triple = (number()?, number()?, number()?);
        let build = match (pieces.next(), build) {
            (None, None) => None,
            (Some(commit), None) | (None, Some(commit)) => Some(commit),
            (Some(_), Some(_)) => return None,
        };
        if pieces.next().is_some()
            || build.is_some_and(|commit| {
                !(7..=40).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return None;
        }
        Some(Self {
            triple,
            build: build.map(str::to_ascii_lowercase),
        })
    }

    /// The same release: one triple, and one commit when both name one. A short commit matches
    /// the longer one it begins.
    pub fn same_release(&self, other: &Self) -> bool {
        self.triple == other.triple
            && match (&self.build, &other.build) {
                (Some(a), Some(b)) => a.starts_with(b.as_str()) || b.starts_with(a.as_str()),
                _ => true,
            }
    }
}

/// The listings for one firmware publisher and board, with their parsed versions.
fn candidates<'a>(
    listings: &'a [PackageListing],
    publisher: &str,
    board: &BoardFamily,
) -> Vec<(&'a PackageListing, Option<ReleaseVersion>)> {
    listings
        .iter()
        .filter(|listing| {
            listing.entry.firmware_publisher == publisher
                && listing.manifest.targets.iter().any(|t| &t.family == board)
        })
        .map(|listing| (listing, ReleaseVersion::parse(&listing.manifest.version)))
        .collect()
}

/// Why no single newest release can be named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewestError {
    /// A listing's version is not one the ordering understands.
    Unordered { package_id: String, version: String },
    /// Two listings share the newest triple.
    Ambiguous { package_ids: Vec<String> },
}

/// The newest listed release for `publisher` on `board`, if any is listed.
pub fn newest<'a>(
    listings: &'a [PackageListing],
    publisher: &str,
    board: &BoardFamily,
) -> Result<Option<&'a PackageListing>, NewestError> {
    let mut best: Vec<(&PackageListing, ReleaseVersion)> = Vec::new();
    for (listing, version) in candidates(listings, publisher, board) {
        let version = version.ok_or_else(|| NewestError::Unordered {
            package_id: listing.entry.package_id.clone(),
            version: listing.manifest.version.clone(),
        })?;
        match best.first().map(|(_, top)| version.triple.cmp(&top.triple)) {
            None | Some(Ordering::Equal) => best.push((listing, version)),
            Some(Ordering::Greater) => best = vec![(listing, version)],
            Some(Ordering::Less) => {}
        }
    }
    match best.as_slice() {
        [] => Ok(None),
        [(listing, _)] => Ok(Some(listing)),
        many => Err(NewestError::Ambiguous {
            package_ids: many
                .iter()
                .map(|(listing, _)| listing.entry.package_id.clone())
                .collect(),
        }),
    }
}

/// Where a radio's running firmware stands against the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing<'a> {
    /// It runs a listed release; `newer` is the newest listed release when that is newer.
    Listed {
        current: &'a PackageListing,
        newer: Option<&'a PackageListing>,
    },
    /// It reports a version the list does not have. Nothing is offered automatically.
    NotListed { reported: String },
    /// The version or the board is not known, or the list cannot be ordered.
    Unknown { reason: String },
}

/// Compare a reported version with the listings for `publisher` on `board`.
pub fn standing<'a>(
    listings: &'a [PackageListing],
    publisher: &str,
    board: Option<&BoardFamily>,
    reported: Option<&str>,
) -> Standing<'a> {
    let unknown = |reason: String| Standing::Unknown { reason };
    let (Some(board), Some(reported)) = (board, reported) else {
        return unknown("the radio's board or firmware version was not read".into());
    };
    let Some(running) = ReleaseVersion::parse(reported) else {
        return Standing::NotListed {
            reported: reported.into(),
        };
    };
    let newest = match newest(listings, publisher, board) {
        Ok(newest) => newest,
        Err(error) => return unknown(format!("the list cannot be ordered: {error:?}")),
    };
    let current = candidates(listings, publisher, board)
        .into_iter()
        .find(|(_, version)| version.as_ref().is_some_and(|v| v.same_release(&running)));
    match current {
        Some((current, Some(version))) => Standing::Listed {
            current,
            newer: newest.filter(|top| {
                ReleaseVersion::parse(&top.manifest.version)
                    .is_some_and(|top| top.triple > version.triple)
            }),
        },
        _ => Standing::NotListed {
            reported: reported.into(),
        },
    }
}

#[cfg(test)]
mod tests;
