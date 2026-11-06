// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Custody acknowledgement: a holder's signed statement that it holds a log.
//!
//! The statement names the holder key, the log key, the newest edition hash
//! and sequence it verified, and a challenge the publisher chose for this one
//! request. The challenge is the session binding: an acknowledgement captured
//! from one request cannot answer another. Drop receipts carry no peer
//! identity by design, so this is the accepted-transfer fact for ordinary
//! sites. It says the holder had the editions at the time it signed; it
//! promises nothing about later retention or service.

use identity::{Ed25519Keypair, Ed25519PublicKey, Ed25519Signature};

const DOMAIN: &[u8] = b"murm-holder/ack/v1";

/// Encoded length of an [`Ack`].
pub const ACK_LEN: usize = 32 * 4 + 4 + 64;

/// Why an acknowledgement is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AckError {
    /// The bytes are not an acknowledgement.
    #[error("malformed acknowledgement")]
    Malformed,
    /// Signed by a key other than the configured holder.
    #[error("acknowledgement is from a different holder")]
    WrongHolder,
    /// About a different log.
    #[error("acknowledgement is for a different log")]
    WrongLog,
    /// Answers a different request.
    #[error("acknowledgement does not answer this challenge")]
    WrongChallenge,
    /// The signature does not verify.
    #[error("acknowledgement signature does not verify")]
    BadSignature,
}

/// A holder-signed custody statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ack {
    /// The holder's key, the one its Noise sessions prove.
    pub holder: [u8; 32],
    /// The release log's key.
    pub log: [u8; 32],
    /// Hash of the newest edition the holder verified.
    pub head: [u8; 32],
    /// That edition's sequence number.
    pub seq: u32,
    /// The publisher's per-request challenge.
    pub challenge: [u8; 32],
    signature: [u8; 64],
}

fn signing_bytes(
    holder: &[u8; 32],
    log: &[u8; 32],
    head: &[u8; 32],
    seq: u32,
    challenge: &[u8; 32],
) -> Vec<u8> {
    let mut out = DOMAIN.to_vec();
    for part in [holder, log, head, challenge] {
        out.extend_from_slice(part);
    }
    out.extend_from_slice(&seq.to_be_bytes());
    out
}

impl Ack {
    /// Sign a statement as `holder`.
    pub fn sign(
        holder: &Ed25519Keypair,
        log: [u8; 32],
        head: [u8; 32],
        seq: u32,
        challenge: [u8; 32],
    ) -> Self {
        let key = holder.public_key().to_bytes();
        let signature = holder
            .sign(&signing_bytes(&key, &log, &head, seq, &challenge))
            .to_bytes();
        Self {
            holder: key,
            log,
            head,
            seq,
            challenge,
            signature,
        }
    }

    /// Check this answers `challenge` for `log` and was signed by `holder`.
    ///
    /// # Errors
    /// The first field that differs from what was asked, or a bad signature.
    pub fn verify(
        &self,
        holder: &[u8; 32],
        log: &[u8; 32],
        challenge: &[u8; 32],
    ) -> Result<(), AckError> {
        if self.holder != *holder {
            return Err(AckError::WrongHolder);
        }
        if self.log != *log {
            return Err(AckError::WrongLog);
        }
        if self.challenge != *challenge {
            return Err(AckError::WrongChallenge);
        }
        let key = Ed25519PublicKey::from_bytes(&self.holder).map_err(|_| AckError::Malformed)?;
        let message = signing_bytes(
            &self.holder,
            &self.log,
            &self.head,
            self.seq,
            &self.challenge,
        );
        if key.verify(&message, &Ed25519Signature::from_bytes(&self.signature)) {
            Ok(())
        } else {
            Err(AckError::BadSignature)
        }
    }

    /// The wire form.
    pub fn encode(&self) -> [u8; ACK_LEN] {
        let mut out = [0u8; ACK_LEN];
        let mut at = 0;
        for part in [&self.holder, &self.log, &self.head, &self.challenge] {
            out[at..at + 32].copy_from_slice(part);
            at += 32;
        }
        out[at..at + 4].copy_from_slice(&self.seq.to_be_bytes());
        out[at + 4..].copy_from_slice(&self.signature);
        out
    }

    /// Parse the wire form. The signature is checked by [`verify`](Self::verify).
    ///
    /// # Errors
    /// The length is wrong.
    pub fn decode(bytes: &[u8]) -> Result<Self, AckError> {
        if bytes.len() != ACK_LEN {
            return Err(AckError::Malformed);
        }
        let take = |from: usize| <[u8; 32]>::try_from(&bytes[from..from + 32]).expect("32 bytes");
        Ok(Self {
            holder: take(0),
            log: take(32),
            head: take(64),
            challenge: take(96),
            seq: u32::from_be_bytes(bytes[128..132].try_into().expect("4 bytes")),
            signature: bytes[132..].try_into().expect("64 bytes"),
        })
    }
}
