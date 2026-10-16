use super::*;

#[test]
fn sennet_only_uses_its_lease_and_no_other_identity_and_reports_physical_completion() {
    let channel = Channel {
        hash: 8,
        key: ChannelKey::Aes128([2; 16]),
    };
    let instance = SennetInstance::new(
        SennetInstanceConfig {
            channel,
            flood: ManagedFloodConfig {
                channel_hash: 8,
                relay_node: 7,
                seen_capacity: 8,
                delay: RelayDelayWindow::new(
                    core::time::Duration::ZERO,
                    core::time::Duration::ZERO,
                )
                .unwrap(),
            },
            directory: Default::default(),
            pending_ttl: 1000,
        },
        PacketIdLease::new(7, 257, 513, 257).unwrap(),
    )
    .unwrap();
    let profile = PhyProfile::meshtastic_long_fast(906_875_000);
    let config = Config {
        controller: ControllerConfig {
            home: SENNET,
            pin: Some(SENNET),
            installed: InstalledPersonalitySet::new(&[SENNET]).unwrap(),
            coverage: CoveragePolicy::AllowGap,
            max_excursion_ms: 1000,
            return_budget_ms: 10,
            max_defer_ms: 100,
            transition_timeout_ms: 10,
        },
        profiles: [profile; 3],
        tx_budget_ms: 10,
        frame_ttl_ms: 1000,
    };
    let mut runtime = Runtime::sennet_only(0, config, instance).unwrap();
    assert!(runtime.installed_retinue().is_none());
    assert!(runtime.installed_tucket().is_none());
    runtime.send_sennet(1, header(), "channel text").unwrap();
    let tx = runtime.begin_tx(2).unwrap().unwrap();
    let packet = channel.open_text(&tx.frame).unwrap().unwrap();
    assert_eq!(packet.text, "channel text");
    assert_eq!((packet.header.source, packet.header.packet_id), (7, 257));
    let report = runtime.complete_tx(3, tx.id, true).unwrap();
    assert!(report.events.iter().any(|event| matches!(
        event,
        Event::Sennet(sennet::instance::InstanceEvent::TxCompleted { .. })
    )));
    assert!(runtime.profile(TUCKET).is_err());
    assert!(runtime.profile(RETINUE).is_err());
    assert!(runtime.advertise_tucket(4, 1, &[]).is_err());
}
