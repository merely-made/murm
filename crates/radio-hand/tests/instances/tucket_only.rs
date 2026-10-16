use super::*;

#[test]
fn one_protocol_runtime_needs_no_other_identity_or_packet_id_lease() {
    let profile = PhyProfile::meshcore(906_875_000, 250_000, 10, 5);
    let config = Config {
        controller: ControllerConfig {
            home: TUCKET,
            pin: Some(TUCKET),
            installed: InstalledPersonalitySet::new(&[TUCKET]).unwrap(),
            coverage: CoveragePolicy::AllowGap,
            max_excursion_ms: 1_000,
            return_budget_ms: 10,
            max_defer_ms: 100,
            transition_timeout_ms: 10,
        },
        profiles: [profile; 3],
        tx_budget_ms: 10,
        frame_ttl_ms: 1_000,
    };
    let mut node = TucketNode::new(LocalIdentity::from_seed([3; 32]), false);
    let advert = node.advert_frame(1, &[]);
    let own_hash = node.my_hash();
    let mut peer = TucketNode::new(LocalIdentity::from_seed([4; 32]), false);
    peer.on_frame(&advert);
    peer.set_route(own_hash, tucket::node::DirectRoute::new(0, &[]).unwrap());
    let mut runtime = Runtime::tucket_only(
        0,
        config,
        TucketInstance::new(node, tucket::instance::InstanceConfig::new(2, 20).unwrap()),
    )
    .unwrap();
    assert!(runtime.installed_retinue().is_none());
    assert!(runtime.installed_sennet().is_none());
    runtime.ingest(1, &peer.advert_frame(1, &[])).unwrap();
    let (text, expected) = peer.text_frame(own_hash, 2, "one scoped key").unwrap();
    runtime.ingest(2, &text).unwrap();
    assert_eq!(runtime.next_deadline(), Some(202));
    assert!(runtime.begin_tx(201).unwrap().is_none());
    let tx = runtime.begin_tx(202).unwrap().unwrap();
    let (events, _) = peer.on_frame(&tx.frame);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, tucket::node::Event::Ack(ack) if *ack == expected))
    );
    runtime.complete_tx(203, tx.id, true).unwrap();
    for target in [RETINUE, SENNET] {
        assert!(
            runtime
                .request(
                    204,
                    Excursion {
                        target,
                        duration_ms: 10,
                        interruption: InterruptionPolicy::ResumableOnly
                    },
                    || [0; 16]
                )
                .is_err()
        );
        assert_eq!(runtime.active().unwrap().instance, TUCKET);
    }
    assert!(runtime.profile(RETINUE).is_err());
}
