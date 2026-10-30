// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{BoxFuture, CallError, SessionError, SessionHandle, Value};
use std::{any::Any, collections::BTreeMap, sync::Arc};

/// Trusted host-supplied receiving-session context. Never encoded on the wire.
#[derive(Clone)]
pub struct SessionAuthority(pub(crate) Arc<dyn Any + Send + Sync>);

impl SessionAuthority {
    pub fn new<T: Any + Send + Sync>(authority: T) -> Self {
        Self(Arc::new(authority))
    }
}

#[derive(Clone)]
pub struct CallContext {
    pub(crate) authority: SessionAuthority,
    pub(crate) session: SessionHandle,
}

impl CallContext {
    pub fn authority<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.authority.0.downcast_ref()
    }
    pub fn session(&self) -> &SessionHandle {
        &self.session
    }
}

/// Only explicitly registered implementations are callable. Argument symbols
/// confer no ambient authority and there is no reflection or code evaluation.
pub trait HostObject: Send + Sync + 'static {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>>;
}

#[derive(Clone, Default)]
pub struct Bootstrap {
    pub(crate) objects: BTreeMap<Vec<u8>, Arc<dyn HostObject>>,
}

impl Bootstrap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(
        &mut self,
        swiss: impl Into<Vec<u8>>,
        object: Arc<dyn HostObject>,
    ) -> Result<(), SessionError> {
        let swiss = swiss.into();
        if swiss.is_empty() || swiss.len() > 4096 {
            return Err(SessionError::Limit("swiss bytes"));
        }
        if self.objects.len() >= 4096 {
            return Err(SessionError::Limit("bootstrap objects"));
        }
        if self.objects.contains_key(&swiss) {
            return Err(SessionError::Protocol("duplicate swiss number".into()));
        }
        self.objects.insert(swiss, object);
        Ok(())
    }
}
