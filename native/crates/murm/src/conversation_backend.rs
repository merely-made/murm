// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Configurable storage backend for Murm conversations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use muniment::{Backend, MemoryBackend, RedbBackend, StoreError, WriteOp};

use crate::ConversationStoreError;

/// Host-selected persistence for direct conversations.
///
/// Memory remains useful for tests and temporary sessions. Desktop hosts should
/// select [`Redb`](Self::Redb); each cabal gets one database named by its stable
/// public id, so reopening the same cabal key reopens the same store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConversationStorage {
    /// Ephemeral process-local storage.
    #[default]
    Memory,
    /// One durable redb database per cabal under `directory`.
    Redb {
        /// Host-configured directory for conversation databases.
        directory: PathBuf,
    },
}

impl ConversationStorage {
    /// Select durable desktop storage under `directory`.
    pub fn redb(directory: impl Into<PathBuf>) -> Self {
        Self::Redb {
            directory: directory.into(),
        }
    }

    fn open(
        &self,
        conversation_id: [u8; 32],
    ) -> Result<ConversationBackend, ConversationStoreError> {
        match self {
            Self::Memory => Ok(ConversationBackend::Memory(MemoryBackend::new())),
            Self::Redb { directory } => {
                std::fs::create_dir_all(directory).map_err(|error| {
                    ConversationStoreError::Backend(format!(
                        "create conversation store directory {}: {error}",
                        directory.display()
                    ))
                })?;
                let path = conversation_path(directory, conversation_id);
                Ok(ConversationBackend::Redb(RedbBackend::open(path)?))
            },
        }
    }
}

fn conversation_path(directory: &Path, conversation_id: [u8; 32]) -> PathBuf {
    directory.join(format!("{}.redb", hex::encode(conversation_id)))
}

/// One concrete backend type shared by the runtime, LogSync, and drop import.
#[derive(Clone)]
pub enum ConversationBackend {
    /// Ephemeral memory backend.
    Memory(MemoryBackend),
    /// Durable redb backend.
    Redb(RedbBackend),
}

impl ConversationBackend {
    pub(crate) fn open(
        storage: &ConversationStorage,
        conversation_id: [u8; 32],
    ) -> Result<Self, ConversationStoreError> {
        storage.open(conversation_id)
    }
}

#[async_trait]
impl Backend for ConversationBackend {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        match self {
            Self::Memory(backend) => backend.get(key).await,
            Self::Redb(backend) => backend.get(key).await,
        }
    }

    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        match self {
            Self::Memory(backend) => backend.put(key, bytes).await,
            Self::Redb(backend) => backend.put(key, bytes).await,
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        match self {
            Self::Memory(backend) => backend.delete(key).await,
            Self::Redb(backend) => backend.delete(key).await,
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        match self {
            Self::Memory(backend) => backend.list(prefix).await,
            Self::Redb(backend) => backend.list(prefix).await,
        }
    }

    async fn scan(&self, start: &str, end: &str) -> Result<Vec<String>, StoreError> {
        match self {
            Self::Memory(backend) => backend.scan(start, end).await,
            Self::Redb(backend) => backend.scan(start, end).await,
        }
    }

    async fn apply(&self, ops: &[WriteOp]) -> Result<(), StoreError> {
        match self {
            Self::Memory(backend) => backend.apply(ops).await,
            Self::Redb(backend) => backend.apply(ops).await,
        }
    }
}
