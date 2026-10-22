use super::*;
use crate::destination::DestinationName;
use crate::identity::PrivateIdentity;
use crate::link::{LinkMode, LinkTrailer, PendingLink, accept};

#[test]
fn packet_limits_preserve_the_wrong_destination_gate() {
    let identity = PrivateIdentity::from_secret_bytes(&[0x22; 64]);
    let trailer = LinkTrailer {
        mode: LinkMode::Aes256Cbc,
        mtu: 500,
    };
    let destination =
        DestinationName::new("retinue", ["bounded-packet"]).destination_hash(identity.public());
    let (pending, request) =
        PendingLink::open(destination, *identity.public(), &[0x33; 64], trailer);
    let (sender, proof) = accept(&request, &identity, &[0x44; 64], trailer).unwrap();
    let receiver = pending.prove(&proof).unwrap();
    let response = Response::new(AddressHash::from_bytes([3; 16]), b"data".to_vec());
    let mut packet = sender.response_packet(&response.pack(), &[5; 16]);
    let mut wrong = packet.clone();
    wrong.destination = AddressHash::from_bytes([0xff; 16]);
    let sealed = wrong.payload.clone();
    assert!(
        receive_bounded_packet(
            &receiver,
            &mut wrong,
            Some(ResourceReceiveLimits {
                max_encoded_bytes: 1,
                max_decoded_bytes: 1,
            })
        )
        .unwrap()
        .is_none(),
        "wrong destination is ignored before any limit or decode"
    );
    assert_eq!(wrong.payload, sealed);
    let Some(Inbound::Response(bytes)) = receive_bounded_packet(
        &receiver,
        &mut packet,
        Some(ResourceReceiveLimits::default()),
    )
    .unwrap() else {
        panic!("matching response remains admitted");
    };
    assert_eq!(Response::unpack(&bytes).unwrap(), response);
    assert!(
        packet.payload.is_empty(),
        "the encrypted buffer was consumed in place"
    );
}
