// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Readable peer routes: what an endpoint ticket or a live path carries, named
//! without iroh types, so a caller can report a route without parsing tickets.

use std::net::SocketAddr;

use iroh::{EndpointAddr, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;

use crate::{PeerID, TransportError};

/// One address a peer can be reached on.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeerAddr {
    /// A direct UDP socket address.
    Direct(SocketAddr),
    /// A relay the peer is reachable through, by url.
    Relay(String),
    /// A transport this crate does not name, in iroh's own display form.
    Other(String),
}

impl std::fmt::Display for PeerAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct(addr) => write!(f, "direct {addr}"),
            Self::Relay(url) => write!(f, "relay {url}"),
            Self::Other(addr) => write!(f, "other {addr}"),
        }
    }
}

/// An address the endpoint holds for a peer, and whether it carries traffic now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerPath {
    /// Where.
    pub addr: PeerAddr,
    /// True when the endpoint is using this address right now.
    pub active: bool,
}

pub(crate) fn peer_addr(addr: &TransportAddr) -> PeerAddr {
    match addr {
        TransportAddr::Ip(addr) => PeerAddr::Direct(*addr),
        TransportAddr::Relay(url) => PeerAddr::Relay(url.to_string()),
        other => PeerAddr::Other(other.to_string()),
    }
}

/// Decode an endpoint ticket into the peer it names and the addresses it carries.
pub fn decode_peer_ticket(ticket: &str) -> Result<(PeerID, Vec<PeerAddr>), TransportError> {
    let ticket: EndpointTicket = ticket
        .trim()
        .parse()
        .map_err(|e| TransportError::Backend(format!("parse ticket: {e}")))?;
    let addr = EndpointAddr::from(ticket);
    let peer = PeerID::from_bytes(addr.id.as_bytes())
        .map_err(|e| TransportError::Backend(format!("ticket peer id: {e}")))?;
    Ok((peer, addr.addrs.iter().map(peer_addr).collect()))
}

/// Encode a peer and its direct and relay addresses as an endpoint ticket.
/// [`PeerAddr::Other`] has no encoding here and is refused.
pub fn encode_peer_ticket(peer: PeerID, addrs: &[PeerAddr]) -> Result<String, TransportError> {
    let id = iroh::PublicKey::from_bytes(&peer.to_bytes())
        .map_err(|e| TransportError::Backend(format!("peer key: {e}")))?;
    let mut parts = Vec::with_capacity(addrs.len());
    for addr in addrs {
        parts.push(match addr {
            PeerAddr::Direct(addr) => TransportAddr::Ip(*addr),
            PeerAddr::Relay(url) => TransportAddr::Relay(
                url.parse()
                    .map_err(|e| TransportError::Backend(format!("relay url {url:?}: {e}")))?,
            ),
            PeerAddr::Other(addr) => {
                return Err(TransportError::Backend(format!(
                    "cannot encode the unnamed transport address {addr:?}"
                )));
            },
        });
    }
    Ok(EndpointTicket::from(EndpointAddr::from_parts(id, parts)).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerID {
        PeerID::from_bytes(iroh::SecretKey::from_bytes(&[7; 32]).public().as_bytes()).unwrap()
    }

    #[test]
    fn a_ticket_round_trips_its_direct_and_relay_addresses() {
        let addrs = vec![
            PeerAddr::Direct("192.168.1.32:51234".parse().unwrap()),
            PeerAddr::Direct("[::1]:51234".parse().unwrap()),
            PeerAddr::Relay("https://relay.example.test./".into()),
        ];
        let ticket = encode_peer_ticket(peer(), &addrs).unwrap();
        let (named, mut decoded) = decode_peer_ticket(&ticket).unwrap();
        assert_eq!(named, peer());
        let mut expected = addrs.clone();
        expected.sort();
        decoded.sort();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn a_garbled_ticket_is_an_error_not_an_empty_route() {
        assert!(decode_peer_ticket("endpointnotaticket").is_err());
    }
}
