// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! What the resolver needs from a release address, and nothing of its grammar.
//!
//! The `knot://` grammar belongs to Gemot (`gemot-address`), which implements
//! [`ReleaseAddress`] for its parsed address (`gemot-site-host`, feature
//! `murm-resolve`). The resolver reads four things from an address: its host,
//! the governed form's collection, the pinned edition and the path.
//! [`LogAddress`] is the plain form a caller that already holds a log key can
//! build without any parser.

/// What the address host names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressHost {
    /// A release-log key, of any tier.
    LogKey([u8; 32]),
    /// A Moot id: the governed form. The collection is the path's first segment.
    MootId([u8; 32]),
}

impl AddressHost {
    /// The 32 key or anchor bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8; 32] {
        match self {
            Self::LogKey(bytes) | Self::MootId(bytes) => bytes,
        }
    }
}

/// The parts of a governed address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MootCollection {
    pub moot_id: [u8; 32],
    pub collection_id: [u8; 32],
    /// The page or file inside the collection's site, possibly empty (the root).
    pub page: String,
}

/// An address the resolver can act on.
pub trait ReleaseAddress {
    /// The host: a log key to resolve, or a Moot id to hand back.
    fn host(&self) -> AddressHost;

    /// The governed form's parts. `Some` exactly when [`host`](Self::host) is
    /// [`AddressHost::MootId`].
    fn collection(&self) -> Option<MootCollection>;

    /// The edition id the address pins, when it names one.
    fn pin(&self) -> Option<[u8; 32]>;

    /// The page or file path inside the edition, without the leading slash.
    fn path(&self) -> &str;
}

/// A release address for a known log key: the key, a path and an optional pin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogAddress {
    pub key: [u8; 32],
    pub path: String,
    pub pin: Option<[u8; 32]>,
}

impl LogAddress {
    /// An address for `key` with `path` and no pin.
    #[must_use]
    pub fn new(key: [u8; 32], path: impl Into<String>) -> Self {
        Self {
            key,
            path: path.into(),
            pin: None,
        }
    }

    /// Pin an edition id the chain must contain.
    #[must_use]
    pub fn pinned(mut self, edition: [u8; 32]) -> Self {
        self.pin = Some(edition);
        self
    }
}

impl ReleaseAddress for LogAddress {
    fn host(&self) -> AddressHost {
        AddressHost::LogKey(self.key)
    }

    fn collection(&self) -> Option<MootCollection> {
        None
    }

    fn pin(&self) -> Option<[u8; 32]> {
        self.pin
    }

    fn path(&self) -> &str {
        &self.path
    }
}
