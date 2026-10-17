//! Real bounded runtime queue and failure accounting; synthetic keys, no RF.
use radio_hand::{
    instances::{Config, Error, Event, Runtime, TUCKET},
    work::WorkError,
};
use selvage::{PhyProfile, personality::*};
use tucket::{identity::LocalIdentity, instance::Instance, node::Node};

/// Eight queued frames, ninth refused, maximum+1 ingress refused, stale
/// completion refused, all eight driver failures accounted, then fresh restart.
#[inline(never)]
pub fn run(checkpoint: fn()) {
    for _ in 0..4 {
        let config = Config {
            controller: ControllerConfig {
                home: TUCKET,
                pin: Some(TUCKET),
                installed: InstalledPersonalitySet::new(&[TUCKET]).unwrap(),
                coverage: CoveragePolicy::AllowGap,
                max_excursion_ms: 1000,
                return_budget_ms: 10,
                max_defer_ms: 100,
                transition_timeout_ms: 10,
            },
            profiles: [PhyProfile::meshcore(906_875_000, 250_000, 10, 5); 3],
            tx_budget_ms: 10,
            frame_ttl_ms: 1000,
        };
        let instance = Instance::new(
            Node::new(LocalIdentity::from_seed([0x71; 32]), false),
            tucket::instance::InstanceConfig::new(2, 100).unwrap(),
        );
        let mut runtime = Runtime::tucket_only(0, config, instance).unwrap();
        checkpoint();
        for _ in 0..8 {
            assert!(
                runtime
                    .advertise_tucket(0, 1, &[0x5a; 32])
                    .unwrap()
                    .events
                    .is_empty()
            );
        }
        let refusal = runtime.advertise_tucket(0, 1, &[0x5a; 32]).unwrap();
        core::hint::black_box(&runtime);
        assert!(refusal.events.iter().any(|e| matches!(
            e,
            Event::FrameDropped {
                reason: WorkError::Full,
                ..
            }
        )));
        assert!(matches!(
            runtime.ingest(0, &[0; 256]),
            Err(Error::FrameTooLong)
        ));
        for _ in 0..8 {
            let tx = runtime.begin_tx(0).unwrap().unwrap();
            // A failed physical driver still receives the complete frame.
            core::hint::black_box(&tx.frame);
            let mut stale = tx.id;
            stale.sequence += 1;
            assert!(runtime.complete_tx(0, stale, false).is_err());
            let report = runtime.complete_tx(0, tx.id, false).unwrap();
            assert!(
                report
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::WorkLost(id) if *id == tx.id))
            );
        }
        assert!(runtime.begin_tx(0).unwrap().is_none());
        checkpoint();
        // Drop then reconstruct, never carrying queued work across a restart.
    }
}
