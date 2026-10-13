// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The queue policy both runtimes obey.
//!
//! C1 requires that a sender stop above a high-water mark and resume only once
//! the queue drains past a *lower* one. The two marks are the whole point: a
//! single threshold oscillates, because the queue sits at the mark and every
//! completed write re-crosses it. The gap is hysteresis, and it is why this is
//! a type rather than a pair of loose constants.
//!
//! This lives in the shared core, not in either adapter, because the browser
//! and the native host must agree on the numbers. The browser reads
//! `RTCDataChannel.bufferedAmount`; the native side reads its own outbound
//! queue depth. Different sources, same policy.

use crate::error::BackpressureError;

/// The largest outbound queue any carrier may configure, in bytes.
///
/// A ceiling on the *configuration*, not on a single write: a peer that never
/// drains must not be able to make this end buffer without bound. Chosen as
/// sixteen maximum frames, so a sender can keep a reasonable window in flight
/// without the queue becoming a memory-exhaustion surface.
pub const MAX_QUEUED_BYTES: usize = 16 * crate::MAX_FRAME_BYTES;

/// Default high-water mark: eight maximum frames.
pub const DEFAULT_HIGH_WATER_BYTES: usize = 8 * crate::MAX_FRAME_BYTES;

/// Default low-water mark: two maximum frames.
///
/// A quarter of the high mark. Resuming at, say, seven-eighths would wake the
/// sender constantly; resuming at zero would stall the link waiting for a
/// fully drained queue.
pub const DEFAULT_LOW_WATER_BYTES: usize = 2 * crate::MAX_FRAME_BYTES;

/// When a sender must pause, and when it may resume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backpressure {
    high_water_bytes: usize,
    low_water_bytes: usize,
}

impl Default for Backpressure {
    fn default() -> Self {
        Self {
            high_water_bytes: DEFAULT_HIGH_WATER_BYTES,
            low_water_bytes: DEFAULT_LOW_WATER_BYTES,
        }
    }
}

impl Backpressure {
    /// Build a policy, rejecting the shapes that do not work.
    ///
    /// `low` must be strictly below `high`: equal marks are the oscillation
    /// this type exists to prevent, and cannot be written by accident.
    pub fn new(high_water_bytes: usize, low_water_bytes: usize) -> Result<Self, BackpressureError> {
        if high_water_bytes > MAX_QUEUED_BYTES {
            return Err(BackpressureError::HighWaterTooLarge {
                requested: high_water_bytes,
                max: MAX_QUEUED_BYTES,
            });
        }
        if high_water_bytes == 0 {
            return Err(BackpressureError::HighWaterZero);
        }
        if low_water_bytes >= high_water_bytes {
            return Err(BackpressureError::MarksNotSeparated {
                high: high_water_bytes,
                low: low_water_bytes,
            });
        }
        Ok(Self {
            high_water_bytes,
            low_water_bytes,
        })
    }

    /// The mark at or above which a sender stops.
    pub const fn high_water_bytes(&self) -> usize {
        self.high_water_bytes
    }

    /// The mark at or below which a paused sender resumes.
    pub const fn low_water_bytes(&self) -> usize {
        self.low_water_bytes
    }

    /// Should a sender stop, given the bytes currently queued?
    ///
    /// Inclusive at the mark: a queue exactly at high water has reached the
    /// limit the owner configured, not merely approached it.
    pub const fn should_pause(&self, queued_bytes: usize) -> bool {
        queued_bytes >= self.high_water_bytes
    }

    /// May a paused sender resume, given the bytes currently queued?
    pub const fn should_resume(&self, queued_bytes: usize) -> bool {
        queued_bytes <= self.low_water_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_separated_and_within_the_ceiling() {
        let bp = Backpressure::default();
        assert!(bp.low_water_bytes() < bp.high_water_bytes());
        assert!(bp.high_water_bytes() <= MAX_QUEUED_BYTES);
    }

    #[test]
    fn equal_marks_are_refused() {
        // The oscillation case: pause and resume would both be true at the
        // mark, so the sender would wake on every completed write.
        assert!(Backpressure::new(1024, 1024).is_err());
    }

    #[test]
    fn an_inverted_pair_is_refused() {
        assert!(Backpressure::new(1024, 2048).is_err());
    }

    #[test]
    fn a_high_water_above_the_ceiling_is_refused() {
        assert!(Backpressure::new(MAX_QUEUED_BYTES + 1, 0).is_err());
    }

    #[test]
    fn there_is_a_band_where_a_paused_sender_stays_paused() {
        // The hysteresis itself: between the marks, neither predicate fires,
        // so a paused sender waits and a running one keeps going.
        let bp = Backpressure::new(1000, 200).expect("valid");
        let mid = 600;
        assert!(!bp.should_pause(mid));
        assert!(!bp.should_resume(mid));
    }

    #[test]
    fn the_marks_are_inclusive_on_both_sides() {
        let bp = Backpressure::new(1000, 200).expect("valid");
        assert!(bp.should_pause(1000));
        assert!(bp.should_resume(200));
        assert!(!bp.should_pause(999));
        assert!(!bp.should_resume(201));
    }
}
