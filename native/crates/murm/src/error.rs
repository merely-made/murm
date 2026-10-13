// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Error types for `murm`.

use thiserror::Error;

/// Errors raised by murm operations.
#[derive(Debug, Error)]
pub enum MurmError {
    /// An identity-layer error.
    #[error("identity error: {0}")]
    Identity(#[from] identity::IdentityError),

    /// A transport-layer error.
    #[error("transport error: {0}")]
    Transport(#[from] transport::TransportError),

    /// A post's wire representation is malformed.
    #[error("malformed post wire encoding")]
    MalformedPost,

    /// A required post field is missing or invalid.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// Shared conversation admission or storage failed.
    #[error(transparent)]
    ConversationStore(#[from] crate::ConversationStoreError),

    /// Retained causal metadata cannot form a legal projection.
    #[error(transparent)]
    Causality(#[from] stickleback::CausalError),

    /// The local author's retained tail is blocked on unavailable history.
    #[error("local author head is causally incomplete")]
    IncompleteAuthorHead,

    /// A key epoch was missing or installed out of order.
    #[error(transparent)]
    Keyring(#[from] crate::CabalKeyringError),

    /// The requested cabal is not known to this Murm instance.
    #[error("cabal not found")]
    CabalNotFound,

    /// Backend-specific error.
    #[error("backend error: {0}")]
    Backend(String),
}
