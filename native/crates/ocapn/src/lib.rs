// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Bounded two-party CapTP on an ordered stream admitted by its service host.
//!
//! The supported wire profile is explicitly pinned to the independent OCapN
//! test suite at 31f0b806, rather than claiming the evolving latest draft.
//! This crate owns no endpoint, router, admission policy, or domain authority.

mod host;
mod limits;
mod session;
mod syrup;
mod value;
mod wire;

pub use futures_util::future::BoxFuture;
pub use host::{Bootstrap, CallContext, HostObject, SessionAuthority};
pub use limits::Limits;
pub use session::{
    Driver, LocalPromise, RemotePromise, Resolver, SessionConfig, SessionHandle, attach,
};
pub use value::{CallError, Capability, SessionError, SessionId, Value};
pub use wire::{PeerLocator, Profile};
