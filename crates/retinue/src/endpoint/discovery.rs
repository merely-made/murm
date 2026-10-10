//! Path requests: answering from the path table, recursive discovery, and the interface-mode
//! rules (`Transport.py` 3279-3325, 3380-3600; `Interface.py` 55-56).
//!
//! Seam: the router's path-request handling moved here unchanged, with a state slot. Unit V2
//! (A4) implements discovery.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::path::PathRequest;

use super::interface::InterfaceId;
use super::queue::TrafficClass;
use super::shared::Shared;

/// Path requests this node is searching for on others' behalf.
#[derive(Debug, Default)]
pub(super) struct Discovery {}

/// Answer a path request heard on `iface`. RNS drops a tagless request and a repeated target
/// and tag (`Transport.py` 1838-1856), and answers on the requesting interface only
/// (`Transport.py` 3452-3456). `ingress_limited` forbids a recursive search (3542-3547).
pub(super) fn handle_path_request(
    shared: &Arc<Shared>,
    iface: InterfaceId,
    request: PathRequest,
    ingress_limited: bool,
) {
    let _ = (&shared.discovery, ingress_limited);
    let fresh = request
        .unique_tag()
        .is_some_and(|tag| shared.path_request_tags.lock().unwrap().insert(tag));
    if !fresh {
        shared
            .routing_stats
            .filtered_packets
            .fetch_add(1, Ordering::Relaxed);
    } else if let Some(resp) = shared.path_response(request.target) {
        shared.send_on_class(iface, resp, TrafficClass::Control);
    }
}
