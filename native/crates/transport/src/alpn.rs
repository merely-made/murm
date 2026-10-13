// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! ALPN identifier types.

/// An ALPN identifier — the protocol identifier sent during transport
/// handshake to select between concurrent protocols on the same underlying
/// peer connection.
///
/// Mere uses a hierarchical naming convention:
///
/// - `mere/cable/v1` — Cable bilateral chat
/// - `mere/coop/v1` — co-op session presence
/// - `mere/mls/v1` — future MLS bilateral E2EE chat
/// - `mere/community/v1` — future gemot federation
///
/// ## Why a wrapper type?
///
/// Distinguishes ALPN identifiers from arbitrary byte sequences in API
/// signatures. Prevents mistakes like passing message-content where an ALPN
/// is expected.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Alpn(Vec<u8>);

impl Alpn {
    /// Construct an ALPN from a string slice (the typical case).
    ///
    /// This is intentionally not named `from_str` to avoid colliding with
    /// the [`std::str::FromStr`] trait — ALPN construction from a string is
    /// infallible (any string forms a syntactically-valid ALPN; semantic
    /// validation, if any, happens elsewhere), so a `Result`-returning
    /// `FromStr` impl would be misleading.
    pub fn new(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }

    /// Construct an ALPN from raw bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// View the ALPN as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the ALPN is empty (typically invalid).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<[u8]> for Alpn {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
