// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Native conversation storage over the shared replication substrate.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, RwLock};

use muniment::{Backend, MemoryBackend, RedbBackend, StoreError};
use p2panda_core::{Operation, Topic};
use proofs::Digest;
use stickleback::{
    Admission, CausalEntry, CausalLimits, CheckpointAuthority, DropExportProfile, DropImportReport,
    DropIoError, DropLimits, DropProtector, MunimentStore, OperationPolicy, OperationProcessor,
    ProcessError, ProcessOutcome, Reject, StoreTarget, decode_operation_record,
    export_topic_operations, import_plain_drop, import_protected_drop, validate_causal_metadata,
};

use crate::{CabalExt, ChannelName};

/// Failure while opening or processing a native conversation store.
#[derive(Debug, thiserror::Error)]
pub enum ConversationStoreError {
    /// Host filesystem or backend selection failed before the store opened.
    #[error("conversation backend: {0}")]
    Backend(String),
    /// The shared backend could not be opened or read.
    #[error("conversation store: {0}")]
    Store(#[from] StoreError),
    /// Structural, domain, continuity, or write processing failed.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// Native-drop decoding, staging, or import failed.
    #[error(transparent)]
    Drop(#[from] DropIoError),
}

/// Owner-supplied authority for pruning an author's own cabal log.
///
/// The host resolves this from its accepted policy; holding a cabal key, joining
/// a channel, or carrying an operation never supplies this permission. Reopened
/// stores need the corresponding authority installed before reading a pruned
/// root or admitting a new checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CabalCheckpointAuthority {
    revision: Digest,
    signers: BTreeSet<[u8; 32]>,
}

impl CabalCheckpointAuthority {
    /// Bind the exact accepted policy revision to its permitted signers.
    pub fn new(revision: Digest, signers: impl IntoIterator<Item = [u8; 32]>) -> Self {
        Self {
            revision,
            signers: signers.into_iter().collect(),
        }
    }
}

impl CheckpointAuthority for CabalCheckpointAuthority {
    fn authority_revision(&self) -> Digest {
        self.revision.clone()
    }

    fn permits_checkpoint(&self, author: [u8; 32], named_revision: &Digest) -> bool {
        *named_revision == self.revision && self.signers.contains(&author)
    }
}

#[derive(Default)]
struct RetentionAuthorities {
    current: Option<CabalCheckpointAuthority>,
    // Historical revisions justify retained roots, never new ingress. The
    // policy owner reinstalls accepted history after a durable reopen.
    history: HashMap<Digest, CabalCheckpointAuthority>,
}

#[derive(Clone)]
struct ConversationPolicy {
    conversation_id: [u8; 32],
    checkpoint_authority: Option<CabalCheckpointAuthority>,
}

impl ConversationPolicy {
    fn reject(detail: impl Into<String>) -> Reject {
        Reject::new("invalid-conversation-operation", detail)
    }

    fn validate_grammar(&self, operation: &Operation<CabalExt>) -> Result<(), Reject> {
        let ext = &operation.header.extensions;
        validate_causal_metadata(operation, &ext.parents, CausalLimits::default())
            .map_err(|error| Self::reject(error.to_string()))?;
        if ext.post_type != 6 && (ext.prune_flag.is_set() || ext.authority_revision.is_some()) {
            return Err(Self::reject("ordinary post carries checkpoint fields"));
        }
        let body = operation.body.as_ref().map(|body| body.to_bytes());
        let requires_channel = matches!(ext.post_type, 0 | 3 | 4 | 5);
        if requires_channel && !ChannelName::is_valid_name(&ext.channel) {
            return Err(Self::reject("operation has an invalid channel"));
        }
        if !requires_channel && !ext.channel.is_empty() {
            return Err(Self::reject(
                "channel-less operation carries a channel name",
            ));
        }
        match ext.post_type {
            0 | 3 => {
                if !ext.info.is_empty() || !ext.deletes.is_empty() {
                    return Err(Self::reject(
                        "content operation carries unrelated header fields",
                    ));
                }
                if body
                    .as_ref()
                    .is_some_and(|bytes| std::str::from_utf8(bytes).is_err())
                {
                    return Err(Self::reject("conversation text is not UTF-8"));
                }
            },
            1 => {
                if !ext.info.is_empty() {
                    return Err(Self::reject("deletion operation carries profile data"));
                }
                if operation.header.payload_hash.is_some() || operation.header.payload_size != 0 {
                    return Err(Self::reject("deletion operation carries a body"));
                }
            },
            2 => {
                if !ext.deletes.is_empty() {
                    return Err(Self::reject("profile operation carries deletion targets"));
                }
                if operation.header.payload_hash.is_some() || operation.header.payload_size != 0 {
                    return Err(Self::reject("profile operation carries a body"));
                }
            },
            4 | 5 => {
                if !ext.info.is_empty() || !ext.deletes.is_empty() {
                    return Err(Self::reject(
                        "membership operation carries unrelated header fields",
                    ));
                }
                if operation.header.payload_hash.is_some() || operation.header.payload_size != 0 {
                    return Err(Self::reject("membership operation carries a body"));
                }
            },
            6 => {
                if !ext.prune_flag.is_set()
                    || ext.authority_revision.is_none()
                    || !ext.info.is_empty()
                    || !ext.deletes.is_empty()
                    || !ext.parents.is_empty()
                    || operation.header.payload_hash.is_some()
                    || operation.header.payload_size != 0
                {
                    return Err(Self::reject("invalid retention checkpoint grammar"));
                }
            },
            _ => return Err(Self::reject("unknown conversation post type")),
        }
        Ok(())
    }
}

impl OperationPolicy<CabalExt> for ConversationPolicy {
    type LogId = u64;

    fn admit(&self, operation: &Operation<CabalExt>) -> Result<Admission<Self::LogId>, Reject> {
        if operation.header.extensions.cabal_id != self.conversation_id {
            return Err(Reject::new(
                "wrong-conversation",
                "operation addresses a different conversation",
            ));
        }
        self.validate_grammar(operation)?;
        let target = StoreTarget::new(Topic::from(self.conversation_id), 0);
        if operation.header.extensions.prune_flag.is_set() {
            let revision = operation
                .header
                .extensions
                .authority_revision
                .as_ref()
                .unwrap();
            if !self.checkpoint_authority.as_ref().is_some_and(|authority| {
                authority.permits_checkpoint(*operation.header.verifying_key.as_bytes(), revision)
            }) {
                return Err(Reject::new(
                    "unauthorized-retention-checkpoint",
                    "checkpoint signer or authority revision is not permitted",
                ));
            }
            Ok(Admission::prune_before_current(target))
        } else {
            Ok(Admission::keep(target))
        }
    }
}

/// One native conversation's shared operation store.
///
/// This replaces `PersistentCabalStore` in Murm's live `ConversationEngine`.
/// The same handle serves admission, LogSync, and native-drop export.
#[derive(Clone)]
pub struct ConversationStore<B> {
    conversation_id: [u8; 32],
    store: MunimentStore<B, CabalExt>,
    checkpoint_authority: Arc<RwLock<RetentionAuthorities>>,
}

impl ConversationStore<MemoryBackend> {
    /// Create an ephemeral conversation store for tests and temporary sessions.
    pub fn in_memory(conversation_id: [u8; 32]) -> Self {
        Self {
            conversation_id,
            store: MunimentStore::new(MemoryBackend::new()),
            checkpoint_authority: Arc::new(RwLock::new(RetentionAuthorities::default())),
        }
    }
}

impl ConversationStore<RedbBackend> {
    /// Open a durable conversation store at `path`.
    pub fn at_path(
        conversation_id: [u8; 32],
        path: impl AsRef<Path>,
    ) -> Result<Self, ConversationStoreError> {
        Ok(Self {
            conversation_id,
            store: MunimentStore::new(RedbBackend::open(path)?),
            checkpoint_authority: Arc::new(RwLock::new(RetentionAuthorities::default())),
        })
    }
}

impl<B: Backend + Clone> ConversationStore<B> {
    /// Wrap a host-selected backend for one conversation.
    pub fn new(conversation_id: [u8; 32], backend: B) -> Self {
        Self {
            conversation_id,
            store: MunimentStore::new(backend),
            checkpoint_authority: Arc::new(RwLock::new(RetentionAuthorities::default())),
        }
    }

    /// The public identifier this store admits.
    pub const fn conversation_id(&self) -> [u8; 32] {
        self.conversation_id
    }

    /// Clone the shared store for LogSync or native-drop export.
    pub fn sync_store(&self) -> MunimentStore<B, CabalExt> {
        self.store.clone()
    }

    /// Install or withdraw authority for new checkpoints. Installed historical
    /// revisions continue to justify retained roots. After reopen, the policy
    /// owner reinstalls accepted revisions, ending with the current one.
    pub fn set_checkpoint_authority(&self, authority: Option<CabalCheckpointAuthority>) {
        let mut state = self.checkpoint_authority.write().unwrap();
        if let Some(authority) = authority.as_ref() {
            state
                .history
                .insert(authority.authority_revision(), authority.clone());
        }
        state.current = authority;
    }

    fn policy(&self) -> ConversationPolicy {
        ConversationPolicy {
            conversation_id: self.conversation_id,
            checkpoint_authority: self.checkpoint_authority.read().unwrap().current.clone(),
        }
    }

    /// Retained metadata with only authorized checkpoint backlinks cut.
    /// Cross-author parents are never excused by another author's prune flag.
    pub(crate) fn causal_entries(
        &self,
        operations: &[Operation<CabalExt>],
    ) -> Result<Vec<CausalEntry<u64>>, ConversationStoreError> {
        let history = self.checkpoint_authority.read().unwrap().history.clone();
        operations
            .iter()
            .map(|operation| {
                let mut entry = CausalEntry::from_operation(
                    operation,
                    0,
                    operation.header.extensions.parents.clone(),
                );
                if operation.header.extensions.prune_flag.is_set() {
                    let authority = operation
                        .header
                        .extensions
                        .authority_revision
                        .as_ref()
                        .and_then(|revision| history.get(revision))
                        .cloned();
                    let processor = OperationProcessor::new(
                        self.store.clone(),
                        ConversationPolicy {
                            conversation_id: self.conversation_id,
                            checkpoint_authority: authority,
                        },
                    );
                    processor.preflight(operation)?;
                    entry.backlink = None;
                }
                Ok(entry)
            })
            .collect()
    }

    /// Validate, admit, and atomically index one conversation operation.
    pub async fn accept(
        &self,
        operation: &Operation<CabalExt>,
    ) -> Result<ProcessOutcome, ConversationStoreError> {
        let processor = self.processor();
        Ok(processor.process(operation).await?)
    }

    fn processor(&self) -> OperationProcessor<B, CabalExt, ConversationPolicy> {
        OperationProcessor::new(self.store.clone(), self.policy())
    }

    /// Read every retained operation for rebuilding the materialized view.
    pub async fn operations(&self) -> Result<Vec<Operation<CabalExt>>, ConversationStoreError> {
        let records = export_topic_operations::<B, CabalExt, u64>(
            &self.store,
            &Topic::from(self.conversation_id),
            DropExportProfile::default(),
        )
        .await?;
        let mut operations = Vec::with_capacity(records.len());
        for record in &records {
            if let Some(operation) = decode_operation_record(record)? {
                operations.push(operation);
            }
        }
        Ok(operations)
    }

    /// Import a verified plaintext/public drop through conversation policy.
    pub async fn import_plain<R: Read>(
        &self,
        reader: R,
        limits: DropLimits,
    ) -> Result<DropImportReport, ConversationStoreError> {
        Ok(import_plain_drop(reader, limits, &self.processor()).await?)
    }

    /// Import a verified protected drop through conversation policy.
    pub async fn import_protected<R: Read, D: DropProtector>(
        &self,
        reader: R,
        limits: DropLimits,
        protector: &D,
    ) -> Result<DropImportReport, ConversationStoreError> {
        Ok(import_protected_drop(reader, limits, &self.processor(), protector).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelName, PostId, PostKind, post_to_operation, sign_post};
    use identity::{IdentityProvider, InMemoryProvider};
    use stickleback::{
        DropExportBudget, DropExportDecision, DropExportSelector, DropRecord,
        decode_operation_record, export_topic_operations_selected,
    };

    use crate::{ConversationDropPriorities, ConversationDropPrivacy, ConversationDropSelector};

    const CONVERSATION: [u8; 32] = [0x43; 32];

    fn chain() -> [Operation<CabalExt>; 2] {
        let keypair = InMemoryProvider::from_seed([8; 32])
            .derive_keypair(b"conversation-store")
            .unwrap();
        let message = post_to_operation(&sign_post(
            &keypair,
            CONVERSATION,
            0,
            None,
            Vec::new(),
            PostKind::Text {
                channel: ChannelName::new("general"),
                text: "private message".into(),
                timestamp_ms: 10,
            },
        ))
        .unwrap();
        let message_id = PostId::new(*message.hash.as_bytes());
        let deletion = post_to_operation(&sign_post(
            &keypair,
            CONVERSATION,
            1,
            Some(message_id),
            Vec::new(),
            PostKind::Delete {
                posts: vec![message_id],
                timestamp_ms: 20,
            },
        ))
        .unwrap();
        [message, deletion]
    }

    #[tokio::test]
    async fn shared_processor_admits_a_conversation_chain_idempotently() {
        let store = ConversationStore::in_memory(CONVERSATION);
        let [message, deletion] = chain();
        assert!(store.accept(&message).await.unwrap().inserted());
        assert!(store.accept(&deletion).await.unwrap().inserted());
        assert_eq!(
            store.accept(&deletion).await.unwrap(),
            ProcessOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn wrong_conversation_is_rejected_before_mutation() {
        let store = ConversationStore::in_memory([0x99; 32]);
        let [message, _] = chain();
        let error = store.accept(&message).await.unwrap_err();
        assert!(error.to_string().contains("wrong-conversation"));
        assert!(
            store
                .sync_store()
                .get_operation(&message.hash)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn conversation_selector_drives_the_shared_exporter() {
        let store = ConversationStore::in_memory(CONVERSATION);
        let [message, deletion] = chain();
        store.accept(&message).await.unwrap();
        store.accept(&deletion).await.unwrap();
        let selector = ConversationDropSelector::radio(
            ConversationDropPrivacy {
                include_post_headers: true,
                include_message_bodies: false,
                include_topic_bodies: false,
                include_profile_info: false,
                include_membership: false,
            },
            ConversationDropPriorities {
                deletion: 500,
                membership: 400,
                profile_info: 300,
                topic: 200,
                message: 100,
            },
        );
        assert_eq!(
            selector.select(&message),
            DropExportDecision::Header { priority: 100 }
        );
        let (records, stats) = export_topic_operations_selected::<_, _, u64, _>(
            &store.sync_store(),
            &Topic::from(CONVERSATION),
            &selector,
            DropExportBudget::default(),
        )
        .await
        .unwrap();
        assert_eq!(stats.selected_records, 2);
        let first = decode_operation_record::<CabalExt>(&records[0])
            .unwrap()
            .unwrap();
        assert_eq!(first.header.extensions.post_type, 1);
        let DropRecord::Operation { inline_body, .. } = &records[1] else {
            panic!("conversation exports operation records")
        };
        assert!(inline_body.is_none());
    }
}
