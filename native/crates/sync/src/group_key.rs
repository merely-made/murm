// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::future::Future;

use muniment::Backend;
use stickleback::{
    GROUP_KEY_LANE, GroupKeyAdmission, GroupKeyExt, GroupKeyLane, GroupKeyLaneError,
    GroupSessionDispatch, JoinError, Operation, group_key_sync_topic, lane_id,
};

use crate::{Endpoint, Gossip, JoinedSpace};

/// Live networking for an Eidetic group-key lane.
pub trait GroupKeySync {
    /// Join through the lane's existing admission receiver. The host still
    /// applies accepted frames through `GroupKeyLane::drain_ready`.
    fn join(
        &self,
        endpoint: Endpoint,
        gossip: Gossip,
    ) -> impl Future<Output = Result<JoinedSpace<GroupKeyExt>, JoinError>> + Send;

    /// Store a freshly authored dispatch, then publish it. A publish failure
    /// leaves the operation stored for later reconciliation.
    fn publish(
        &self,
        joined: &JoinedSpace<GroupKeyExt>,
        dispatch: &GroupSessionDispatch,
    ) -> impl Future<Output = Result<Operation<GroupKeyExt>, GroupKeyLaneError>> + Send;
}

impl<B: Backend + Clone + Send + Sync + 'static> GroupKeySync for GroupKeyLane<B> {
    async fn join(
        &self,
        endpoint: Endpoint,
        gossip: Gossip,
    ) -> Result<JoinedSpace<GroupKeyExt>, JoinError> {
        let receiver = self.receiver();
        JoinedSpace::join::<_, u64, _, _>(
            lane_id(GROUP_KEY_LANE, self.group().0),
            self.sync_store(),
            endpoint,
            gossip,
            group_key_sync_topic(self.group()),
            move |operation: Operation<GroupKeyExt>| {
                let receiver = receiver.clone();
                async move {
                    matches!(
                        receiver.accept(&operation).await,
                        Ok(GroupKeyAdmission::Inserted { .. })
                    )
                }
            },
        )
        .await
    }

    async fn publish(
        &self,
        joined: &JoinedSpace<GroupKeyExt>,
        dispatch: &GroupSessionDispatch,
    ) -> Result<Operation<GroupKeyExt>, GroupKeyLaneError> {
        let operation = self.author_dispatch(dispatch).await?;
        joined.publish(operation.clone())?;
        Ok(operation)
    }
}
