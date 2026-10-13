// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Open connections per remote, counted from iroh's handshake hook (pairing
//! plan rulings 47 to 51).
//!
//! iroh reports no per-remote connection count, and its path state stays
//! Active for a minute or more after a peer dies. Every connection on the
//! endpoint passes `after_handshake`, whoever opened it, and its weak handle
//! reports the close.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks};

/// Shared by its clones. Holds counts only: never the endpoint, never a strong
/// connection handle.
#[derive(Clone, Debug, Default)]
pub(super) struct OpenConnections {
    counts: Arc<Mutex<HashMap<[u8; 32], usize>>>,
}

impl OpenConnections {
    /// Connections to `peer` open on this endpoint now.
    pub(super) fn count(&self, peer: &[u8; 32]) -> usize {
        self.counts.lock().unwrap().get(peer).copied().unwrap_or(0)
    }

    fn opened(&self, peer: [u8; 32]) {
        *self.counts.lock().unwrap().entry(peer).or_default() += 1;
    }

    fn closed(&self, peer: &[u8; 32]) {
        let mut counts = self.counts.lock().unwrap();
        if let Some(open) = counts.get_mut(peer) {
            *open -= 1;
            if *open == 0 {
                counts.remove(peer);
            }
        }
    }
}

impl EndpointHooks for OpenConnections {
    fn after_handshake<'a>(
        &'a self,
        conn: &'a Connection,
    ) -> impl Future<Output = AfterHandshakeOutcome> + Send + 'a {
        let peer = *conn.remote_id().as_bytes();
        self.opened(peer);
        // Taken while `conn` is held, so the close is always delivered; the
        // future does not keep the connection alive.
        let closed = conn.weak_handle().closed();
        let counts = self.clone();
        tokio::spawn(async move {
            closed.await;
            counts.closed(&peer);
        });
        async { AfterHandshakeOutcome::accept() }
    }
}
