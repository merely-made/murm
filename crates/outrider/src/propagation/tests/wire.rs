//! Captured request grammar and fractional announce transition boundaries.

use super::*;

#[test]
fn captured_fetch_requests_decode_to_offer_and_selection() {
    let offer =
        hex::decode("93cb41da9a05b2533e92c4109dc1a72883468f57fed571e796e9ce9892c0c0").unwrap();
    assert_eq!(
        decode_get_request(&decode_fetch_request(&offer).unwrap()).unwrap(),
        GetRequest::Offer
    );

    let followup = hex::decode(
        "93cb41da9a060120c7b3c4109dc1a72883468f57fed571e796e9ce989391c420444444444444444444444444444444444444444444444444444444444444444490cd03e8",
    )
    .unwrap();
    assert_eq!(
        decode_get_request(&decode_fetch_request(&followup).unwrap()).unwrap(),
        GetRequest::Fetch {
            wanted: vec![[0x44; 32]],
            handled: Vec::new(),
            limit_kb: Some(FETCH_LIMIT as f64),
        }
    );
}

#[test]
fn fractional_propagation_triplet_supersedes_legacy_ceilings() {
    let Value::Array(mut parts) = decode_one(&captured_announce()).unwrap() else {
        unreachable!()
    };
    parts.push(Value::Array(vec![
        Value::F64(8.5),
        Value::F64(0.25),
        Value::F64(9.75),
    ]));
    let announce =
        PropagationAnnounce::decode(&encode_value(&Value::Array(parts.clone())).unwrap()).unwrap();
    assert_eq!(
        announce.costs.propagation,
        PropagationCost::from_f64(8.5).unwrap()
    );
    assert_eq!(
        announce.costs.flexibility,
        PropagationCost::from_f64(0.25).unwrap()
    );
    assert_eq!(
        announce.costs.peering,
        PropagationCost::from_f64(9.75).unwrap()
    );
    let Value::Array(encoded) = decode_one(&announce.encode().unwrap()).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        encoded[5],
        Value::Array(vec![9.into(), 1.into(), 10.into()])
    );
    assert_eq!(encoded[7], parts[7]);
    parts[7] = Value::Array(vec![Value::F64(f64::NAN), 0.into(), 0.into()]);
    assert!(PropagationAnnounce::decode(&encode_value(&Value::Array(parts)).unwrap()).is_err());
}

#[test]
fn fractional_override_does_not_hide_a_malformed_legacy_triplet() {
    for legacy in [
        Value::Array(vec![]),
        Value::Array(vec!["bad".into(), 1.into(), 9.into()]),
    ] {
        let Value::Array(mut parts) = decode_one(&captured_announce()).unwrap() else {
            unreachable!()
        };
        parts[5] = legacy;
        parts.push(Value::Array(vec![8.5.into(), 1.into(), 9.into()]));
        assert!(PropagationAnnounce::decode(&encode_value(&Value::Array(parts)).unwrap()).is_err());
    }
}

#[test]
fn propagation_flexibility_is_subtracted_before_target_quantization() {
    for (propagation, flexibility, expected) in [
        (8.001, 0.002, 2048),
        (8.002, 0.001, 2048),
        (8.001, 0.003, 2047),
        (8.003, 0.001, 2049),
    ] {
        let mut announce = PropagationAnnounce::decode(&captured_announce()).unwrap();
        announce.costs.propagation = PropagationCost::from_f64(propagation).unwrap();
        announce.costs.flexibility = PropagationCost::from_f64(flexibility).unwrap();
        let encoded = announce.encode().unwrap();
        let restored = PropagationAnnounce::decode(&encoded).unwrap();
        assert_eq!(
            restored.costs, announce.costs,
            "off-grid raw costs must survive wire round-trip"
        );
        let cost = NodePolicy::from_announce(&restored).stamp_floor();
        assert_eq!(cost.to_fp(), expected);
        assert!(cost.accepts_digest(&cost.target_digest()));
        let weaker = StampCost::from_fp(expected - 1).target_digest();
        assert!(
            !cost.accepts_digest(&weaker),
            "one-grid-step weaker digest must be refused"
        );
    }
}
