// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The browser half of C1: an initiator over the browser's own WebRTC.
//!
//! `mere-webrtc-carrier`'s default core (the crate root) computes over a
//! transcript and frames bytes; it opens no socket and creates no peer
//! connection anywhere, on purpose (see the crate root doc). This module is
//! the adapter that does: [`BrowserInitiator`] drives a real
//! `web_sys::RtcPeerConnection` — the browser's own WebRTC stack, not a
//! Rust one compiled into Wasm — to open one ordered, reliable data channel
//! and carry the core's frames over it.
//!
//! Gated on `all(feature = "browser", target_arch = "wasm32")` at the crate
//! root: everything in this tree needs `web_sys`, which is only ever a
//! dependency of this crate on `wasm32` (see `Cargo.toml`), so there is no
//! partial-compilation state where this module exists but `web_sys` does
//! not.
//!
//! ## Layout
//!
//! - [`send_gate`] — the pause/resume decision, kept apart from `web_sys` so
//!   it is checkable against a fake buffered-amount source.
//! - [`frame_assembler`] — reassembling whole frames out of an `onmessage`
//!   byte stream whose chunk boundaries need not match frame boundaries.
//! - [`state`] — mapping `web_sys`'s state enums onto [`BrowserError`].
//! - [`error`] — [`BrowserError`] itself.
//! - [`initiator`] — [`BrowserInitiator`], which wires the three above to a
//!   real `RtcPeerConnection`/`RtcDataChannel` pair.
//!
//! The first three are unit-tested directly with `wasm_bindgen_test`,
//! without a peer connection anywhere in sight. `initiator` is not: a real
//! two-peer exchange is proven later by a headed receipt (plan §4), not by
//! a unit test here.

mod error;
mod frame_assembler;
mod initiator;
mod send_gate;
mod state;

pub use error::BrowserError;
pub use frame_assembler::FrameAssembler;
pub use initiator::{
    BrowserInitiator, BrowserInitiatorConfig, IceCandidate, IceServer, IceTransportPolicy,
};
pub use send_gate::{BufferedAmountSource, SendGate};
