// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Carrier-observed facts for the initiator of an outbound session.

use crate::{Alpn, PeerID};

/// An outbound stream and the carrier facts its initiator can honestly bind.
///
/// `authenticated_initiator` names the local endpoint only when the carrier
/// proves that identity to the responder. A local routing key alone does not
/// establish this: Reticulum reports no initiator identity and supplies its
/// real shared link instead. Unknown adapters leave both facts absent.
#[derive(Debug)]
pub struct OutboundSession<S> {
    /// The bidirectional stream.
    pub stream: S,
    /// The protocol the carrier opened.
    pub protocol: Alpn,
    /// The initiator identity the carrier proves to the responder, if any.
    pub authenticated_initiator: Option<PeerID>,
    /// The real link identifier shared by both ends, if available.
    pub shared_link: Option<[u8; 16]>,
}

impl<S> OutboundSession<S> {
    /// Build an outbound record from carrier-observed facts.
    pub fn new(
        stream: S,
        protocol: Alpn,
        authenticated_initiator: Option<PeerID>,
        shared_link: Option<[u8; 16]>,
    ) -> Self {
        Self {
            stream,
            protocol,
            authenticated_initiator,
            shared_link,
        }
    }

    /// Consume the record when the caller does not need its binding facts.
    pub fn into_stream(self) -> S {
        self.stream
    }
}
