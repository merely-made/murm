// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::SessionError;
use std::time::Duration;

/// All runtime-owned buffers and tables have independently enforced ceilings.
/// Their products (for example frames * bytes per frame) bound aggregate use.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_depth: usize,
    pub max_value_nodes: usize,
    pub max_collection_items: usize,
    pub max_exports: usize,
    pub max_imports: usize,
    pub max_answers: usize,
    pub max_listeners: usize,
    pub max_pipeline_calls: usize,
    pub max_host_calls: usize,
    pub max_command_queue: usize,
    pub max_outbound_frames: usize,
    pub max_bootstrap_objects: usize,
    pub max_swiss_bytes: usize,
    pub handshake_timeout: Duration,
    pub request_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_session_duration: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 256 * 1024,
            max_depth: 32,
            max_value_nodes: 8192,
            max_collection_items: 1024,
            max_exports: 256,
            max_imports: 256,
            max_answers: 256,
            max_listeners: 256,
            max_pipeline_calls: 256,
            max_host_calls: 32,
            max_command_queue: 64,
            max_outbound_frames: 32,
            max_bootstrap_objects: 64,
            max_swiss_bytes: 256,
            handshake_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(120),
            max_session_duration: Duration::from_secs(3600),
        }
    }
}

impl Limits {
    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        for (name, value, ceiling) in [
            ("frame bytes", self.max_frame_bytes, 16 * 1024 * 1024),
            ("depth", self.max_depth, 128),
            ("value nodes", self.max_value_nodes, 65536),
            ("collection items", self.max_collection_items, 65536),
            ("exports", self.max_exports, 65536),
            ("imports", self.max_imports, 65536),
            ("answers", self.max_answers, 65536),
            ("listeners", self.max_listeners, 65536),
            ("pipeline calls", self.max_pipeline_calls, 65536),
            ("host calls", self.max_host_calls, 4096),
            ("command queue", self.max_command_queue, 4096),
            ("outbound frames", self.max_outbound_frames, 4096),
            ("bootstrap objects", self.max_bootstrap_objects, 4096),
            ("swiss bytes", self.max_swiss_bytes, 4096),
        ] {
            if value == 0 || value > ceiling {
                return Err(SessionError::Limit(name));
            }
        }
        for duration in [
            self.handshake_timeout,
            self.request_timeout,
            self.idle_timeout,
            self.max_session_duration,
        ] {
            if duration.is_zero() || duration > Duration::from_secs(24 * 3600) {
                return Err(SessionError::Limit("deadline"));
            }
        }
        Ok(())
    }
}
