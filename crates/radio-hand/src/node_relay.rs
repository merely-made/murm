//! Where a node build's relay setting comes from (wall node D2: "Config; on when unclaimed"),
//! and how a running node follows the configuration in force (D6: "Stop at once").
//!
//! Radio-free, so the mapping is tested on the desk under `replay`, the radio-free form that
//! links Retinue's node; the boards reach it through `node`, which implies `replay`.

use retinue::node::{FIRST_HOP_ALLOWANCE_BITS, InterfaceId, Node, TransportConfig};
use selvage::PhyProfile;

/// What the configuration in force says about relaying: the flags of the last configuration
/// the board applied, or none applied this boot on an unclaimed board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySource {
    /// No control journal: the board relays both.
    Unclaimed,
    /// A claimed board follows its committed configuration.
    Claimed {
        relay_announces: bool,
        relay_packets: bool,
    },
}

/// The node's transport settings. Only the two relay flags follow the source; the hop limit
/// and lifetimes stay at [`TransportConfig::transit`]'s, whatever the configuration says.
pub fn transport_config(source: RelaySource) -> TransportConfig {
    let transit = TransportConfig::transit();
    match source {
        RelaySource::Unclaimed => transit,
        RelaySource::Claimed {
            relay_announces,
            relay_packets,
        } => TransportConfig {
            relay_announces,
            relay_packets,
            ..transit
        },
    }
}

/// Bring a node to the configuration in force: its relay flags, from the next packet, and the
/// first-hop airtime allowance of `profile`, the profile the radio then runs (Ruling 70).
/// Routes and carried links are kept; the forwarding paths read the flags per packet.
pub fn follow<const PEERS: usize, const ACTIONS: usize, const LINKS: usize>(
    node: &mut Node<PEERS, ACTIONS, LINKS>,
    source: RelaySource,
    interface: InterfaceId,
    profile: &PhyProfile,
) {
    node.set_transport_config(transport_config(source));
    let allowance = crate::phy::nominal_bits_ms(
        profile.spreading_factor,
        profile.bandwidth_hz,
        profile.coding_rate_denominator,
        FIRST_HOP_ALLOWANCE_BITS,
    )
    .unwrap_or(0);
    // One interface on a node that names one: the table cannot be full.
    let _ = node.set_first_hop_airtime(interface, allowance);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claimed(relay_announces: bool, relay_packets: bool) -> TransportConfig {
        transport_config(RelaySource::Claimed {
            relay_announces,
            relay_packets,
        })
    }

    fn keeps_transit_limits(config: TransportConfig) {
        let transit = TransportConfig::transit();
        assert_eq!(config.max_hops, transit.max_hops);
        assert_eq!(config.route_ttl, transit.route_ttl);
        assert_eq!(config.bridge_ttl, transit.bridge_ttl);
    }

    #[test]
    fn unclaimed_board_relays_both() {
        let config = transport_config(RelaySource::Unclaimed);
        assert_eq!(config, TransportConfig::transit());
        assert!(config.relay_announces && config.relay_packets);
    }

    #[test]
    fn claimed_announce_flag_alone_relays_only_announces() {
        let config = claimed(true, false);
        assert!(config.relay_announces);
        assert!(!config.relay_packets);
        keeps_transit_limits(config);
    }

    #[test]
    fn claimed_packet_flag_alone_relays_only_packets() {
        let config = claimed(false, true);
        assert!(!config.relay_announces);
        assert!(config.relay_packets);
        keeps_transit_limits(config);
    }

    #[test]
    fn claimed_both_flags_relay_both() {
        let config = claimed(true, true);
        assert_eq!(config, TransportConfig::transit());
    }

    #[test]
    fn claimed_neither_flag_relays_nothing() {
        let config = claimed(false, false);
        assert!(!config.relay_announces);
        assert!(!config.relay_packets);
        keeps_transit_limits(config);
    }

    fn running_node() -> Node<32, 8, 4> {
        let mut node = Node::new(
            retinue::identity::PrivateIdentity::from_secret_bytes(&[0x42; 64]),
            retinue::destination::DestinationName::new("retinue", ["node"]).name_hash(),
        );
        follow(&mut node, RelaySource::Unclaimed, 0, &long_fast());
        node
    }

    fn long_fast() -> PhyProfile {
        PhyProfile::meshtastic_long_fast(906_875_000)
    }

    fn claimed_source(relay_announces: bool, relay_packets: bool) -> RelaySource {
        RelaySource::Claimed {
            relay_announces,
            relay_packets,
        }
    }

    #[test]
    fn a_running_node_follows_each_flag_alone_and_both() {
        let mut node = running_node();
        assert_eq!(node.transport_config(), TransportConfig::transit());
        for (announces, packets) in [(true, false), (false, true), (true, true), (false, false)] {
            follow(
                &mut node,
                claimed_source(announces, packets),
                0,
                &long_fast(),
            );
            let config = node.transport_config();
            assert_eq!(config.relay_announces, announces);
            assert_eq!(config.relay_packets, packets);
            keeps_transit_limits(config);
        }
    }

    #[test]
    fn the_first_hop_allowance_follows_the_profile_then_running() {
        let mut node = running_node();
        assert_eq!(node.first_hop_airtime(0), 3_724, "LongFast (Ruling 50)");
        let mut faster = long_fast();
        faster.spreading_factor = 7;
        follow(&mut node, claimed_source(true, true), 0, &faster);
        assert!(node.first_hop_airtime(0) < 3_724);
    }

    #[test]
    fn a_revert_restores_the_earlier_settings() {
        let mut node = running_node();
        let committed = claimed_source(true, false);
        follow(&mut node, committed, 0, &long_fast());
        let before = (node.transport_config(), node.first_hop_airtime(0));

        // A provisional candidate: no relaying, a faster profile.
        let mut faster = long_fast();
        faster.spreading_factor = 7;
        follow(&mut node, claimed_source(false, false), 0, &faster);
        assert_ne!((node.transport_config(), node.first_hop_airtime(0)), before);

        // Revert or expiry applies the committed configuration again.
        follow(&mut node, committed, 0, &long_fast());
        assert_eq!((node.transport_config(), node.first_hop_airtime(0)), before);
    }
}
