/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! # comms
//!
//! Graphshell domain module — a host-neutral comms model for the Mere browser.
//!
//! It names the shapes the comms pane renders ([`Conversation`], [`Message`],
//! [`Identity`], [`Draft`]) and the [`ProtocolAdapter`] seam that turns a backend
//! into them. [`Comms`] holds a set of adapters and presents them as one inbox,
//! routing per-conversation operations to the backend that owns each.
//!
//! The core (this model + the trait) is WASM-clean: no egui, no tokio runtime, no
//! platform I/O, testable without booting a host. The concrete backends live
//! behind features — `murm-adapter` and `misfin-adapter` — because they pull
//! async transports a wasm build of the bare model should not.
//!
//! ```
//! # use comms::Comms;
//! # async fn demo() {
//! // Backends are added with `.with_adapter(...)`; an empty model is a valid,
//! // quiet inbox. (The in-memory and real adapters live behind features.)
//! let comms = Comms::new();
//! let inbox = comms.inbox().await;
//! assert!(inbox.conversations.is_empty());
//! assert!(inbox.failures.is_empty());
//! # }
//! ```

#![doc(html_root_url = "https://docs.rs/comms/0.0.1")]

pub mod adapter;
pub mod comms;
pub mod model;
pub mod pane;

#[cfg(any(test, feature = "test-support"))]
pub mod in_memory;

#[cfg(feature = "murm-adapter")]
pub mod murm_adapter;

#[cfg(feature = "misfin-adapter")]
pub mod misfin_adapter;

pub use adapter::{AdapterError, ProtocolAdapter};
pub use comms::{AdapterFailure, Comms, Inbox};
pub use model::{
    Conversation, ConversationId, Direction, Draft, Identity, Message, MessageBody, MessageId,
    ProtocolKind,
};
pub use pane::{CommsPane, DockSide, DockState};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lifecycle stage marker.
pub const STAGE: &str = "pre-alpha";

#[cfg(test)]
mod tests;
