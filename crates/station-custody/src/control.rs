// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Signed low-bandwidth control frames for a sited station.
//!
//! The carrier is ordinary authenticated LXMF. This module owns only the
//! application frame that rides inside it: an expiring station grant, a
//! permanent revoke instruction, and the station-signed acknowledgement. It
//! does not teach Retinue about Personae or decide how a headless device seals
//! its delegated identity and this receiver snapshot at rest.

use std::fmt;

use insigne::CheckFault;
use insigne::DerivedKeyAttestation;
use p2panda_core::cbor::{decode_cbor, decode_cbor_strict, encode_cbor};
use sporran::station_grant::{SitedStationGrant, SitedStationGrantError};
use sporran::{DeviceGrantError, decode_device_grant_set, encode_device_grant_set};
use personae::carry::DeviceId;
use personae::{
    Ed25519Keypair, Ed25519PublicKey, Ed25519Signature, IdentityError, IdentityProvider,
};
use retinue::identity::{Identity, PrivateIdentity};
use serde::{Deserialize, Serialize};

/// LXMF title identifying a version-one station-control frame.
pub const SITED_STATION_CONTROL_TITLE: &[u8] = b"mere.sited-station-control/v1";
/// LXMF title identifying a version-one station-control acknowledgement.
pub const SITED_STATION_CONTROL_ACK_TITLE: &[u8] = b"mere.sited-station-control-ack/v1";

const CONTROL_SCHEMA_VERSION: u16 = 1;
const MAX_CONTROL_FRAME_BYTES: usize = 4096;

/// The host-only signing authority for one station's control frames.
///
/// It is a per-device Personae child. The frame includes the master's
/// attestation of that child, allowing a headless station to validate commands
/// from the Persona that signed its current grant without holding the master
/// secret.
#[derive(Clone)]
pub struct SitedStationControlSigner {
    device_id: DeviceId,
    keypair: Ed25519Keypair,
    attestation: DerivedKeyAttestation,
}

impl fmt::Debug for SitedStationControlSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SitedStationControlSigner")
            .field("device_id", &self.device_id)
            .field("public_key", &self.keypair.public_key().to_bytes())
            .finish_non_exhaustive()
    }
}

impl SitedStationControlSigner {
    pub(crate) fn derive(
        provider: &dyn IdentityProvider,
        device_id: DeviceId,
    ) -> Result<Self, IdentityError> {
        let salt = control_salt(device_id);
        Ok(Self {
            device_id,
            keypair: provider.derive_keypair(&salt)?,
            attestation: provider.attest_derived_key(&salt)?,
        })
    }

    pub(crate) fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Sign a grant-delivery or renewal frame for this station.
    pub fn grant(
        &self,
        grant: &SitedStationGrant,
    ) -> Result<SitedStationControl, SitedStationControlError> {
        if grant.device_id() != self.device_id {
            return Err(SitedStationControlError::WrongDevice {
                expected: self.device_id,
                actual: grant.device_id(),
            });
        }
        // The set is what travels now, not a single envelope. A station's set
        // is one device certificate, so this stays one record on the wire.
        let bytes = encode_device_grant_set(grant.signed())
            .map_err(|_| SitedStationControlError::Encode)?;
        self.sign(
            grant.issued_at_ms(),
            SitedStationControlCommand::Grant { grant: bytes },
        )
    }

    /// Sign a permanent revoke instruction for this station.
    pub fn revoke(
        &self,
        revoked_at_ms: u64,
    ) -> Result<SitedStationControl, SitedStationControlError> {
        self.sign(
            revoked_at_ms,
            SitedStationControlCommand::Revoke {
                device_id: self.device_id,
                revoked_at_ms,
            },
        )
    }

    fn sign(
        &self,
        issued_at_ms: u64,
        command: SitedStationControlCommand,
    ) -> Result<SitedStationControl, SitedStationControlError> {
        let body = SitedStationControlBody {
            schema_version: CONTROL_SCHEMA_VERSION,
            issued_at_ms,
            attestation: self.attestation.clone(),
            command,
        };
        let signature = self.keypair.sign(&encode(&body)?).to_bytes().to_vec();
        Ok(SitedStationControl { body, signature })
    }
}

/// A signed command delivered to a particular sited station.
#[derive(Clone, Debug)]
pub struct SitedStationControl {
    body: SitedStationControlBody,
    signature: Vec<u8>,
}

impl SitedStationControl {
    /// Decode a bounded control frame from an LXMF body.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SitedStationControlError> {
        if bytes.len() > MAX_CONTROL_FRAME_BYTES {
            return Err(SitedStationControlError::FrameTooLarge {
                actual: bytes.len(),
            });
        }
        let wire: SitedStationControlWire = decode(bytes)?;
        if wire.schema_version != CONTROL_SCHEMA_VERSION {
            return Err(SitedStationControlError::UnsupportedSchema {
                actual: wire.schema_version,
            });
        }
        Ok(Self {
            body: SitedStationControlBody {
                schema_version: wire.schema_version,
                issued_at_ms: wire.issued_at_ms,
                attestation: wire.attestation,
                command: wire.command,
            },
            signature: wire.signature,
        })
    }

    /// Encode this frame for an LXMF body.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SitedStationControlError> {
        let bytes = encode(&SitedStationControlWire {
            schema_version: self.body.schema_version,
            issued_at_ms: self.body.issued_at_ms,
            attestation: self.body.attestation.clone(),
            command: self.body.command.clone(),
            signature: self.signature.clone(),
        })?;
        if bytes.len() > MAX_CONTROL_FRAME_BYTES {
            return Err(SitedStationControlError::FrameTooLarge {
                actual: bytes.len(),
            });
        }
        Ok(bytes)
    }

    /// Decode this frame only when the accompanying LXMF title is ours.
    pub fn from_delivery(
        title: &[u8],
        body: &[u8],
    ) -> Result<Option<Self>, SitedStationControlError> {
        (title == SITED_STATION_CONTROL_TITLE)
            .then(|| Self::from_bytes(body))
            .transpose()
    }

    /// Stable correlation id for its acknowledgement and any retransmission.
    pub fn id(&self) -> Result<[u8; 32], SitedStationControlError> {
        Ok(*blake3::hash(&self.to_bytes()?).as_bytes())
    }

    /// The station device this command targets.
    pub fn device_id(&self) -> Result<DeviceId, SitedStationControlError> {
        match &self.body.command {
            SitedStationControlCommand::Grant { grant } => {
                Ok(SitedStationGrant::from_signed(decode_device_grant_set(grant)?)?.device_id())
            },
            SitedStationControlCommand::Revoke { device_id, .. } => Ok(*device_id),
        }
    }

    fn verify(
        &self,
        expected_issuer: [u8; 32],
        device_id: DeviceId,
    ) -> Result<(), SitedStationControlError> {
        let checked = self
            .body
            .attestation
            .check(&control_salt(device_id))
            .map_err(SitedStationControlError::InvalidControlAttestation)?;
        if checked.master() != &expected_issuer {
            return Err(SitedStationControlError::ControlIssuerMismatch);
        }
        let derived = Ed25519PublicKey::from_bytes(checked.derived())
            .map_err(SitedStationControlError::Identity)?;
        let signature: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| SitedStationControlError::InvalidControlSignatureLength)?;
        if !derived.verify(
            &encode(&self.body)?,
            &Ed25519Signature::from_bytes(&signature),
        ) {
            return Err(SitedStationControlError::InvalidControlSignature);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SitedStationControlBody {
    schema_version: u16,
    issued_at_ms: u64,
    attestation: DerivedKeyAttestation,
    command: SitedStationControlCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SitedStationControlWire {
    schema_version: u16,
    issued_at_ms: u64,
    attestation: DerivedKeyAttestation,
    command: SitedStationControlCommand,
    signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum SitedStationControlCommand {
    Grant {
        grant: Vec<u8>,
    },
    Revoke {
        device_id: DeviceId,
        revoked_at_ms: u64,
    },
}

/// The receiver's result, signed into an acknowledgement by the station key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SitedStationControlResult {
    /// A first grant or a pre-expiry renewal was durably accepted.
    GrantInstalled { expires_at_ms: u64 },
    /// The receiver became permanently revoked.
    Revoked,
}

/// A station-signed receipt for one accepted control frame.
#[derive(Clone, Debug)]
pub struct SitedStationControlAck {
    body: SitedStationControlAckBody,
    signature: Vec<u8>,
}

impl SitedStationControlAck {
    fn sign(
        control: &SitedStationControl,
        device_id: DeviceId,
        acknowledged_at_ms: u64,
        result: SitedStationControlResult,
        station_identity: &PrivateIdentity,
    ) -> Result<Self, SitedStationControlError> {
        let body = SitedStationControlAckBody {
            schema_version: CONTROL_SCHEMA_VERSION,
            control_id: control.id()?,
            device_id,
            acknowledged_at_ms,
            result,
        };
        Ok(Self {
            signature: station_identity.sign(&encode(&body)?).to_vec(),
            body,
        })
    }

    /// Decode a bounded acknowledgement frame from an LXMF body.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SitedStationControlError> {
        if bytes.len() > MAX_CONTROL_FRAME_BYTES {
            return Err(SitedStationControlError::FrameTooLarge {
                actual: bytes.len(),
            });
        }
        let wire: SitedStationControlAckWire = decode(bytes)?;
        if wire.schema_version != CONTROL_SCHEMA_VERSION {
            return Err(SitedStationControlError::UnsupportedSchema {
                actual: wire.schema_version,
            });
        }
        Ok(Self {
            body: SitedStationControlAckBody {
                schema_version: wire.schema_version,
                control_id: wire.control_id,
                device_id: wire.device_id,
                acknowledged_at_ms: wire.acknowledged_at_ms,
                result: wire.result,
            },
            signature: wire.signature,
        })
    }

    /// Encode this acknowledgement for an LXMF body.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SitedStationControlError> {
        let bytes = encode(&SitedStationControlAckWire {
            schema_version: self.body.schema_version,
            control_id: self.body.control_id,
            device_id: self.body.device_id,
            acknowledged_at_ms: self.body.acknowledged_at_ms,
            result: self.body.result.clone(),
            signature: self.signature.clone(),
        })?;
        if bytes.len() > MAX_CONTROL_FRAME_BYTES {
            return Err(SitedStationControlError::FrameTooLarge {
                actual: bytes.len(),
            });
        }
        Ok(bytes)
    }

    /// Decode this acknowledgement only when the accompanying LXMF title is ours.
    pub fn from_delivery(
        title: &[u8],
        body: &[u8],
    ) -> Result<Option<Self>, SitedStationControlError> {
        (title == SITED_STATION_CONTROL_ACK_TITLE)
            .then(|| Self::from_bytes(body))
            .transpose()
    }

    /// Verify that the target station signed this acknowledgement.
    pub fn verify(
        &self,
        expected_station: &Identity,
        expected_control: &SitedStationControl,
    ) -> Result<&SitedStationControlResult, SitedStationControlError> {
        let signature: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| SitedStationControlError::InvalidAckSignatureLength)?;
        if !expected_station.verify(&encode(&self.body)?, &signature) {
            return Err(SitedStationControlError::InvalidAckSignature);
        }
        if self.body.control_id != expected_control.id()? {
            return Err(SitedStationControlError::AckControlMismatch);
        }
        if self.body.device_id != expected_control.device_id()? {
            return Err(SitedStationControlError::AckDeviceMismatch);
        }
        Ok(&self.body.result)
    }

    /// The device that accepted the frame.
    pub fn device_id(&self) -> DeviceId {
        self.body.device_id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SitedStationControlAckBody {
    schema_version: u16,
    control_id: [u8; 32],
    device_id: DeviceId,
    acknowledged_at_ms: u64,
    result: SitedStationControlResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SitedStationControlAckWire {
    schema_version: u16,
    control_id: [u8; 32],
    device_id: DeviceId,
    acknowledged_at_ms: u64,
    result: SitedStationControlResult,
    signature: Vec<u8>,
}

/// The durable, non-secret state a headless station must retain with its
/// delegated Reticulum identity.
///
/// This type intentionally leaves the at-rest implementation to the board
/// host. Call [`Self::snapshot`] after accepting a frame and place its bytes in
/// the board's secure storage boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SitedStationControlReceiver {
    device_id: DeviceId,
    station_ed25519_public_key: [u8; 32],
    grant: Option<Vec<u8>>,
    closed: bool,
    revoked: bool,
}

impl SitedStationControlReceiver {
    /// Start an uncommissioned receiver for one station key.
    pub fn new(device_id: DeviceId, station_ed25519_public_key: [u8; 32]) -> Self {
        Self {
            device_id,
            station_ed25519_public_key,
            grant: None,
            closed: false,
            revoked: false,
        }
    }

    /// The durable device this receiver is bound to.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Restore a persisted receiver snapshot without granting any new authority.
    pub fn from_snapshot(
        bytes: &[u8],
        device_id: DeviceId,
        station_ed25519_public_key: [u8; 32],
    ) -> Result<Self, SitedStationControlError> {
        let state: SitedStationControlState = decode_snapshot(bytes)?;
        if state.schema_version != CONTROL_SCHEMA_VERSION {
            return Err(SitedStationControlError::UnsupportedSchema {
                actual: state.schema_version,
            });
        }
        if state.device_id != device_id {
            return Err(SitedStationControlError::WrongDevice {
                expected: device_id,
                actual: state.device_id,
            });
        }
        if state.station_ed25519_public_key != station_ed25519_public_key {
            return Err(SitedStationControlError::StationKeyMismatch { device_id });
        }
        let receiver = Self {
            device_id,
            station_ed25519_public_key,
            grant: state.grant,
            closed: state.closed,
            revoked: state.revoked,
        };
        if let Some(grant) = receiver.current_grant()? {
            receiver.assert_grant_binding(&grant)?;
        }
        Ok(receiver)
    }

    /// Produce the non-secret state a headless station must atomically persist.
    pub fn snapshot(&self) -> Result<Vec<u8>, SitedStationControlError> {
        encode(&SitedStationControlState {
            schema_version: CONTROL_SCHEMA_VERSION,
            device_id: self.device_id,
            station_ed25519_public_key: self.station_ed25519_public_key,
            grant: self.grant.clone(),
            closed: self.closed,
            revoked: self.revoked,
        })
    }

    /// Process one control frame and return a station-signed acknowledgement.
    ///
    /// The caller must persist [`Self::snapshot`] before sending the returned
    /// acknowledgement. A power loss may then cause a harmless retransmission,
    /// rather than an acknowledgement for authority that was never retained.
    pub fn receive(
        &mut self,
        frame: &SitedStationControl,
        now_ms: u64,
        station_identity: &PrivateIdentity,
    ) -> Result<SitedStationControlAck, SitedStationControlError> {
        if station_identity.public().ed25519_bytes() != &self.station_ed25519_public_key {
            return Err(SitedStationControlError::StationKeyMismatch {
                device_id: self.device_id,
            });
        }
        let result = match &frame.body.command {
            SitedStationControlCommand::Grant { grant } => {
                self.install_grant(frame, grant, now_ms)?
            },
            SitedStationControlCommand::Revoke {
                device_id,
                revoked_at_ms: _,
            } => self.revoke(frame, *device_id)?,
        };
        SitedStationControlAck::sign(frame, self.device_id, now_ms, result, station_identity)
    }

    /// Interpret one LXMF delivery as station control when, and only when, it
    /// bears this protocol's title.
    ///
    /// An unrelated chat delivery returns `Ok(None)`. The caller must persist
    /// [`Self::snapshot`] before it transmits the returned acknowledgement.
    pub fn receive_delivery(
        &mut self,
        title: &[u8],
        body: &[u8],
        now_ms: u64,
        station_identity: &PrivateIdentity,
    ) -> Result<Option<SitedStationControlAck>, SitedStationControlError> {
        let Some(frame) = SitedStationControl::from_delivery(title, body)? else {
            return Ok(None);
        };
        self.receive(&frame, now_ms, station_identity).map(Some)
    }

    /// Authorize the current delegated station identity at one instant.
    ///
    /// Reaching expiry closes this receiver permanently. A later control grant
    /// cannot restart it; a physical/operator recovery flow must create a new
    /// runtime from a freshly commissioned state.
    pub fn authorize_at(&mut self, now_ms: u64) -> Result<u64, SitedStationControlError> {
        if self.revoked {
            return Err(SitedStationControlError::Revoked {
                device_id: self.device_id,
            });
        }
        if self.closed {
            return Err(SitedStationControlError::Closed {
                device_id: self.device_id,
            });
        }
        let grant = self
            .current_grant()?
            .ok_or(SitedStationControlError::MissingGrant {
                device_id: self.device_id,
            })?;
        self.assert_grant_binding(&grant)?;
        if let Err(error) =
            grant.authorize_at(self.device_id, self.station_ed25519_public_key, now_ms)
        {
            if matches!(error, SitedStationGrantError::Expired { .. }) {
                self.closed = true;
            }
            return Err(error.into());
        }
        Ok(grant.expires_at_ms())
    }

    /// Whether this receiver has permanently stopped accepting new authority.
    pub fn is_closed(&self) -> bool {
        self.closed || self.revoked
    }

    fn install_grant(
        &mut self,
        frame: &SitedStationControl,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<SitedStationControlResult, SitedStationControlError> {
        if self.revoked {
            return Err(SitedStationControlError::Revoked {
                device_id: self.device_id,
            });
        }
        if self.closed {
            return Err(SitedStationControlError::Closed {
                device_id: self.device_id,
            });
        }
        let existing = self.current_grant()?;
        if let Some(existing) = &existing {
            self.assert_grant_binding(existing)?;
            let existing_expiry = existing.expires_at_ms();
            if now_ms >= existing_expiry {
                self.closed = true;
                return Err(SitedStationControlError::DeadlineElapsed {
                    device_id: self.device_id,
                    expires_at_ms: existing_expiry,
                    now_ms,
                });
            }
        }
        let incoming = SitedStationGrant::from_signed(decode_device_grant_set(bytes)?)?;
        self.assert_grant_binding(&incoming)?;
        frame.verify(incoming.issuer_public_key(), self.device_id)?;
        incoming.authorize_at(self.device_id, self.station_ed25519_public_key, now_ms)?;

        if let Some(existing) = existing {
            if self.grant.as_deref() == Some(bytes) {
                return Ok(SitedStationControlResult::GrantInstalled {
                    expires_at_ms: existing.expires_at_ms(),
                });
            }
            if existing.issuer_public_key() != incoming.issuer_public_key() {
                return Err(SitedStationControlError::ControlIssuerMismatch);
            }
            let existing_expiry = existing.expires_at_ms();
            if incoming.expires_at_ms() <= existing_expiry {
                return Err(SitedStationControlError::NonExtendingGrant {
                    device_id: self.device_id,
                    existing_expires_at_ms: existing_expiry,
                    incoming_expires_at_ms: incoming.expires_at_ms(),
                });
            }
        }
        self.grant = Some(bytes.to_vec());
        Ok(SitedStationControlResult::GrantInstalled {
            expires_at_ms: incoming.expires_at_ms(),
        })
    }

    fn revoke(
        &mut self,
        frame: &SitedStationControl,
        device_id: DeviceId,
    ) -> Result<SitedStationControlResult, SitedStationControlError> {
        if device_id != self.device_id {
            return Err(SitedStationControlError::WrongDevice {
                expected: self.device_id,
                actual: device_id,
            });
        }
        let existing = self
            .current_grant()?
            .ok_or(SitedStationControlError::MissingGrant {
                device_id: self.device_id,
            })?;
        self.assert_grant_binding(&existing)?;
        frame.verify(existing.issuer_public_key(), self.device_id)?;
        self.closed = true;
        self.revoked = true;
        Ok(SitedStationControlResult::Revoked)
    }

    fn current_grant(&self) -> Result<Option<SitedStationGrant>, SitedStationControlError> {
        self.grant
            .as_deref()
            .map(decode_device_grant_set)
            .transpose()?
            .map(SitedStationGrant::from_signed)
            .transpose()
            .map_err(Into::into)
    }

    fn assert_grant_binding(
        &self,
        grant: &SitedStationGrant,
    ) -> Result<(), SitedStationControlError> {
        if grant.device_id() != self.device_id {
            return Err(SitedStationControlError::WrongDevice {
                expected: self.device_id,
                actual: grant.device_id(),
            });
        }
        if grant.station_ed25519_public_key() != self.station_ed25519_public_key {
            return Err(SitedStationControlError::StationKeyMismatch {
                device_id: self.device_id,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SitedStationControlState {
    schema_version: u16,
    device_id: DeviceId,
    station_ed25519_public_key: [u8; 32],
    grant: Option<Vec<u8>>,
    closed: bool,
    revoked: bool,
}

/// Why a station-control frame, acknowledgement, or receiver transition failed.
#[derive(Debug)]
pub enum SitedStationControlError {
    /// Canonical control encoding or signing preimage construction failed.
    Encode,
    /// A control frame or snapshot could not be decoded.
    Decode,
    /// The schema version is not understood.
    UnsupportedSchema { actual: u16 },
    /// A bounded low-bandwidth frame exceeded the control-carrier limit.
    FrameTooLarge { actual: usize },
    /// A Personae attestation could not expose one of its public keys.
    Identity(IdentityError),
    /// The control child was not attested for this station id; the fault
    /// says which step of the check failed.
    InvalidControlAttestation(CheckFault),
    /// The attested control child was not authorized by the current grant issuer.
    ControlIssuerMismatch,
    /// The command signature did not have the required Ed25519 length.
    InvalidControlSignatureLength,
    /// The command signature was invalid.
    InvalidControlSignature,
    /// The acknowledgement signature did not have the required Ed25519 length.
    InvalidAckSignatureLength,
    /// The acknowledgement was not signed by the expected station identity.
    InvalidAckSignature,
    /// The acknowledgement describes a different command than the one sent.
    AckControlMismatch,
    /// The acknowledgement describes a different device than the command sent.
    AckDeviceMismatch,
    /// A frame named a different station.
    WrongDevice {
        expected: DeviceId,
        actual: DeviceId,
    },
    /// A grant bound a different Reticulum signing key.
    StationKeyMismatch { device_id: DeviceId },
    /// The station has no currently stored control grant.
    MissingGrant { device_id: DeviceId },
    /// The station was permanently stopped by expiry or operator action.
    Closed { device_id: DeviceId },
    /// The station was permanently revoked.
    Revoked { device_id: DeviceId },
    /// The previous accepted deadline passed before a renewal landed.
    DeadlineElapsed {
        device_id: DeviceId,
        expires_at_ms: u64,
        now_ms: u64,
    },
    /// A renewal failed to extend the current live deadline.
    NonExtendingGrant {
        device_id: DeviceId,
        existing_expires_at_ms: u64,
        incoming_expires_at_ms: u64,
    },
    /// The carried station grant was invalid.
    Grant(SitedStationGrantError),
}

impl From<SitedStationGrantError> for SitedStationControlError {
    fn from(error: SitedStationGrantError) -> Self {
        Self::Grant(error)
    }
}

impl From<DeviceGrantError> for SitedStationControlError {
    fn from(error: DeviceGrantError) -> Self {
        Self::Grant(SitedStationGrantError::Envelope(error))
    }
}

impl fmt::Display for SitedStationControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode => f.write_str("could not encode sited-station control frame"),
            Self::Decode => f.write_str("could not decode sited-station control frame"),
            Self::UnsupportedSchema { actual } => {
                write!(f, "unsupported sited-station control schema {actual}")
            },
            Self::FrameTooLarge { actual } => write!(
                f,
                "sited-station control frame is {actual} bytes, over the {MAX_CONTROL_FRAME_BYTES}-byte carrier limit"
            ),
            Self::Identity(error) => write!(f, "sited-station control identity failed: {error}"),
            Self::InvalidControlAttestation(fault) => write!(
                f,
                "sited-station control key is not attested for this device: {fault}"
            ),
            Self::ControlIssuerMismatch => f.write_str(
                "sited-station control key was not authorized by this station's grant issuer",
            ),
            Self::InvalidControlSignatureLength => {
                f.write_str("sited-station control signature was not 64 bytes")
            },
            Self::InvalidControlSignature => f.write_str("sited-station control signature failed"),
            Self::InvalidAckSignatureLength => {
                f.write_str("sited-station control acknowledgement signature was not 64 bytes")
            },
            Self::InvalidAckSignature => {
                f.write_str("sited-station control acknowledgement signature failed")
            },
            Self::AckControlMismatch => {
                f.write_str("sited-station acknowledgement names a different control frame")
            },
            Self::AckDeviceMismatch => {
                f.write_str("sited-station acknowledgement names a different device")
            },
            Self::WrongDevice { expected, actual } => write!(
                f,
                "sited-station control targets {}, not expected device {}",
                actual.as_uuid(),
                expected.as_uuid()
            ),
            Self::StationKeyMismatch { device_id } => write!(
                f,
                "sited-station control key does not match device {}",
                device_id.as_uuid()
            ),
            Self::MissingGrant { device_id } => write!(
                f,
                "sited-station device {} has no control grant",
                device_id.as_uuid()
            ),
            Self::Closed { device_id } => write!(
                f,
                "sited-station device {} is closed and requires recovery",
                device_id.as_uuid()
            ),
            Self::Revoked { device_id } => {
                write!(f, "sited-station device {} is revoked", device_id.as_uuid())
            },
            Self::DeadlineElapsed {
                device_id,
                expires_at_ms,
                now_ms,
            } => write!(
                f,
                "sited-station device {} expired at {expires_at_ms}; renewal arrived at {now_ms}",
                device_id.as_uuid()
            ),
            Self::NonExtendingGrant {
                device_id,
                existing_expires_at_ms,
                incoming_expires_at_ms,
            } => write!(
                f,
                "sited-station device {} renewal expires at {incoming_expires_at_ms}, not after existing deadline {existing_expires_at_ms}",
                device_id.as_uuid()
            ),
            Self::Grant(error) => write!(f, "sited-station control grant failed: {error}"),
        }
    }
}

impl std::error::Error for SitedStationControlError {}

fn control_salt(device_id: DeviceId) -> Vec<u8> {
    personae::reticulum::station_control_salt(device_id.as_uuid().as_bytes())
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, SitedStationControlError> {
    encode_cbor(value).map_err(|_| SitedStationControlError::Encode)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, SitedStationControlError> {
    decode_cbor_strict(bytes).map_err(|_| SitedStationControlError::Decode)
}

// A local snapshot this receiver wrote; peers never supply it.
fn decode_snapshot<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
) -> Result<T, SitedStationControlError> {
    decode_cbor(bytes).map_err(|_| SitedStationControlError::Decode)
}

#[cfg(test)]
mod tests {
    use personae::carry::DeviceId;
    use personae::InMemoryProvider;

    use crate::authority::test_support::TestWallet;

    /// The test wallet and the directory it writes in, kept alive together.
    struct Commissioned {
        _dir: tempfile::TempDir,
        wallet: std::sync::Arc<TestWallet>,
    }
    use tempfile::tempdir;

    use super::*;
    use crate::SitedStationCredential;

    fn commissioned() -> (
        Commissioned,
        InMemoryProvider,
        DeviceId,
        SitedStationCredential,
        SitedStationGrant,
    ) {
        let dir = tempdir().unwrap();
        let wallet = TestWallet::new(dir.path(), 0x5a);
        let provider = wallet.provider();
        let device_id = DeviceId::new();
        let credential = SitedStationCredential::derive_for_device(&provider, device_id).unwrap();
        let grant = credential
            .issue_remote_auth_grant(wallet.as_ref(), device_id, "Ridge north", 100, 200)
            .unwrap();
        (
            Commissioned { _dir: dir, wallet },
            provider,
            device_id,
            credential,
            grant,
        )
    }

    #[test]
    fn a_grant_is_carried_acknowledged_and_survives_receiver_restart() {
        let (_root, _provider, device_id, credential, grant) = commissioned();
        let control = credential.control_signer().grant(&grant).unwrap();
        let encoded = control.to_bytes().unwrap();
        assert!(encoded.len() <= MAX_CONTROL_FRAME_BYTES);
        assert_eq!(
            SitedStationControl::from_delivery(SITED_STATION_CONTROL_TITLE, &encoded)
                .unwrap()
                .unwrap()
                .id()
                .unwrap(),
            control.id().unwrap()
        );

        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        let ack = receiver
            .receive_delivery(
                SITED_STATION_CONTROL_TITLE,
                &encoded,
                101,
                &credential.identity,
            )
            .unwrap();
        let ack = ack.expect("station-control title should be received");
        assert_eq!(
            ack.verify(credential.public_identity(), &control).unwrap(),
            &SitedStationControlResult::GrantInstalled { expires_at_ms: 200 }
        );
        assert_eq!(receiver.authorize_at(199).unwrap(), 200);

        let snapshot = receiver.snapshot().unwrap();
        let mut restored = SitedStationControlReceiver::from_snapshot(
            &snapshot,
            device_id,
            *credential.public_identity().ed25519_bytes(),
        )
        .unwrap();
        assert_eq!(restored.authorize_at(199).unwrap(), 200);
    }

    #[test]
    fn a_lost_ack_can_be_retried_but_a_changed_same_expiry_grant_is_refused() {
        let (_root, _provider, device_id, credential, grant) = commissioned();
        let control = credential.control_signer().grant(&grant).unwrap();
        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        receiver
            .receive(&control, 101, &credential.identity)
            .unwrap();

        let retry = receiver
            .receive(&control, 102, &credential.identity)
            .unwrap();
        assert_eq!(
            retry
                .verify(credential.public_identity(), &control)
                .unwrap(),
            &SitedStationControlResult::GrantInstalled { expires_at_ms: 200 }
        );

        let mut different = control.clone();
        different.body.issued_at_ms = 102;
        let error = receiver
            .receive(&different, 102, &credential.identity)
            .unwrap_err();
        assert!(matches!(
            error,
            SitedStationControlError::InvalidControlSignature
        ));
    }

    #[test]
    fn malformed_or_wrong_title_frames_do_not_become_station_controls() {
        assert!(
            SitedStationControl::from_delivery(b"chat", &[0; 16])
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            SitedStationControl::from_delivery(SITED_STATION_CONTROL_TITLE, &[0; 16]),
            Err(SitedStationControlError::Decode)
        ));

        let (_, _, device_id, credential, _) = commissioned();
        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        assert!(
            receiver
                .receive_delivery(b"chat", b"hello", 101, &credential.identity)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn only_a_pre_expiry_extension_from_the_grant_persona_is_accepted() {
        let (root, _provider, device_id, credential, grant) = commissioned();
        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        receiver
            .receive(
                &credential.control_signer().grant(&grant).unwrap(),
                101,
                &credential.identity,
            )
            .unwrap();

        let renewed = credential
            .issue_remote_auth_grant(root.wallet.as_ref(), device_id, "Ridge north", 150, 300)
            .unwrap();
        let renewal = credential.control_signer().grant(&renewed).unwrap();
        let ack = receiver
            .receive(&renewal, 150, &credential.identity)
            .unwrap();
        assert_eq!(
            ack.verify(credential.public_identity(), &renewal).unwrap(),
            &SitedStationControlResult::GrantInstalled { expires_at_ms: 300 }
        );
        assert_eq!(receiver.authorize_at(200).unwrap(), 300);

        let intruder = InMemoryProvider::random();
        let wrong_credential =
            SitedStationCredential::derive_for_device(&intruder, device_id).unwrap();
        let error = receiver
            .receive(
                &wrong_credential.control_signer().grant(&renewed).unwrap(),
                201,
                &credential.identity,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            SitedStationControlError::ControlIssuerMismatch
        ));
    }

    #[test]
    fn expiry_and_revocation_are_permanent_for_one_receiver_runtime() {
        let (root, _provider, device_id, credential, grant) = commissioned();
        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        receiver
            .receive(
                &credential.control_signer().grant(&grant).unwrap(),
                101,
                &credential.identity,
            )
            .unwrap();
        assert!(matches!(
            receiver.authorize_at(200),
            Err(SitedStationControlError::Grant(
                SitedStationGrantError::Expired { .. }
            ))
        ));
        let renewed = credential
            .issue_remote_auth_grant(root.wallet.as_ref(), device_id, "Ridge north", 150, 300)
            .unwrap();
        assert!(matches!(
            receiver.receive(
                &credential.control_signer().grant(&renewed).unwrap(),
                201,
                &credential.identity,
            ),
            Err(SitedStationControlError::Closed { .. })
        ));

        let (_root, _provider, device_id, credential, grant) = commissioned();
        let mut receiver = SitedStationControlReceiver::new(
            device_id,
            *credential.public_identity().ed25519_bytes(),
        );
        receiver
            .receive(
                &credential.control_signer().grant(&grant).unwrap(),
                101,
                &credential.identity,
            )
            .unwrap();
        let revoke = credential.control_signer().revoke(150).unwrap();
        let ack = receiver
            .receive(&revoke, 151, &credential.identity)
            .unwrap();
        assert_eq!(
            ack.verify(credential.public_identity(), &revoke).unwrap(),
            &SitedStationControlResult::Revoked
        );
        assert!(matches!(
            receiver.authorize_at(151),
            Err(SitedStationControlError::Revoked { .. })
        ));
    }
}
