//! The endpoint runtime: the tokio shell that turns the R0–R4 primitives into a working
//! peer.
//!
//! An [`Endpoint`] holds an identity, an address book, and any number of interfaces. A
//! background router reads packets from every interface, tagged with the interface they
//! arrived on, and dispatches them: announces populate the address book, inbound link
//! requests are proved and surfaced as connections, and link data reaches the
//! [`LinkStream`] for its link. This is the seam a host implements its own transport trait
//! against; see the crate root.

mod announces;
mod attach;
mod config;
mod dedup;
mod dial;
mod discovery;
mod entropy;
mod facts;
mod iface_config;
mod iface_policy;
mod iface_stats;
mod inbound;
mod inbound_queue;
mod interface;
mod known_destinations;
mod link_receipt;
mod link_setup;
mod listen;
mod local;
mod mtu;
mod paths;
#[cfg(feature = "pipe")]
mod pipe;
mod pump;
mod queue;
mod rebroadcast;
mod registration;
mod reliable_driver;
mod resource_errors;
mod resource_inbound;
mod resource_pace;
mod resource_requests;
mod resource_session;
mod router;
mod routing;
mod runtime;
mod sealing;
mod shared;
mod single;
mod sockopt;
mod stream;
#[cfg(test)]
mod tests;
mod transit;
mod tunnel;
mod watchdog;

pub use announces::AnnounceFreshnessPolicy;
pub use dial::TcpClient;
pub use facts::{
    AnnounceFact, EndpointFacts, LinkDirection, LinkFact, LinkFactKind, LinkRemoteFact,
    PeerAnnounce, RouteFact,
};
pub use iface_config::{
    Carrier, IdBeacon, InterfaceSettings, RnsAttached, RnsConfig, RnsConfigError, RnsInterface,
    UdpCarrier,
};
pub use iface_policy::{AnnounceRate, IfacePolicy, PathRequestPolicy};
pub use iface_stats::InterfaceCounters;
pub use inbound::{Accepted, AcceptedResource, InboundLinkLimits};
pub use interface::{Interface, InterfaceId, InterfaceSink};
pub use link_receipt::{LinkDelivery, PayloadReceipt};
pub use link_setup::{LinkSetupTimedOut, is_link_setup_timeout};
pub use listen::{FlapPolicy, ListenAddr, ListenPolicy, Listener};
pub use local::{SHARED_INSTANCE_PORT, SharedInstance, SharedInstanceKind};
pub use mtu::HwMtu;
#[cfg(feature = "pipe")]
pub use pipe::{PIPE_HW_MTU, PipeConfig};
pub use queue::{
    ClassCounters, OutboundPackets, QueueCounters, QueueDepths, QueueWeights, TrafficClass,
};
pub use resource_inbound::{ReceivedLinkData, SessionInbound};
pub use resource_requests::{ReceivedRawRequest, ReceivedRawResponse, ReceivedRequest};
pub use resource_session::{
    PayloadMode, ReceivedPayload, ResourceNeverRequested, ResourceSession, ResourceTransferConfig,
    is_resource_never_requested,
};
pub use routing::{InterfaceSelector, RoutingCounters, RoutingPolicy};
pub use runtime::Endpoint;
pub use single::{ProofStrategy, ReceivedSingle, SingleDelivery, SinglePacketReceipt};
pub use stream::LinkStream;
