// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Errors the browser initiator surfaces to its Rust caller.
//!
//! Every terminal condition funnels through exactly one of these variants —
//! never a bare `JsValue`, never a silent drop. `onclose`, `onerror`, and a
//! peer-connection state change to `failed`/`disconnected`/`closed` each
//! produce a distinct, matchable variant, which is plan §4's explicit
//! requirement: a caller must be able to tell these apart, not just learn
//! that "something happened."

use wasm_bindgen::{JsCast, JsValue};

use crate::{BackpressureError, FrameError};

/// Something the browser initiator could not do, or was told went wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    /// The data channel's `onclose` fired.
    #[error("the data channel closed")]
    ChannelClosed,

    /// The data channel's `onerror` fired.
    ///
    /// `web_sys` types `RTCDataChannel.onerror`'s event as a plain `Event`,
    /// not the newer `RTCErrorEvent`, so no JS-supplied message travels with
    /// it here. The variant still exists on its own, distinct from
    /// [`ChannelClosed`](Self::ChannelClosed) and the connection-state
    /// variants below, so a caller can match "the channel itself reported an
    /// error" apart from "the channel closed" or "the connection died."
    #[error("the data channel reported an error")]
    ChannelError,

    /// `RTCPeerConnection.connectionState` became `failed`.
    #[error("the peer connection reached the `failed` state")]
    ConnectionFailed,

    /// `RTCPeerConnection.connectionState` became `disconnected`.
    #[error("the peer connection reached the `disconnected` state")]
    ConnectionDisconnected,

    /// `RTCPeerConnection.connectionState` became `closed`.
    ///
    /// Distinct from [`ChannelClosed`](Self::ChannelClosed): a connection
    /// can close — locally, via [`BrowserInitiator::close`][close], or
    /// because the remote end closed it — without the data channel's own
    /// `onclose` having fired first, and that race must not read as
    /// silence.
    ///
    /// [close]: super::BrowserInitiator::close
    #[error("the peer connection closed")]
    ConnectionClosed,

    /// A send was attempted while the channel's `readyState` was not
    /// `open`.
    #[error("cannot send: data channel is not open (state: {0})")]
    NotOpen(String),

    /// A received message was not an `ArrayBuffer`.
    ///
    /// [`BrowserInitiator::new`](super::BrowserInitiator::new) sets
    /// `binary_type` to `arraybuffer`, so this means the two ends disagree
    /// about framing at a level below this crate's own frames — not
    /// something a caller can recover from mid-connection.
    #[error("received a non-ArrayBuffer message")]
    UnexpectedMessageType,

    /// A frame this crate's core rejected — most importantly, an oversize
    /// declared length caught before allocation
    /// ([`FrameAssembler`](super::FrameAssembler)).
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// A [`Backpressure`](crate::Backpressure) policy the caller supplied
    /// could not be built.
    #[error(transparent)]
    Backpressure(#[from] BackpressureError),

    /// A JavaScript call threw, or a promise it returned rejected.
    #[error("browser WebRTC call failed: {0}")]
    Js(String),
}

impl From<JsValue> for BrowserError {
    fn from(value: JsValue) -> Self {
        Self::Js(js_value_to_string(&value))
    }
}

/// Best-effort human-readable text for a thrown or rejected `JsValue`.
///
/// A JS `Error` (what `RTCPeerConnection`'s promises reject with) carries its
/// message on a `message` property that plain `JsValue::as_string` cannot
/// see, since the value itself is not a string. `dyn_ref::<js_sys::Error>`
/// is the strict path — for anything else, `{value:?}` at least names the
/// value's type rather than losing it.
fn js_value_to_string(value: &JsValue) -> String {
    if let Some(text) = value.as_string() {
        return text;
    }
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    format!("{value:?}")
}
