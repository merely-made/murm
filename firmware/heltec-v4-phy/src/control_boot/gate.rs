//! Preserve the durable first-write boundary before any ordinary service.

use radio_hand::control::{FirstWriteStatus, PairEvidence};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FirstWriteBootGate {
    ControlPresent,
    ClaimOnly,
    StatusOnly,
}

pub(super) const fn first_write_boot_gate(status: FirstWriteStatus) -> FirstWriteBootGate {
    if matches!(status.control, PairEvidence::Valid) {
        FirstWriteBootGate::ControlPresent
    } else if status.ordinary_service_eligible()
        || status.actions().permits_resume()
        || status.actions().permits_abandon()
    {
        FirstWriteBootGate::ClaimOnly
    } else {
        FirstWriteBootGate::StatusOnly
    }
}
