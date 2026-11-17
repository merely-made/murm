//! Independent stock RNS 1.6.0 wire observations. No private material is retained.
use retinue::announce::{self, AnnounceBlob};
use retinue::network_stamp::{self, NetworkStamp};
use retinue::packet::Packet;
use retinue::stamp::StampCost;
use retinue::{AddressHash, Announce, DestinationName, PrivateIdentity};
use serde_json::Value;
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/network_stamps_1_6_0.json")).unwrap()
}
fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}
fn cost(value: f64) -> StampCost {
    StampCost::from_f64(value).unwrap()
}

#[test]
fn stock_signed_extension_preserves_ratchets_and_application_data() {
    for entry in fixture()["announces"].as_array().unwrap() {
        let packet = Packet::decode(&bytes(&entry["raw"])).unwrap();
        let mut announce = Announce::decode(&packet).unwrap();
        assert!(announce.apply_stamp_policy(cost(4.5), true));
        assert_eq!(
            announce.app_data,
            entry["app"]
                .as_str()
                .map(hex::decode)
                .transpose()
                .unwrap()
                .unwrap_or_default()
        );
        assert_eq!(
            announce.ratchet.map(|v| hex::encode(v)),
            entry["ratchet"].as_str().map(String::from)
        );
        assert_eq!(announce.network_stamp.unwrap().link_cost, cost(3.5));
        assert!(announce.stamp_valid(cost(4.5)));
        assert!(!announce.stamp_valid(cost(20.0)));
        let mut tampered = packet.clone();
        *tampered.payload.last_mut().unwrap() ^= 1;
        assert!(
            Announce::decode(&tampered).is_err(),
            "link cost is authenticated"
        );
        let mut weak = Announce::decode(&packet).unwrap();
        assert!(weak.apply_stamp_policy(cost(20.0), false));
        assert!(weak.network_stamp.is_none());
        let overhead =
            retinue::announce::MIN_PAYLOAD_LEN + if packet.context_flag { 32 } else { 0 };
        assert_eq!(weak.app_data, packet.payload[overhead..]);
    }
}

#[test]
fn stock_path_and_embedded_link_stamps_bind_destination_and_tag_or_signing_key() {
    let f = fixture();
    let entry = &f["path_request"];
    let target = AddressHash::from_slice(&bytes(&entry["target"])).unwrap();
    let mut packet = retinue::path::path_request(target, &[0; 16]);
    packet.payload = bytes(&entry["body"]);
    let request = retinue::path::PathRequest::parse(&packet).unwrap();
    assert!(request.stamp_valid(cost(2.5)));
    packet.payload[32] ^= 1;
    assert!(
        !retinue::path::PathRequest::parse(&packet)
            .unwrap()
            .stamp_valid(cost(2.5))
    );
    let entry = &f["link_request"];
    let (_, mut packet) = retinue::link::PendingLink::open(
        target,
        *PrivateIdentity::from_secret_bytes(&[9; 64]).public(),
        &[8; 64],
        retinue::link::LinkTrailer {
            mode: retinue::link::LinkMode::Aes256Cbc,
            mtu: 500,
        },
    );
    packet.payload[..32].copy_from_slice(&bytes(&entry["public"]));
    packet.payload[32..64].copy_from_slice(&bytes(&entry["sig_public"]));
    assert!(retinue::link::request_stamp_valid(&packet, cost(3.5)));
    packet.payload[32] ^= 1;
    assert!(!retinue::link::request_stamp_valid(&packet, cost(3.5)));
}

#[test]
fn extension_recognition_keeps_unknown_future_formats_as_application_data() {
    let id = PrivateIdentity::from_secret_bytes(&[42; 64]);
    let name = DestinationName::new("retinue", ["unknown-stamp"]);
    for suffix in [
        [0xc1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80],
        [0xc1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 101],
    ] {
        let packet = announce::build(
            &id,
            name.name_hash(),
            &AnnounceBlob::from_wire([1; 10]),
            None,
            &suffix,
        );
        let a = Announce::decode(&packet).unwrap();
        assert!(a.network_stamp.is_none());
        assert_eq!(a.app_data, suffix);
    }
}

#[test]
fn detached_generation_exhaustion_and_cancellation_return_no_stamp() {
    let material = b"bounded job";
    assert_eq!(
        network_stamp::generate_detached(material, cost(100.0), 0, |_| {}, || false),
        None
    );
    let calls = std::cell::Cell::new(0);
    assert_eq!(
        network_stamp::generate_detached(
            material,
            cost(100.0),
            10,
            |_| calls.set(calls.get() + 1),
            || true
        ),
        None
    );
    assert_eq!(calls.get(), 0);
    let stamp =
        network_stamp::generate_detached(material, cost(0.0), 1, |out| out.fill(7), || false)
            .unwrap();
    assert!(retinue::stamp::detached_valid(material, &stamp, cost(0.0)));
    let ext = NetworkStamp {
        stamp,
        link_cost: cost(3.5),
    };
    assert_eq!(NetworkStamp::decode(&ext.encode()), Some(ext));
}
