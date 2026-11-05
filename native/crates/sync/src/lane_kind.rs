// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Which key scheme a lane's group runs under (release-log D12).
//!
//! Conversations (posts, co-op chat, direct messages) default to forward
//! secrecy: a stolen device or leaked key opens no past message. Shared and
//! published data keeps the durable data scheme, because a late reader must be
//! able to decrypt it, but its old epoch secrets are forgotten after a bounded
//! window.

use identity::IdentityProvider;
use stickleback::{
    GroupEncryptionProfile, GroupPrekeyBundle, GroupSecretId, GroupSession, GroupSessionError,
    GroupSessionId, MessageSession,
};

/// What a lane carries, and so how its group is keyed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneKind {
    /// Posts, chat and direct messages: forward-secure message ratchets.
    #[default]
    Conversation,
    /// Sealed releases, moot collections and shares: durable epochs with a
    /// bounded retention window.
    Durable,
}

impl LaneKind {
    pub const fn profile(self) -> GroupEncryptionProfile {
        match self {
            Self::Conversation => GroupEncryptionProfile::forward_secure_messages(),
            Self::Durable => GroupEncryptionProfile::bounded_durable(),
        }
    }

    /// Open the local member's session for `group` under this kind's scheme.
    ///
    /// # Errors
    /// The identity provider or the key engine refused.
    pub fn open_session(
        self,
        group: GroupSessionId,
        identity: &dyn IdentityProvider,
    ) -> Result<LaneSession, GroupSessionError> {
        Ok(match self {
            Self::Conversation => LaneSession::Conversation(MessageSession::new(group, identity)?),
            Self::Durable => {
                let (session, prekey) = GroupSession::new(group, identity)?;
                LaneSession::Durable { session, prekey }
            }
        })
    }
}

/// A member's session under the scheme its lane kind selected.
pub enum LaneSession {
    Conversation(MessageSession),
    Durable {
        session: GroupSession,
        /// Publish this so others can add the member.
        prekey: GroupPrekeyBundle,
    },
}

impl LaneSession {
    pub fn kind(&self) -> LaneKind {
        match self {
            Self::Conversation(_) => LaneKind::Conversation,
            Self::Durable { .. } => LaneKind::Durable,
        }
    }

    /// Forget durable epochs that have left the retention window. A
    /// conversation has none to forget.
    ///
    /// # Errors
    /// The session could not commit the forgetting.
    pub fn enforce_retention(&mut self) -> Result<Vec<GroupSecretId>, GroupSessionError> {
        let profile = self.kind().profile();
        match self {
            Self::Conversation(_) => Ok(Vec::new()),
            Self::Durable { session, .. } => session.enforce_retention(profile),
        }
    }
}

#[cfg(test)]
mod tests {
    use identity::InMemoryProvider;
    use stickleback::GroupEncryptionMode;

    use super::*;

    #[test]
    fn conversations_are_forward_secure_unless_a_lane_asks_for_durable() {
        assert_eq!(LaneKind::default(), LaneKind::Conversation);
        assert_eq!(
            LaneKind::default().profile(),
            GroupEncryptionProfile::forward_secure_messages()
        );
        let durable = LaneKind::Durable.profile();
        assert_eq!(durable.mode, GroupEncryptionMode::Data);
        assert!(durable.retained_data_epochs > 0);

        let identity = InMemoryProvider::from_seed([7; 32]);
        let group = GroupSessionId([9; 32]);
        let mut conversation = LaneKind::default().open_session(group, &identity).unwrap();
        assert!(matches!(conversation, LaneSession::Conversation(_)));
        assert!(conversation.enforce_retention().unwrap().is_empty());

        let mut shared = LaneKind::Durable.open_session(group, &identity).unwrap();
        assert!(matches!(shared, LaneSession::Durable { .. }));
        assert_eq!(shared.kind(), LaneKind::Durable);
        assert!(shared.enforce_retention().unwrap().is_empty());
    }
}
