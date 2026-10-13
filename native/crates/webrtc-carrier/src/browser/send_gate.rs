// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The pause/resume decision, kept apart from `web_sys`.
//!
//! Plan §4's backpressure requirement is stateful — "stop sending at/above
//! `high_water_bytes`... resume only once it crosses `low_water_bytes`" is a
//! hysteresis, not a single comparison — so it needs somewhere to live
//! between calls. [`SendGate`] is that somewhere, and it is deliberately
//! generic over how the current queue depth is read, via
//! [`BufferedAmountSource`], rather than reaching into a
//! `web_sys::RtcDataChannel` itself. That is what makes the decision logic
//! checkable with a bare `u32` in a unit test instead of a peer connection.

use crate::Backpressure;

/// Anything that can report how many bytes are presently queued to send.
///
/// `RtcDataChannel::buffered_amount()` is the production source, read fresh
/// on every [`SendGate::may_send_now`] call — this trait never caches. A
/// bare `u32` implements it too, which is the whole point: it lets the
/// pause/resume decision be exercised with a fake buffered-amount source and
/// no `web_sys` object anywhere.
pub trait BufferedAmountSource {
    /// Bytes presently queued to send, not yet drained by the browser's own
    /// SCTP stack.
    fn buffered_amount(&self) -> u32;
}

impl BufferedAmountSource for u32 {
    fn buffered_amount(&self) -> u32 {
        *self
    }
}

/// Tracks whether a sender is presently paused, and decides when it may
/// resume.
///
/// This is [`Backpressure`]'s hysteresis made stateful.
/// [`may_send_now`](Self::may_send_now) answers "should I send right now",
/// remembering that it already said no until the low-water mark is crossed —
/// so a queue sitting exactly on the high-water mark cannot oscillate
/// between the two answers on successive calls the way a single stateless
/// comparison against [`Backpressure::should_pause`] alone would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendGate {
    policy: Backpressure,
    paused: bool,
}

impl SendGate {
    /// Starts unpaused, under `policy`.
    pub const fn new(policy: Backpressure) -> Self {
        Self {
            policy,
            paused: false,
        }
    }

    /// The policy this gate enforces.
    pub const fn policy(&self) -> Backpressure {
        self.policy
    }

    /// Whether a send attempt right now would find the gate paused.
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Reads `source` and answers whether sending may proceed right now.
    ///
    /// Call this immediately before every send attempt, never once and
    /// cached: `source` is read fresh each time, so the answer reflects
    /// whatever has drained since the last call. This is the "am I allowed
    /// to grow the queue further" half of the gate's state;
    /// [`note_buffered_amount_low`](Self::note_buffered_amount_low) is the
    /// other half, for the event that wakes a gate that was already paused.
    pub fn may_send_now(&mut self, source: &impl BufferedAmountSource) -> bool {
        let queued = source.buffered_amount() as usize;
        if self.paused {
            if self.policy.should_resume(queued) {
                self.paused = false;
            }
        } else if self.policy.should_pause(queued) {
            self.paused = true;
        }
        !self.paused
    }

    /// Called from the `bufferedamountlow` handler.
    ///
    /// The channel's low threshold is configured at
    /// [`Backpressure::low_water_bytes`], so this event firing *is* the
    /// queue crossing that mark — there is nothing left to check, unlike
    /// [`may_send_now`](Self::may_send_now), which has to re-read the
    /// current count because it can be called from an arbitrary point in
    /// time, not from the event that proves the mark was just crossed.
    pub fn note_buffered_amount_low(&mut self) {
        self.paused = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn gate() -> SendGate {
        SendGate::new(Backpressure::new(1000, 200).expect("valid"))
    }

    #[wasm_bindgen_test]
    fn stays_open_under_the_high_water_mark() {
        let mut gate = gate();
        assert!(gate.may_send_now(&500u32));
        assert!(!gate.is_paused());
    }

    #[wasm_bindgen_test]
    fn pauses_at_the_high_water_mark_and_stays_paused_in_the_band() {
        let mut gate = gate();
        assert!(!gate.may_send_now(&1000u32));
        assert!(gate.is_paused());
        // Still above the low mark: a second read must not clear it early —
        // this is the hysteresis, exercised directly.
        assert!(!gate.may_send_now(&600u32));
        assert!(gate.is_paused());
    }

    #[wasm_bindgen_test]
    fn resumes_once_a_read_finds_it_at_or_under_the_low_mark() {
        let mut gate = gate();
        assert!(!gate.may_send_now(&1000u32));
        assert!(gate.may_send_now(&200u32));
        assert!(!gate.is_paused());
    }

    #[wasm_bindgen_test]
    fn the_buffered_amount_low_event_clears_a_pause_unconditionally() {
        let mut gate = gate();
        assert!(!gate.may_send_now(&1000u32));
        gate.note_buffered_amount_low();
        assert!(!gate.is_paused());
    }

    #[wasm_bindgen_test]
    fn a_fake_source_drives_the_same_decision_a_real_channel_would() {
        // The point of `BufferedAmountSource`: no `web_sys::RtcDataChannel`
        // anywhere, and the decision is exactly the one a real channel
        // reporting the same numbers would produce.
        struct Fake(u32);
        impl BufferedAmountSource for Fake {
            fn buffered_amount(&self) -> u32 {
                self.0
            }
        }
        let mut gate = gate();
        assert!(gate.may_send_now(&Fake(0)));
        assert!(!gate.may_send_now(&Fake(1000)));
    }
}
