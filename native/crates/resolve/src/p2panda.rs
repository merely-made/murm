// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The p2panda adapter: holders are the peers the address book tags with the
//! log's topic, dialed under native iroh security.
//!
//! The reader does not `subscribe` to the topic: joining the overlay would
//! announce which logs it reads. Discovery (or `set_topics` for explicit
//! bootstrap) tags holders; this adapter only asks who is tagged.

use murm_sync::StreamSyncOutcome;
use stickleback::edition::edition_topic;
use transport::P2pandaTransport;
use transport::security::SetupLimits;
use transport::sync_lane::SyncLane;

use crate::carrier::{Ask, Carrier, Local, PullError, Route, short};
use crate::wire::{ReadSession, release_alpn};

/// Resolve through a [`P2pandaTransport`] whose address book knows holders.
pub struct P2pandaAdapter {
    transport: P2pandaTransport,
}

impl P2pandaAdapter {
    /// An adapter over `transport`, which must have bound the release ALPN
    /// only if it also serves; a pure reader needs none.
    pub fn new(transport: P2pandaTransport) -> Self {
        Self { transport }
    }

    /// The transport, for tagging holders (`add_peer`, `set_topics`).
    pub fn transport(&self) -> &P2pandaTransport {
        &self.transport
    }
}

impl Carrier for P2pandaAdapter {
    fn name(&self) -> &'static str {
        "p2panda"
    }

    fn routes<'a>(&'a self, ask: &'a Ask<'a>) -> Local<'a, Vec<Route>> {
        Box::pin(async move {
            let topic = *edition_topic(&ask.key).as_bytes();
            let known = self
                .transport
                .peers_for_topic(topic)
                .await
                .unwrap_or_default();
            let mut routes: Vec<_> = known
                .into_iter()
                .filter(|known| known.reachable)
                .map(|known| Route {
                    label: format!("p2panda peer {}", short(&known.peer)),
                    peer: Some(known.peer),
                    expected: None,
                    addr: None,
                })
                .collect();
            // A peer hint is a route to try, not a holder claim.
            for hint in ask.hints {
                if let crate::Hint::Peer(peer) = hint
                    && !routes.iter().any(|route| route.peer == Some(*peer))
                {
                    routes.push(Route {
                        label: format!("p2panda hinted peer {}", short(peer)),
                        peer: Some(*peer),
                        expected: None,
                        addr: None,
                    });
                }
            }
            routes
        })
    }

    fn pull<'a>(
        &'a self,
        route: &'a Route,
        _ask: &'a Ask<'a>,
        session: ReadSession,
    ) -> Local<'a, Result<StreamSyncOutcome, PullError>> {
        Box::pin(async move {
            let peer = route.peer.ok_or(PullError::Unusable)?;
            let lane = SyncLane::new(release_alpn(), 4, SetupLimits::default());
            Ok(lane.dial_native(&self.transport, peer, session).await?)
        })
    }
}
