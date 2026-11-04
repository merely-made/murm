// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Bounded application stream relays over existing admitted Murm transports.
//!
//! Hosts inject live authorization, registered routes and a shared device
//! budget. This module owns neither membership nor group keys. It makes no
//! host-location privacy claim: the I2P acceptance gate remains separate.

mod budget;
mod pump;
mod session;

pub use budget::{DeviceBudget, DeviceLimits, DeviceStats, FlowLimits};
pub use session::{RelayChannel, RelayGrant, RelayRequest, RelayService, RelayTarget};

/// Protocol registered by a host for this application stream service.
pub const RELAY_PROTOCOL: &[u8] = b"murm/relay/stream/v1";
/// Domain-separated protocol for the end-to-end recipient session.
pub const RECIPIENT_PROTOCOL: &[u8] = b"murm/relay/recipient/v1";

/// A refused or interrupted relay operation. Contains no private route/identity.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    /// Invalid owner limits.
    #[error("invalid relay limits")]
    InvalidLimits,
    /// Device/session/byte allowance is exhausted.
    #[error("relay capacity exhausted")]
    Capacity,
    /// Current authority, target or lease does not allow forwarding.
    #[error("relay authority unavailable or expired")]
    Unauthorized,
    /// Carrier, security or admission failed. Details remain with that owner.
    #[error("relay session setup failed")]
    Setup,
    /// An established stream ended unexpectedly or failed.
    #[error("relay stream failed")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests;
