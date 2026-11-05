// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Bounded message framing for LogSync over a byte stream.
//!
//! One frame is a four-byte big-endian length and that many bytes of CBOR.
//! The declared length is checked before any body is buffered, and the bytes a
//! session may read in total are capped, so a peer cannot make the reader
//! allocate or loop without bound. A refusal is recorded in a shared [`Fault`]
//! because the protocol flattens codec errors into strings.

use std::io;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use bytes::{Buf, BufMut, BytesMut};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::codec::{Decoder, Encoder};

const PREFIX: usize = 4;

/// Why the codec refused a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaultKind {
    /// A frame, declared or about to be sent, exceeds the frame ceiling.
    Oversize,
    /// The session read more bytes than its budget.
    Budget,
}

/// The first refusal a codec made, readable after the protocol has failed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fault(Arc<Mutex<Option<FaultKind>>>);

impl Fault {
    fn record(&self, kind: FaultKind) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(kind);
    }

    pub(crate) fn get(&self) -> Option<FaultKind> {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct CborCodec<M> {
    max_frame: u32,
    max_total: u64,
    read: u64,
    fault: Fault,
    _message: PhantomData<fn() -> M>,
}

impl<M> CborCodec<M> {
    pub(crate) fn new(max_frame: u32, max_total: u64, fault: Fault) -> Self {
        Self {
            max_frame,
            max_total,
            read: 0,
            fault,
            _message: PhantomData,
        }
    }

    fn refuse(&self, kind: FaultKind, text: String) -> io::Error {
        self.fault.record(kind);
        io::Error::new(io::ErrorKind::InvalidData, text)
    }
}

impl<M: DeserializeOwned> Decoder for CborCodec<M> {
    type Item = M;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<M>, io::Error> {
        if src.len() < PREFIX {
            return Ok(None);
        }
        let declared = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
        if declared > self.max_frame {
            return Err(self.refuse(
                FaultKind::Oversize,
                format!("frame of {declared} bytes exceeds {}", self.max_frame),
            ));
        }
        let total = PREFIX + declared as usize;
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }
        self.read += total as u64;
        if self.read > self.max_total {
            return Err(self.refuse(
                FaultKind::Budget,
                format!("session read budget of {} bytes spent", self.max_total),
            ));
        }
        src.advance(PREFIX);
        let body = src.split_to(declared as usize);
        p2panda_core::cbor::decode_cbor(&body[..])
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }
}

impl<M: Serialize> Encoder<M> for CborCodec<M> {
    type Error = io::Error;

    fn encode(&mut self, item: M, dst: &mut BytesMut) -> Result<(), io::Error> {
        let body = p2panda_core::cbor::encode_cbor(&item)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        if body.len() > self.max_frame as usize {
            return Err(self.refuse(
                FaultKind::Oversize,
                format!(
                    "outgoing frame of {} bytes exceeds {}",
                    body.len(),
                    self.max_frame
                ),
            ));
        }
        dst.reserve(PREFIX + body.len());
        dst.put_u32(body.len() as u32);
        dst.extend_from_slice(&body);
        Ok(())
    }
}
