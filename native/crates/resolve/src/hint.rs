// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Provider hints: where a holder might be, never who to believe.
//!
//! A hint comes from a link, an edition body or a contact. It can only add a
//! route to try. Whatever it points at must serve the address's log or it is
//! refused, so a hint needs no trust and carries no authority.

use std::net::SocketAddr;

use transport::PeerID;

/// A parsed hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// `peer:<64-hex>`: a peer id reachable on a peer-addressed carrier.
    Peer(PeerID),
    /// `noise+tcp://<ip:port>/<64-hex peer>`: a Noise-over-TCP holder and the
    /// key it should prove.
    NoiseTcp { addr: SocketAddr, peer: PeerID },
    /// `i2p:<destination>`: an I2P destination.
    I2p(String),
    /// `onion:<host>`: an onion service host.
    Onion(String),
}

/// Why a hint string is not usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unrecognized hint")]
pub struct HintError;

fn peer_from_hex(text: &str) -> Option<PeerID> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (slot, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    PeerID::from_bytes(&bytes).ok()
}

impl Hint {
    /// Parse one hint string.
    ///
    /// # Errors
    /// Fails on an unknown scheme or a malformed address or key.
    pub fn parse(text: &str) -> Result<Self, HintError> {
        if let Some(hex) = text.strip_prefix("peer:") {
            return peer_from_hex(hex).map(Self::Peer).ok_or(HintError);
        }
        if let Some(rest) = text.strip_prefix("noise+tcp://") {
            let (addr, peer) = rest.split_once('/').ok_or(HintError)?;
            return Ok(Self::NoiseTcp {
                addr: addr.parse().map_err(|_| HintError)?,
                peer: peer_from_hex(peer).ok_or(HintError)?,
            });
        }
        match (text.strip_prefix("i2p:"), text.strip_prefix("onion:")) {
            (Some(name), _) if !name.is_empty() => Ok(Self::I2p(name.to_string())),
            (_, Some(name)) if !name.is_empty() => Ok(Self::Onion(name.to_string())),
            _ => Err(HintError),
        }
    }
}
