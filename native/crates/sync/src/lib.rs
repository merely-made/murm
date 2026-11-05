// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Live peer synchronization over host-injected endpoints.
//!
//! The caller supplies a store and an admission callback. Eidetic's
//! Stickleback owns validated history and group encryption; this crate owns
//! joining, receive-task lifetime, resync observation and transport addresses.

pub mod address_book;
pub mod advertisements;
mod group_key;
mod joined_space;
mod stream_codec;
mod stream_sync;
mod synced_space;

pub use address_book::MunimentAddressBook;
pub use group_key::GroupKeySync;
pub use joined_space::JoinedSpace;
pub use p2panda_net::sync::hooks::{AfterHandshakeOutcome, SyncHooks, SyncHooksList};
pub use p2panda_net::{Endpoint, Gossip};
pub use stickleback::JoinError;
pub use stream_sync::{
    MAX_DEADLINE, MAX_FRAME_BYTES, MAX_SESSION_BYTES, StreamSession, StreamSync, StreamSyncError,
    StreamSyncLimits, StreamSyncOutcome, processor_accept,
};
pub use synced_space::{SyncRound, SyncStatus, SyncedSpace};
