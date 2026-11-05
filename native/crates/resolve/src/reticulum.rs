// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The Reticulum adapter: holders announce `mere.release.v1.<key hex>`, and
//! the reader dials whoever announced the name.
//!
//! Destination names are fixed when a `ReticulumTransport` binds, so the
//! reader binds with [`ReticulumTransportBuilder::watch_alpns`] naming the
//! address's key (`key_alpn`) and a holder binds with `alpns` naming each log
//! it serves. A wildcard watch needs name filtering in retinue and is not
//! here.
//!
//! [`ReticulumTransportBuilder::watch_alpns`]: transport::reticulum_transport::ReticulumTransportBuilder::watch_alpns

use identity::Ed25519Keypair;
use transport::ReticulumTransport;

use crate::carrier::TransportAdapter;

/// A [`TransportAdapter`] that finds holders by announced name.
pub fn reticulum_adapter(
    transport: ReticulumTransport,
    noise_key: Ed25519Keypair,
) -> TransportAdapter<ReticulumTransport> {
    TransportAdapter::new("reticulum", transport, noise_key)
        .announced(ReticulumTransport::announced_peers)
}
