//! How a resource transfer ends in error: refused, timed out, or canceled.

use alloc::format;
use alloc::string::ToString;

use std::io;

use crate::resource_transfer::{SegmentedReceiver, SegmentedSender};

/// The I/O error for a resource this endpoint refused to receive: one that needs segment
/// accumulation, one past a size or part ceiling or refused by the accept policy, or one
/// whose body did not recover. The receiver has already sent the sender its cancel.
fn resource_receive_failure(error: crate::Error) -> io::Error {
    let kind = match error {
        crate::Error::MultiSegmentResource => io::ErrorKind::Unsupported,
        crate::Error::ResourceRejected => io::ErrorKind::PermissionDenied,
        crate::Error::ResourceTimedOut => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::InvalidData,
    };
    io::Error::new(kind, error)
}

/// The error for a publish that gave up: on a silent receiver, or a proof that never came.
pub(super) fn publish_timed_out<D: AsRef<[u8]>>(
    sender: &SegmentedSender<D>,
    awaiting_proof: bool,
) -> io::Error {
    let message = if awaiting_proof {
        "resource proof never arrived".to_string()
    } else if sender.served_parts() > 0 {
        format!(
            "resource publish timed out after serving {} requested part(s)",
            sender.served_parts()
        )
    } else if sender.has_started() {
        "resource publish timed out after receiver request matched no parts".to_string()
    } else {
        return io::Error::new(io::ErrorKind::TimedOut, ResourceNeverRequested);
    };
    io::Error::new(io::ErrorKind::TimedOut, message)
}

/// The error payload of a Resource the receiver never asked for. Nothing reached it, so the
/// offer can be made again on a fresh link: RNS ignores advertisements on a link it has not
/// activated, and activates on the one RTT packet.
#[derive(Debug)]
pub struct ResourceNeverRequested;

impl core::fmt::Display for ResourceNeverRequested {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("resource publish timed out before receiver request")
    }
}

impl std::error::Error for ResourceNeverRequested {}

/// Whether `error` is a Resource offer the receiver never answered.
pub fn is_resource_never_requested(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<ResourceNeverRequested>())
}

/// The error ending a receive whose transfer failed or was canceled by the sender, if it
/// has.
pub(super) fn resource_receive_ended(receiver: &SegmentedReceiver) -> io::Result<()> {
    if let Some(error) = receiver.failure() {
        Err(resource_receive_failure(error))
    } else if receiver.is_canceled() {
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "resource canceled by sender",
        ))
    } else {
        Ok(())
    }
}
