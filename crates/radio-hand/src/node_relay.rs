//! Where a node build's relay setting comes from (wall node D2: "Config; on when unclaimed").
//!
//! Radio-free, so the mapping is tested on the desk under `replay`, the radio-free form that
//! links Retinue's node; the boards reach it through `node`, which implies `replay`.

use retinue::node::TransportConfig;

/// What the control journal says about relaying, as boot found it.
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
}
