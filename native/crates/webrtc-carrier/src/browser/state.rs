// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Mapping `web_sys` state enums onto [`BrowserError`].
//!
//! Kept apart from the event-handler wiring in [`super::initiator`] because
//! the mapping itself needs nothing but the enum value — no peer connection,
//! no event, no closure — which is what makes it checkable directly, with
//! `wasm_bindgen_test` but no `RtcPeerConnection` anywhere.

use web_sys::{RtcDataChannelState, RtcPeerConnectionState};

use super::error::BrowserError;

/// What a peer-connection state transition means for the carrier, if
/// anything.
///
/// `New`, `Connecting`, and `Connected` are not failures — `None` — and are
/// left to the caller's own liveness handling rather than turned into an
/// error here. `Failed` and `Disconnected` are the two states plan §4 names
/// explicitly; `Closed` is added because a connection can close (locally or
/// remotely) without the data channel's own `onclose` having fired first,
/// and that race must not read as silence. `RtcPeerConnectionState` is
/// `#[non_exhaustive]`, so a future browser-added state falls through the
/// wildcard as "not a failure" rather than failing to compile — the honest
/// default for a state this crate does not yet know the meaning of.
pub(crate) fn connection_state_error(state: RtcPeerConnectionState) -> Option<BrowserError> {
    match state {
        RtcPeerConnectionState::Failed => Some(BrowserError::ConnectionFailed),
        RtcPeerConnectionState::Disconnected => Some(BrowserError::ConnectionDisconnected),
        RtcPeerConnectionState::Closed => Some(BrowserError::ConnectionClosed),
        _ => None,
    }
}

/// What a data-channel ready-state means for a send attempt.
///
/// `Connecting` is not yet an error — a caller awaiting
/// [`BrowserInitiator::wait_until_open`](super::BrowserInitiator::wait_until_open)
/// is exactly the honest way to wait it out. `Open` is the only state a send
/// may proceed in; `Closing` and `Closed` each become
/// [`BrowserError::NotOpen`], carrying the state's `Debug` text so the
/// message names which of the two it was, rather than a send silently
/// vanishing into a channel that will never deliver it.
pub(crate) fn channel_not_open_error(state: RtcDataChannelState) -> Option<BrowserError> {
    match state {
        RtcDataChannelState::Open => None,
        other => Some(BrowserError::NotOpen(format!("{other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn failed_and_disconnected_map_to_distinct_errors() {
        assert_eq!(
            connection_state_error(RtcPeerConnectionState::Failed),
            Some(BrowserError::ConnectionFailed)
        );
        assert_eq!(
            connection_state_error(RtcPeerConnectionState::Disconnected),
            Some(BrowserError::ConnectionDisconnected)
        );
        assert_ne!(
            connection_state_error(RtcPeerConnectionState::Failed),
            connection_state_error(RtcPeerConnectionState::Disconnected)
        );
    }

    #[wasm_bindgen_test]
    fn closed_is_distinct_from_both() {
        let closed = connection_state_error(RtcPeerConnectionState::Closed);
        assert_eq!(closed, Some(BrowserError::ConnectionClosed));
        assert_ne!(
            closed,
            connection_state_error(RtcPeerConnectionState::Failed)
        );
        assert_ne!(
            closed,
            connection_state_error(RtcPeerConnectionState::Disconnected)
        );
    }

    #[wasm_bindgen_test]
    fn live_states_are_not_errors() {
        assert_eq!(connection_state_error(RtcPeerConnectionState::New), None);
        assert_eq!(
            connection_state_error(RtcPeerConnectionState::Connecting),
            None
        );
        assert_eq!(
            connection_state_error(RtcPeerConnectionState::Connected),
            None
        );
    }

    #[wasm_bindgen_test]
    fn only_open_permits_a_send() {
        assert_eq!(channel_not_open_error(RtcDataChannelState::Open), None);
        assert!(channel_not_open_error(RtcDataChannelState::Connecting).is_some());
        assert!(channel_not_open_error(RtcDataChannelState::Closing).is_some());
        assert!(channel_not_open_error(RtcDataChannelState::Closed).is_some());
    }
}
