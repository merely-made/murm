// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, sleep_until};

use super::RelayError;

/// Owner-configured cumulative device allowance, shared across all its pools.
#[derive(Clone, Copy, Debug)]
pub struct DeviceLimits {
    /// Shared setup allowance across every pool and both connection legs.
    pub max_pending: usize,
    /// Zero disables contributions; at most 256 sessions.
    pub max_sessions: usize,
    /// Cumulative bytes across both directions and all pools for this budget.
    pub max_bytes: u64,
    /// Shared fixed-window rate ceiling; zero disables contributions.
    pub bytes_per_second: u64,
}

/// A single flow's intersected community, contribution and use limits.
#[derive(Clone, Copy, Debug)]
pub struct FlowLimits {
    /// Pool-local active-flow ceiling, intersected with the shared device slots.
    pub max_sessions: usize,
    /// Combined bytes in both directions, including forwarded crypto framing.
    pub max_bytes: u64,
    /// Maximum combined forwarding rate in a one-second accounting window.
    pub bytes_per_second: u64,
    /// One fixed buffer per direction, between 1 and 65,536 bytes.
    pub buffer_bytes: usize,
    /// Total lifetime from the reservation, including target connection setup.
    pub lifetime: Duration,
    /// Maximum time between live authority rechecks.
    pub authority_interval: Duration,
}

impl FlowLimits {
    pub(crate) fn validate(&self) -> Result<(), RelayError> {
        if self.max_sessions == 0
            || self.max_sessions > 256
            || self.max_bytes == 0
            || self.bytes_per_second == 0
            || !(1..=65_536).contains(&self.buffer_bytes)
            || self.lifetime.is_zero()
            || self.authority_interval.is_zero()
            || self.authority_interval > self.lifetime
            || Instant::now().checked_add(self.lifetime).is_none()
        {
            return Err(RelayError::InvalidLimits);
        }
        Ok(())
    }
}

/// Counter-only observations; no targets, identities or payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceStats {
    /// Setups still holding the shared service admission gate.
    pub pending_setups: usize,
    /// Reservations currently held.
    pub active: usize,
    /// Highest simultaneous reservations.
    pub peak_active: usize,
    /// Charged bytes, including bytes whose subsequent write failed.
    pub charged_bytes: u64,
}

struct Rate {
    started: Instant,
    used: u64,
}

/// Cloneable device-wide budget. Share this exact handle across its pools.
#[derive(Clone)]
pub struct DeviceBudget {
    inner: Arc<DeviceInner>,
}

struct DeviceInner {
    limits: DeviceLimits,
    slots: Arc<Semaphore>,
    used: AtomicU64,
    peak: AtomicUsize,
    rate: Mutex<Rate>,
    setup: transport::security::SetupGate,
}

impl DeviceBudget {
    /// Create an owner allowance. Cloning never resets accounting.
    pub fn new(limits: DeviceLimits) -> Result<Self, RelayError> {
        if limits.max_sessions > 256 || limits.max_pending > 256 {
            return Err(RelayError::InvalidLimits);
        }
        Ok(Self {
            inner: Arc::new(DeviceInner {
                limits,
                slots: Arc::new(Semaphore::new(limits.max_sessions)),
                used: AtomicU64::new(0),
                peak: AtomicUsize::new(0),
                rate: Mutex::new(Rate {
                    started: Instant::now(),
                    used: 0,
                }),
                setup: transport::security::SetupGate::new(limits.max_pending),
            }),
        })
    }

    pub(crate) fn setup(&self) -> transport::security::SetupGate {
        self.inner.setup.clone()
    }

    /// Read aggregate counters across every holder of the budget.
    pub fn stats(&self) -> DeviceStats {
        DeviceStats {
            pending_setups: self.inner.setup.stats().pending,
            active: self.inner.limits.max_sessions - self.inner.slots.available_permits(),
            peak_active: self.inner.peak.load(Ordering::Relaxed),
            charged_bytes: self.inner.used.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn reserve(&self, limits: FlowLimits) -> Result<Reservation, RelayError> {
        limits.validate()?;
        if self.inner.limits.bytes_per_second == 0
            || self.inner.used.load(Ordering::Relaxed) >= self.inner.limits.max_bytes
        {
            return Err(RelayError::Capacity);
        }
        let permit = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::Capacity)?;
        self.inner
            .peak
            .fetch_max(self.stats().active, Ordering::Relaxed);
        Ok(Reservation {
            device: self.clone(),
            limits,
            started: Instant::now(),
            bytes: AtomicU64::new(0),
            rate: Mutex::new(Rate {
                started: Instant::now(),
                used: 0,
            }),
            _permit: permit,
        })
    }
}

pub(crate) struct Reservation {
    device: DeviceBudget,
    pub limits: FlowLimits,
    pub started: Instant,
    bytes: AtomicU64,
    rate: Mutex<Rate>,
    _permit: OwnedSemaphorePermit,
}

fn charge(counter: &AtomicU64, amount: u64, ceiling: u64) -> Result<(), RelayError> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let next = current
            .checked_add(amount)
            .filter(|next| *next <= ceiling)
            .ok_or(RelayError::Capacity)?;
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(()),
            Err(observed) => current = observed,
        }
    }
}

fn rate_at(rate: &mut Rate, now: Instant) {
    if now.duration_since(rate.started) >= Duration::from_secs(1) {
        rate.started = now;
        rate.used = 0;
    }
}

impl Reservation {
    pub fn buffer_size(&self) -> usize {
        self.limits
            .buffer_bytes
            .min(usize::try_from(self.limits.bytes_per_second).unwrap_or(usize::MAX))
            .min(usize::try_from(self.device.inner.limits.bytes_per_second).unwrap_or(usize::MAX))
    }

    pub async fn charge(&self, amount: usize) -> Result<(), RelayError> {
        let amount = amount as u64;
        // Reserve rate under both locks atomically; never hold either over await.
        loop {
            let wait = {
                let now = Instant::now();
                let mut shared = self
                    .device
                    .inner
                    .rate
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let mut flow = self.rate.lock().unwrap_or_else(|p| p.into_inner());
                rate_at(&mut shared, now);
                rate_at(&mut flow, now);
                let shared_full = shared.used
                    > self
                        .device
                        .inner
                        .limits
                        .bytes_per_second
                        .saturating_sub(amount);
                let flow_full = flow.used > self.limits.bytes_per_second.saturating_sub(amount);
                if shared_full || flow_full {
                    Some(match (shared_full, flow_full) {
                        (true, true) => shared.started.max(flow.started) + Duration::from_secs(1),
                        (true, false) => shared.started + Duration::from_secs(1),
                        _ => flow.started + Duration::from_secs(1),
                    })
                } else {
                    shared.used += amount;
                    flow.used += amount;
                    None
                }
            };
            match wait {
                Some(deadline) => sleep_until(deadline).await,
                None => break,
            }
        }
        charge(&self.bytes, amount, self.limits.max_bytes)?;
        charge(
            &self.device.inner.used,
            amount,
            self.device.inner.limits.max_bytes,
        )
    }
}
