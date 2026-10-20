// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! [`Comms`] — the unified comms model over a set of backend adapters.
//!
//! Holds a `Vec<Box<dyn ProtocolAdapter>>` and presents them as one inbox:
//! [`inbox`](Comms::inbox) merges every adapter's conversations (recency-sorted)
//! and surfaces, rather than hides, any backend that failed;
//! [`messages`](Comms::messages) and [`send`](Comms::send) route to the owning
//! account using the protocol and account qualifier carried by each conversation.

use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::model::{
    AccountId, Conversation, ConversationId, DeliveryStatus, Draft, Identity, Message, MessageId,
    ProtocolKind,
};

struct RegisteredAdapter {
    account_id: AccountId,
    adapter: Box<dyn ProtocolAdapter>,
}

/// One account's identity as exposed by a registered adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountIdentity {
    pub account_id: AccountId,
    pub identity: Identity,
}

/// A backend that failed during [`Comms::inbox`], paired with its account and
/// protocol so the pane can identify the failure without dropping other rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterFailure {
    pub account_id: AccountId,
    pub protocol: ProtocolKind,
    pub error: AdapterError,
}

/// The merged conversation list plus any per-backend failures. Failures are
/// reported, not swallowed: one backend being down degrades to the others rather
/// than blanking the inbox or hiding the problem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inbox {
    pub conversations: Vec<Conversation>,
    pub failures: Vec<AdapterFailure>,
}

/// The unified comms model. Construct with [`Comms::new`] and add backends with
/// [`with_adapter`](Comms::with_adapter) / [`add_adapter`](Comms::add_adapter).
#[derive(Default)]
pub struct Comms {
    adapters: Vec<RegisteredAdapter>,
}

impl Comms {
    /// An empty model with no backends.
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    /// Add a backend adapter (builder style).
    pub fn with_adapter(mut self, adapter: Box<dyn ProtocolAdapter>) -> Self {
        let account_id = AccountId::new(adapter.identity().address);
        self.add_account(account_id, adapter);
        self
    }

    /// Add a backend adapter.
    pub fn add_adapter(&mut self, adapter: Box<dyn ProtocolAdapter>) {
        let account_id = AccountId::new(adapter.identity().address);
        self.add_account(account_id, adapter);
    }

    /// Add a backend with a caller-stable account identifier. Use this when an
    /// account's identity address is not unique among its local stores.
    pub fn with_account(
        mut self,
        account_id: impl Into<AccountId>,
        adapter: Box<dyn ProtocolAdapter>,
    ) -> Self {
        self.add_account(account_id, adapter);
        self
    }

    /// Add a backend with a caller-stable account identifier.
    pub fn add_account(
        &mut self,
        account_id: impl Into<AccountId>,
        adapter: Box<dyn ProtocolAdapter>,
    ) {
        self.adapters.push(RegisteredAdapter {
            account_id: account_id.into(),
            adapter,
        });
    }

    /// This user's backend identity on each adapter, in registration order.
    pub fn identities(&self) -> Vec<Identity> {
        self.adapters
            .iter()
            .map(|registered| registered.adapter.identity())
            .collect()
    }

    /// Account-scoped identities for new-message account selection.
    pub fn account_identities(&self) -> Vec<AccountIdentity> {
        self.adapters
            .iter()
            .map(|registered| AccountIdentity {
                account_id: registered.account_id.clone(),
                identity: registered.adapter.identity(),
            })
            .collect()
    }

    /// The merged inbox: every backend's account-qualified conversations, sorted most-recent first
    /// (conversations with no timestamp sort last). A backend that errors
    /// contributes an [`AdapterFailure`] instead of its conversations.
    pub async fn inbox(&self) -> Inbox {
        let mut conversations = Vec::new();
        let mut failures = Vec::new();
        for registered in &self.adapters {
            let adapter = registered.adapter.as_ref();
            match adapter.conversations().await {
                Ok(mut found) => {
                    let expected_protocol = adapter.protocol();
                    // Keep valid rows from a mixed response, but make malformed
                    // rows visible as an account-scoped failure and discard them.
                    if found
                        .iter()
                        .any(|conversation| conversation.id.protocol != expected_protocol)
                    {
                        failures.push(AdapterFailure {
                            account_id: registered.account_id.clone(),
                            protocol: expected_protocol,
                            error: AdapterError::Backend(
                                "adapter returned a conversation for a different protocol".into(),
                            ),
                        });
                        found.retain(|conversation| conversation.id.protocol == expected_protocol);
                    }
                    for conversation in &mut found {
                        conversation.id.account_id = Some(registered.account_id.clone());
                    }
                    conversations.append(&mut found);
                },
                Err(error) => failures.push(AdapterFailure {
                    account_id: registered.account_id.clone(),
                    protocol: adapter.protocol(),
                    error,
                }),
            }
        }
        // Most recent first; `None` (no activity yet) sorts to the end.
        conversations.sort_by_key(|conversation| std::cmp::Reverse(conversation.last_activity_ms));
        Inbox {
            conversations,
            failures,
        }
    }

    /// The messages in one conversation, routed to the backend that owns it.
    /// Qualified IDs route to their account; legacy IDs route only when one
    /// adapter serves the protocol.
    pub async fn messages(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<Message>, AdapterError> {
        let registered = self.adapter_for(conversation)?;
        registered
            .adapter
            .messages(&conversation.unqualified())
            .await
    }

    /// Send a draft, routed to the backend that owns its target conversation.
    /// Errors if the draft has no target, or no adapter serves its protocol.
    pub async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        let conversation = draft.conversation.as_ref().ok_or_else(|| {
            AdapterError::Unsupported("draft has no target conversation".to_string())
        })?;
        let registered = self.adapter_for(conversation)?;
        let mut native_draft = draft.clone();
        native_draft.conversation = Some(conversation.unqualified());
        registered.adapter.send(&native_draft).await
    }

    /// Read the owning backend's delivery observation for one message.
    pub async fn delivery_status(
        &self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<DeliveryStatus, AdapterError> {
        let registered = self.adapter_for(conversation)?;
        registered
            .adapter
            .delivery_status(&conversation.unqualified(), message)
            .await
    }

    /// Resolve a qualified account exactly, or a legacy ID only when unambiguous.
    fn adapter_for(
        &self,
        conversation: &ConversationId,
    ) -> Result<&RegisteredAdapter, AdapterError> {
        let matches = self
            .adapters
            .iter()
            .filter(|registered| registered.adapter.protocol() == conversation.protocol);
        match &conversation.account_id {
            Some(account_id) => {
                let mut owned = matches.filter(|registered| &registered.account_id == account_id);
                match (owned.next(), owned.next()) {
                    (None, _) => Err(AdapterError::UnknownAccount(account_id.0.clone())),
                    (Some(only), None) => Ok(only),
                    (Some(_), Some(_)) => {
                        Err(AdapterError::AmbiguousAccount(conversation.protocol))
                    },
                }
            },
            None => {
                let mut matches = matches;
                match (matches.next(), matches.next()) {
                    (None, _) => Err(AdapterError::Unsupported(format!(
                        "no adapter registered for {:?}",
                        conversation.protocol
                    ))),
                    (Some(only), None) => Ok(only),
                    (Some(_), Some(_)) => {
                        Err(AdapterError::AmbiguousAccount(conversation.protocol))
                    },
                }
            },
        }
    }
}
