// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{Limits, session::CapInner};
use std::{fmt, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("session is closed")]
    Closed,
    #[error("resource limit: {0}")]
    Limit(&'static str),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("stream I/O: {0}")]
    Io(String),
    #[error("session deadline expired")]
    Timeout,
    #[error("capability belongs to a different session")]
    ForeignCapability,
    #[error("unknown reference")]
    UnknownReference,
    #[error("unsupported profile operation: {0}")]
    Unsupported(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub(crate) [u8; 16]);
impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionId(opaque)")
    }
}

#[derive(Clone)]
pub struct Capability(pub(crate) Arc<CapInner>);
impl Capability {
    pub fn session_id(&self) -> SessionId {
        self.0.session
    }
}
impl fmt::Debug for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Capability(opaque)")
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    String(String),
    Symbol(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Record { label: String, fields: Vec<Value> },
    Cap(Capability),
}

impl Value {
    pub fn symbol(value: impl Into<String>) -> Self {
        Self::Symbol(value.into())
    }
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }
    pub fn as_capability(&self) -> Option<&Capability> {
        if let Self::Cap(cap) = self {
            Some(cap)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub(crate) fn check(&self, limits: &Limits, session: SessionId) -> Result<(), SessionError> {
        walk(self, limits, session, 0, &mut 0, &mut 0)
    }
    pub(crate) fn check_sequence(
        values: &[Value],
        limits: &Limits,
        session: SessionId,
    ) -> Result<(), SessionError> {
        if values.len() > limits.max_collection_items {
            return Err(SessionError::Limit("collection items"));
        }
        let mut nodes = 1;
        let mut bytes = 32;
        for value in values {
            walk(value, limits, session, 1, &mut nodes, &mut bytes)?;
        }
        Ok(())
    }
}

fn walk(
    v: &Value,
    limits: &Limits,
    session: SessionId,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), SessionError> {
    *nodes += 1;
    if depth > limits.max_depth || *nodes > limits.max_value_nodes {
        return Err(SessionError::Limit("value shape"));
    }
    *bytes = bytes.saturating_add(32);
    match v {
        Value::String(s) | Value::Symbol(s) => *bytes = bytes.saturating_add(s.len()),
        Value::Bytes(s) => *bytes = bytes.saturating_add(s.len()),
        Value::List(xs) | Value::Record { fields: xs, .. } => {
            if xs.len() > limits.max_collection_items {
                return Err(SessionError::Limit("collection items"));
            }
            if let Value::Record { label, .. } = v {
                if label.starts_with("desc:")
                    || label.starts_with("op:")
                    || label.starts_with("ocapn-")
                    || label == "my-location"
                {
                    return Err(SessionError::Protocol("reserved record label".into()));
                }
                *bytes = bytes.saturating_add(label.len());
            }
            for x in xs {
                walk(x, limits, session, depth + 1, nodes, bytes)?;
            }
        },
        Value::Map(xs) => {
            if xs.len() > limits.max_collection_items {
                return Err(SessionError::Limit("collection items"));
            }
            for (k, v) in xs {
                walk(k, limits, session, depth + 1, nodes, bytes)?;
                walk(v, limits, session, depth + 1, nodes, bytes)?;
            }
        },
        Value::Cap(cap) if cap.session_id() != session => {
            return Err(SessionError::ForeignCapability);
        },
        _ => (),
    }
    if *bytes > limits.max_frame_bytes {
        return Err(SessionError::Limit("value bytes"));
    }
    Ok(())
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CallError {
    #[error("remote promise broke: {0:?}")]
    Broken(Value),
    #[error(transparent)]
    Session(#[from] SessionError),
}
impl CallError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Broken(Value::String(message.into()))
    }
}
