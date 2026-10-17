//! Construction of bounded resident protocols after durable reservations.
use super::ResidentState;
use crate::store::{ReservationError, SettingsStore};
use core::time::Duration;
use radio_hand::{
    instances::{Config, Runtime},
    resident_wire::{ResidentSetup, SennetKey},
};
use retinue::{
    hash::NameHash,
    identity::PrivateIdentity,
    node::{FreshnessPolicy, Node, PayloadLimits},
};
use selvage::personality::{
    ControllerConfig, CoveragePolicy, InstalledPersonalitySet, PersonalityId,
};
use sennet::{
    flood::{ManagedFloodConfig, RelayDelayWindow},
    instance::{PacketIdLease, SennetInstance, SennetInstanceConfig},
    node::Channel,
    node_info::NodeDirectoryConfig,
    transport::ChannelKey,
};
use tucket::{
    identity::LocalIdentity,
    instance::{Instance, InstanceConfig},
    node::{Node as TucketNode, NodeCapacity},
};
const ANNOUNCE_LEASE: u64 = 65_536;

#[derive(Debug)]
pub(crate) enum BuildError {
    MissingIdentity,
    Storage(ReservationError),
    Configuration,
    Quiet,
    #[cfg(feature = "resident-contact-store")]
    Contacts(radio_hand::tucket_contacts::Fault),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(cause) => write!(f, "storage: {cause:?}"),
            #[cfg(feature = "resident-contact-store")]
            Self::Contacts(cause) => write!(f, "contacts: {cause:?}"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Owned volatile setup and verified reservations. The worker never gets flash
/// or radio access. Deliberately no Debug implementation for identity material.
pub(crate) struct Prepared {
    now: u64,
    setup: ResidentSetup,
    identity: [u8; 64],
    packet: radio_hand::packet_reservation::Lease,
    announce: radio_hand::announce_reservation::ActiveLease,
    #[cfg(feature = "resident-contact-store")]
    cache: radio_hand::tucket_contacts::Snapshot,
}

/// Construct the three cores only from a verified USB setup and already-durable
/// reservations. Identity comes from stored board settings, never USB bytes.
#[inline(never)]
pub(crate) fn prepare(
    now: u64,
    setup: ResidentSetup,
    stored_identity: Option<[u8; 64]>,
    store: &mut SettingsStore,
) -> Result<Prepared, BuildError> {
    let identity = stored_identity.ok_or(BuildError::MissingIdentity)?;
    #[cfg(feature = "resident-contact-store")]
    let cache = {
        use radio_hand::tucket_contacts::{self, Scope, Snapshot};
        let public = LocalIdentity::from_seed(setup.tucket_identity_seed)
            .identity()
            .pub_key;
        let scope = Scope::new(public, setup.profiles[2]);
        let cached = tucket_contacts::load(store, scope).map_err(BuildError::Contacts)?;
        cached
            .map(|c| c.snapshot)
            .unwrap_or(Snapshot::new(scope, &[]).map_err(BuildError::Contacts)?)
    };
    let packet = store
        .reserve_packet_ids(setup.packet_lease_count)
        .map_err(BuildError::Storage)?;
    let announce = store
        .reserve_announce_lease(ANNOUNCE_LEASE)
        .map_err(BuildError::Storage)?;
    Ok(Prepared {
        now,
        setup,
        identity,
        packet,
        announce,
        #[cfg(feature = "resident-contact-store")]
        cache,
    })
}

/// Runs in the construction worker, after the large USB poll frame unwinds.
#[inline(never)]
pub(super) fn build(prepared: Prepared) -> Result<ResidentState, BuildError> {
    let Prepared {
        now,
        setup,
        identity,
        packet,
        announce,
        #[cfg(feature = "resident-contact-store")]
        cache,
    } = prepared;
    #[allow(unused_mut)]
    let mut tucket = Instance::new(
        TucketNode::with_capacity(
            LocalIdentity::from_seed(setup.tucket_identity_seed),
            false,
            NodeCapacity::new(8, 32).map_err(|_| BuildError::Configuration)?,
        )
        .map_err(|_| BuildError::Configuration)?,
        InstanceConfig::new(2, 1_000).ok_or(BuildError::Configuration)?,
    );
    #[cfg(feature = "resident-contact-store")]
    tucket
        .restore_contacts(&cache.keys)
        .map_err(|_| BuildError::Configuration)?;

    let key = match setup.sennet_key {
        SennetKey::Aes128(key) => ChannelKey::Aes128(key),
        SennetKey::Aes256(key) => ChannelKey::Aes256(key),
    };
    let sennet = SennetInstance::new(
        SennetInstanceConfig {
            channel: Channel {
                hash: setup.sennet_channel,
                key,
            },
            flood: ManagedFloodConfig {
                channel_hash: setup.sennet_channel,
                relay_node: (setup.sennet_source & 0xff) as u8,
                seen_capacity: 16,
                delay: RelayDelayWindow::new(Duration::ZERO, Duration::ZERO)
                    .map_err(|_| BuildError::Configuration)?,
            },
            directory: NodeDirectoryConfig {
                capacity: 8,
                id_limit: 32,
                long_name_limit: 64,
                short_name_limit: 16,
            },
            pending_ttl: setup.frame_ttl_ms,
        },
        PacketIdLease::new(
            setup.sennet_source,
            packet.start(),
            packet.end(),
            packet.start(),
        )
        .map_err(|_| BuildError::Configuration)?,
    )
    .map_err(|_| BuildError::Configuration)?;
    let ids = [PersonalityId(0), PersonalityId(1), PersonalityId(2)];
    let controller = ControllerConfig {
        home: PersonalityId(setup.home),
        pin: setup.pin.map(PersonalityId),
        installed: InstalledPersonalitySet::new(&ids).map_err(|_| BuildError::Configuration)?,
        coverage: if setup.require_coverage {
            CoveragePolicy::RequireCoverage
        } else {
            CoveragePolicy::AllowGap
        },
        max_excursion_ms: setup.max_excursion_ms,
        return_budget_ms: setup.return_budget_ms,
        max_defer_ms: setup.max_defer_ms,
        transition_timeout_ms: setup.transition_timeout_ms,
    };
    let name = NameHash::from_bytes(setup.retinue_name_hash);
    let retinue = Node::<8, 4, 1, 4>::new_with_payload_limits(
        PrivateIdentity::from_secret_bytes(&identity),
        name,
        PayloadLimits {
            max_ingress_bytes: 255,
            max_app_data: 32,
            max_link_payload: 160,
            max_outbound_resource: 1_024,
            max_resource_parts: 8,
        },
    )
    .with_freshness_policy(FreshnessPolicy {
        max_destinations: 8,
        max_blobs_per_destination: 2,
    })
    .map_err(|_| BuildError::Configuration)?;
    let runtime = Runtime::new(
        now,
        Config {
            controller,
            profiles: setup.profiles,
            tx_budget_ms: setup.tx_budget_ms,
            frame_ttl_ms: setup.frame_ttl_ms,
        },
        retinue,
        sennet,
        tucket,
    )
    .map_err(|_| BuildError::Configuration)?;
    let announce = retinue::announce::TimebaseGenerator::firmware_lease(
        announce.floor(),
        announce.reserved_through(),
    )
    .map_err(|_| BuildError::Configuration)?;
    Ok(ResidentState {
        runtime,
        announce,
        #[cfg(feature = "resident-contact-store")]
        cache,
    })
}
