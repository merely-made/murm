// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use retained_repair::{
    CollectorStats, Error, ExpiryReport, Lane, ObjectMetadata, PreparedFrame, ProducerStats,
    ReceiveOutcome, RecoveredObject,
};
use serde_json::{Value, json};

use super::protocol::Refusal;

pub(crate) fn metadata(object: &ObjectMetadata) -> Value {
    json!({"generation":object.generation,"digest_hex":hex::encode(object.digest),
        "object_octets":object.object_octets,"publish_at":object.publish_at,
        "expires_at":object.expires_at})
}

pub(crate) fn inventory(objects: &[ObjectMetadata]) -> Vec<Value> {
    objects.iter().map(metadata).collect()
}

pub(crate) fn expiry(report: &ExpiryReport) -> Value {
    json!({"objects":inventory(&report.objects),
        "invalidated_token":report.invalidated_token.map(|token| token.0)})
}

pub(crate) fn lane(lane: Lane) -> &'static str {
    match lane {
        Lane::Manifest => "manifest",
        Lane::Fresh => "fresh",
        Lane::Repair => "repair",
    }
}

pub(crate) fn prepared(frame: &PreparedFrame) -> Value {
    json!({"token":frame.token.0,"channel":frame.channel,"lane":lane(frame.lane),
        "generation":frame.generation,"esi":frame.esi,"frame_bytes_hex":hex::encode(&frame.bytes),
        "frame_octets":frame.bytes.len(),"frame_blake3":blake3::hash(&frame.bytes).to_hex().to_string()})
}

pub(crate) fn recovered(object: &RecoveredObject) -> Value {
    json!({"metadata":metadata(&object.metadata),"original_bytes_hex":hex::encode(&object.bytes),
        "actual_decoder_output":true,"application_admission":false})
}

pub(crate) fn received(outcome: &ReceiveOutcome) -> Value {
    match outcome {
        ReceiveOutcome::ManifestAccepted(object) => {
            json!({"outcome":"manifest_accepted","metadata":metadata(object)})
        }
        ReceiveOutcome::DuplicateManifest(object) => {
            json!({"outcome":"duplicate_manifest","metadata":metadata(object)})
        }
        ReceiveOutcome::SymbolAccepted => json!({"outcome":"symbol_accepted"}),
        ReceiveOutcome::DuplicateSymbol => json!({"outcome":"duplicate_symbol"}),
        ReceiveOutcome::AlreadyCompleted => json!({"outcome":"already_completed"}),
        ReceiveOutcome::Completed(object) => {
            json!({"outcome":"completed","recovered":recovered(object)})
        }
    }
}

pub(crate) fn producer_stats(stats: &ProducerStats) -> Value {
    json!({"encoder_allocations":stats.encoder_allocations,"queued_frames":stats.queued_frames,
        "queued_wire_octets":stats.queued_wire_octets,"queued_manifests":stats.queued_manifests,
        "queued_fresh":stats.queued_fresh,"queued_repairs":stats.queued_repairs,
        "deferrals":stats.deferrals,"expired_objects":stats.expired_objects})
}

pub(crate) fn collector_stats(stats: &CollectorStats) -> Value {
    json!({"decoder_allocations":stats.decoder_allocations,"decode_work":stats.decode_work,
        "completed_objects":stats.completed_objects,"expired_objects":stats.expired_objects})
}

pub(crate) fn refusal(error: Error) -> Refusal {
    let code = match &error {
        Error::InvalidProfile(_) | Error::WrongAuthority => "invalid_profile",
        Error::ClockRegression => "clock_regression",
        Error::EpochExhausted => "epoch_exhausted",
        Error::AdvanceRequired => "advance_required",
        Error::InvalidExpiry => "invalid_expiry",
        Error::ObjectSize => "object_size",
        Error::Full { .. } => "full",
        Error::GenerationReused => "generation_reused",
        Error::ConflictingObject => "conflicting_object",
        Error::UnknownGeneration => "unknown_object",
        Error::PendingFrame => "pending_frame",
        Error::StaleToken => "stale_token",
        Error::TokenExhausted => "token_exhausted",
        Error::EsiExhausted => "esi_exhausted",
        Error::MalformedFrame => "malformed_frame",
        Error::InvalidSignature => "invalid_auth",
        Error::ProfileMismatch => "profile_mismatch",
        Error::ExpiredFrame => "expired",
        Error::DecodeBudget => "decode_budget",
        Error::ReconstructedDigestMismatch => "reconstructed_digest_mismatch",
        Error::CodecInvariant => "codec_invariant",
    };
    let mut refusal = Refusal::new(code, error.to_string());
    if let Error::Full { next_expiry } = error {
        refusal.retry_at_slot = next_expiry;
    }
    refusal
}
