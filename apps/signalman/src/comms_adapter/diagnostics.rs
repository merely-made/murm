//! Read-only operator facts from the station-scoped application journal.
use super::*;
use crate::message::{MessageStatus, QueuedReason};
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LxmfDiagnostics {
    pub local_destination: [u8; 16],
    pub queued_offline: usize,
    pub queued_for_station: usize,
    pub waiting_for_peer: usize,
    pub handed_to_radio: usize,
    pub accepted_by_propagation: usize,
    pub received_direct: usize,
    pub fetched: usize,
    pub cancelled: usize,
    pub failures: Vec<LxmfFailure>,
    pub last_observed_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LxmfFailure {
    pub message_id: [u8; 32],
    pub detail: String,
}

impl LxmfDiagnostics {
    /// Summarize recorded events; no liveness or recipient-read claim follows.
    pub fn summary(&self) -> String {
        format!(
            "Recorded LXMF: {} offline, {} queued for station, {} waiting for peer; {} handed to radio; {} received directly, {} fetched; {} failed.",
            self.queued_offline,
            self.queued_for_station,
            self.waiting_for_peer,
            self.handed_to_radio,
            self.received_direct,
            self.fetched,
            self.failures.len()
        )
    }
}

impl<B: LxmfMailbox> LxmfAdapter<B> {
    /// Project the current immutable owner book. Transport/radio captures keep
    /// their own source and age; this report does not infer a route or RF health.
    pub fn diagnostics(&self) -> Result<LxmfDiagnostics, AdapterError> {
        let mut report = LxmfDiagnostics {
            local_destination: self.local.destination,
            ..Default::default()
        };
        for record in self.mailbox.book()?.iter() {
            if self.other(record).is_none() {
                continue;
            }
            report.last_observed_unix_ms = Some(
                report
                    .last_observed_unix_ms
                    .unwrap_or(0)
                    .max(record.observed_unix_ms),
            );
            match &record.status {
                MessageStatus::Queued(QueuedReason::Offline) => report.queued_offline += 1,
                MessageStatus::Queued(QueuedReason::ReadyForCarriage) => {
                    report.queued_for_station += 1
                }
                MessageStatus::Queued(QueuedReason::WaitingForPeer) => report.waiting_for_peer += 1,
                MessageStatus::HandedToRadio { .. } => report.handed_to_radio += 1,
                MessageStatus::AcceptedByPropagationNode => report.accepted_by_propagation += 1,
                MessageStatus::ReceivedDirect { .. } => report.received_direct += 1,
                MessageStatus::FetchedFromPropagationNode { .. } => report.fetched += 1,
                MessageStatus::Cancelled => report.cancelled += 1,
                MessageStatus::Failed(detail) => report.failures.push(LxmfFailure {
                    message_id: record.message.id().0,
                    detail: detail.clone(),
                }),
            }
        }
        Ok(report)
    }
}
