//! Synthetic, non-RF pressure on the exact T114 endpoint queue implementation.
use radio_hand::{
    tucket_endpoint::{Endpoint, Event},
    work::WorkError,
};
use tucket::{
    identity::LocalIdentity,
    instance::{Instance, InstanceConfig},
    node::{Node, NodeCapacity},
};
#[inline(never)]
pub fn run(checkpoint: fn()) {
    for _ in 0..4 {
        let instance = Instance::new(
            Node::with_capacity(
                LocalIdentity::from_seed([0x71; 32]),
                false,
                NodeCapacity::new(8, 32).unwrap(),
            )
            .unwrap(),
            InstanceConfig::for_radio(2, 10, 200).unwrap(),
        );
        let mut endpoint = Endpoint::new(0, instance, 10, 1000).unwrap();
        checkpoint();
        for _ in 0..8 {
            assert!(
                endpoint
                    .advertise(0, 1, &[0x5a; 32])
                    .unwrap()
                    .events
                    .is_empty()
            );
        }
        assert!(matches!(
            endpoint.advertise(0, 1, b"").unwrap().events.as_slice(),
            [Event::FrameDropped(WorkError::Full)]
        ));
        assert!(endpoint.ingest(0, &[0; 256]).is_err());
        for _ in 0..8 {
            let tx = endpoint.begin_tx(0).unwrap().unwrap();
            core::hint::black_box(&tx.frame);
            let mut stale = tx.id;
            stale.sequence += 1;
            assert!(endpoint.complete_tx(0, stale, false).is_err());
            assert!(
                matches!(endpoint.complete_tx(0,tx.id,false).unwrap().events.as_slice(),[Event::WorkLost(id)] if *id==tx.id)
            );
        }
        assert!(endpoint.begin_tx(0).unwrap().is_none());
        checkpoint();
    }
}
