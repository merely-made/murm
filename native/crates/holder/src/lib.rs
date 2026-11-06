// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Release-log holders: acquire, retain, acknowledge, select.
//!
//! A holder keeps verified editions of logs it agreed to hold (`node`),
//! pulling them over any carrier `murm-resolve` can dial, and serves them
//! back on the same protocol. It signs custody statements (`ack`) that a
//! publisher checks (`replication`, `dial`); until every configured holder
//! has, publishing reads "pending". A stack setting (`pool`) picks among the
//! personal mesh and named pools and shows each route's privacy tier.
//!
//! Publisher devices serve only the release protocol to holders, over a
//! carrier. They never serve HTTP. Carriers are opt-in features.

mod ack;
mod dial;
mod node;
mod pool;
mod replication;
mod service;

pub use ack::{ACK_LEN, Ack, AckError};
pub use dial::{AckSession, Dial, DialError, HolderTarget, NoiseTcpDialer, Reach, TransportDialer};
pub use node::{Acquired, HolderError, HolderNode, Retention};
pub use pool::{
    CarrierClass, DataVisible, Plan, Planned, Pool, PoolId, PoolRoute, PoolSetting, PrivacyTier,
    Refused, Resolution,
};
pub use replication::{AckSink, Replication, ReplicationStatus, Round, RoundError, request_acks};
pub use service::HolderService;
