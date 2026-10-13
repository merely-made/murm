// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Persistent, bounded JSON control of one retained-object producer and collector.
//!
//! This process owns temporary transfer state only. Objects remain opaque;
//! their digest is not an application operation ID, and recovery is not store
//! admission. Logical time is supplied by the caller and never goes backwards.
//! Wall-clock deadlines bound the process independently of that logical clock.

mod actor;
mod format;
mod protocol;
mod provenance;
mod stdio;

pub use protocol::{MAX_LINE_BYTES, MAX_REQUESTS, MAX_TOTAL_BYTES, SCHEMA};
pub use stdio::{IDLE_TIMEOUT, LIFETIME, REQUEST_TIMEOUT, run_stdio};

#[cfg(test)]
mod tests;
