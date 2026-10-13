// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Conversation-owned native-drop privacy and priority mapping.
//!
//! `stickleback` owns selection mechanics. This module interprets the
//! current native conversation operation grammar without moving conversation
//! meaning into the shared store.

use std::collections::BTreeMap;

use p2panda_core::Operation;
use stickleback::{DropExportDecision, DropExportSelector};

use crate::CabalExt;

/// The caller's intended use for a conversation drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationDropProfile {
    /// Operations the receiver has not yet observed.
    CatchUp,
    /// The locally retained conversation history.
    Archive,
    /// A byte-budgeted, priority-ordered transfer over a constrained link.
    Radio,
}

/// Highest observed sequence number for each conversation author.
///
/// Murm currently has a single log per author and conversation. This frontier
/// reports replication coverage, not retention permission or human reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationFrontier {
    authors: BTreeMap<[u8; 32], u32>,
}

impl ConversationFrontier {
    /// Record the highest sequence number already held for `author`.
    pub fn observe(&mut self, author: [u8; 32], seq_num: u32) {
        self.authors
            .entry(author)
            .and_modify(|current| *current = (*current).max(seq_num))
            .or_insert(seq_num);
    }

    /// Return the highest observed sequence number for `author`.
    pub fn observed(&self, author: &[u8; 32]) -> Option<u32> {
        self.authors.get(author).copied()
    }
}

/// Privacy settings at the current signed-post granularity.
///
/// Post headers expose authorship, timing, channel, causal links, and payload
/// commitments. Message and topic bodies can be omitted independently while
/// retaining those signed facts. Profile and membership data live inside the
/// header, so their switches gate the complete operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationDropPrivacy {
    /// Carry signed header metadata at all.
    pub include_post_headers: bool,
    /// Carry message text when it is still locally retained.
    pub include_message_bodies: bool,
    /// Carry channel-topic text when it is still locally retained.
    pub include_topic_bodies: bool,
    /// Carry signed profile information stored in headers.
    pub include_profile_info: bool,
    /// Carry signed join and leave operations.
    pub include_membership: bool,
}

/// Settings-derived ordering. Higher values are selected first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationDropPriorities {
    /// Priority for deletion operations.
    pub deletion: u32,
    /// Priority for join and leave operations.
    pub membership: u32,
    /// Priority for profile-information operations.
    pub profile_info: u32,
    /// Priority for channel-topic operations.
    pub topic: u32,
    /// Priority for message operations.
    pub message: u32,
}

#[derive(Clone, Debug)]
enum ConversationDropScope {
    CatchUp(ConversationFrontier),
    Archive,
    Radio,
}

/// Concrete conversation policy for the shared native-drop exporter.
#[derive(Clone, Debug)]
pub struct ConversationDropSelector {
    profile: ConversationDropProfile,
    scope: ConversationDropScope,
    privacy: ConversationDropPrivacy,
    priorities: ConversationDropPriorities,
}

impl ConversationDropSelector {
    /// Select operations above the receiver's per-author frontier.
    pub fn catch_up(
        frontier: ConversationFrontier,
        privacy: ConversationDropPrivacy,
        priorities: ConversationDropPriorities,
    ) -> Self {
        Self {
            profile: ConversationDropProfile::CatchUp,
            scope: ConversationDropScope::CatchUp(frontier),
            privacy,
            priorities,
        }
    }

    /// Select the locally retained, privacy-permitted conversation history.
    pub fn archive(
        privacy: ConversationDropPrivacy,
        priorities: ConversationDropPriorities,
    ) -> Self {
        Self {
            profile: ConversationDropProfile::Archive,
            scope: ConversationDropScope::Archive,
            privacy,
            priorities,
        }
    }

    /// Select privacy-permitted posts in radio priority order.
    ///
    /// The caller supplies the byte budget to `stickleback` separately.
    pub fn radio(privacy: ConversationDropPrivacy, priorities: ConversationDropPriorities) -> Self {
        Self {
            profile: ConversationDropProfile::Radio,
            scope: ConversationDropScope::Radio,
            privacy,
            priorities,
        }
    }

    /// The intended use of this selector.
    pub const fn profile(&self) -> ConversationDropProfile {
        self.profile
    }

    fn in_scope(&self, operation: &Operation<CabalExt>) -> bool {
        let ConversationDropScope::CatchUp(frontier) = &self.scope else {
            return true;
        };
        frontier
            .observed(operation.header.verifying_key.as_bytes())
            .is_none_or(|seq_num| operation.header.seq_num > seq_num)
    }

    fn header(&self, priority: u32) -> DropExportDecision {
        if self.privacy.include_post_headers {
            DropExportDecision::Header { priority }
        } else {
            DropExportDecision::Omit
        }
    }

    fn body_or_header(&self, include_body: bool, priority: u32) -> DropExportDecision {
        if !self.privacy.include_post_headers {
            return DropExportDecision::Omit;
        }
        if include_body {
            DropExportDecision::Full { priority }
        } else {
            DropExportDecision::Header { priority }
        }
    }
}

impl DropExportSelector<CabalExt> for ConversationDropSelector {
    fn select(&self, operation: &Operation<CabalExt>) -> DropExportDecision {
        if !self.in_scope(operation) {
            return DropExportDecision::Omit;
        }
        match operation.header.extensions.post_type {
            0 => self.body_or_header(self.privacy.include_message_bodies, self.priorities.message),
            1 => self.header(self.priorities.deletion),
            2 if self.privacy.include_profile_info => self.header(self.priorities.profile_info),
            2 => DropExportDecision::Omit,
            3 => self.body_or_header(self.privacy.include_topic_bodies, self.priorities.topic),
            4 | 5 if self.privacy.include_membership => self.header(self.priorities.membership),
            4 | 5 => DropExportDecision::Omit,
            // Required continuity evidence for a deliberately pruned log.
            6 => self.header(self.priorities.deletion),
            _ => DropExportDecision::Omit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelName, InfoEntry, PostId, PostKind, post_to_operation, sign_post};
    use identity::{IdentityProvider, InMemoryProvider};

    const CONVERSATION: [u8; 32] = [0x43; 32];

    fn priorities() -> ConversationDropPriorities {
        ConversationDropPriorities {
            deletion: 500,
            membership: 400,
            profile_info: 300,
            topic: 200,
            message: 100,
        }
    }

    fn privacy() -> ConversationDropPrivacy {
        ConversationDropPrivacy {
            include_post_headers: true,
            include_message_bodies: true,
            include_topic_bodies: true,
            include_profile_info: true,
            include_membership: true,
        }
    }

    fn operation(seq_num: u32, backlink: Option<PostId>, kind: PostKind) -> Operation<CabalExt> {
        let keypair = InMemoryProvider::from_seed([7; 32])
            .derive_keypair(b"conversation-drop")
            .unwrap();
        post_to_operation(&sign_post(
            &keypair,
            CONVERSATION,
            seq_num,
            backlink,
            Vec::new(),
            kind,
        ))
        .unwrap()
    }

    #[test]
    fn radio_can_keep_signed_message_history_without_its_body() {
        let message = operation(
            0,
            None,
            PostKind::Text {
                channel: ChannelName::new("general"),
                text: "private body".into(),
                timestamp_ms: 10,
            },
        );
        let mut privacy = privacy();
        privacy.include_message_bodies = false;
        let selector = ConversationDropSelector::radio(privacy, priorities());
        assert_eq!(selector.profile(), ConversationDropProfile::Radio);
        assert_eq!(
            selector.select(&message),
            DropExportDecision::Header { priority: 100 }
        );
    }

    #[test]
    fn catch_up_uses_each_authors_observed_sequence() {
        let first = operation(
            0,
            None,
            PostKind::Info {
                entries: vec![InfoEntry::name("Mara")],
                timestamp_ms: 10,
            },
        );
        let first_id = PostId::new(*first.hash.as_bytes());
        let second = operation(
            1,
            Some(first_id),
            PostKind::Delete {
                posts: vec![first_id],
                timestamp_ms: 20,
            },
        );
        let mut frontier = ConversationFrontier::default();
        frontier.observe(*first.header.verifying_key.as_bytes(), 0);
        let selector = ConversationDropSelector::catch_up(frontier, privacy(), priorities());
        assert_eq!(selector.select(&first), DropExportDecision::Omit);
        assert_eq!(
            selector.select(&second),
            DropExportDecision::Header { priority: 500 }
        );
    }

    #[test]
    fn header_privacy_gates_metadata_bearing_operations() {
        let joined = operation(
            0,
            None,
            PostKind::Join {
                channel: ChannelName::new("general"),
                timestamp_ms: 10,
            },
        );
        let mut privacy = privacy();
        privacy.include_post_headers = false;
        let selector = ConversationDropSelector::archive(privacy, priorities());
        assert_eq!(selector.select(&joined), DropExportDecision::Omit);
    }
}
