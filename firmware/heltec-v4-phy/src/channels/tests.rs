use super::*;

#[test]
fn control_status_request_reassembles_and_resynchronizes() {
    let mut request = [0_u8; CONTROL_STATUS_REQUEST_FRAME_LEN];
    ControlStatusRequestV1::new([0; radio_hand::control::CONTROL_STATUS_NONCE_LEN])
        .encode_frame(&mut request)
        .unwrap();
    let mut wire = [0_u8; 2 + CONTROL_STATUS_REQUEST_FRAME_LEN * 2];
    let len = selvage::kiss::encode_into(&request, &mut wire).unwrap();

    let mut parser = ControlFrameStream::new();
    assert!(parser.push(&wire[..1]).is_none());
    assert!(parser.in_frame());
    assert!(parser.push(&wire[1..len - 1]).is_none());
    assert!(parser.in_frame());
    assert!(matches!(
        parser.push(&wire[len - 1..len]),
        Some(ControlDemux::StatusRequest(_))
    ));
    assert!(!parser.in_frame());

    let foreign = [selvage::kiss::FEND, 1, 2, 3, selvage::kiss::FEND];
    assert!(parser.push(&foreign).is_none());
    assert!(parser.push(&wire[..len]).is_some());
}

#[test]
#[cfg(all(feature = "host-usb", not(feature = "host-uart-low-power")))]
fn control_stream_separates_a_signed_command_frame_from_the_diagnostic() {
    use radio_hand::control::MIN_CONTROL_COMMAND_FRAME_LEN;
    // Shape only: a tagged frame of the minimum signed-command length. Verification is
    // the carrier's, and this parser must hand it over without judging it.
    let mut frame = [0x5a_u8; MIN_CONTROL_COMMAND_FRAME_LEN];
    frame[0] = CONTROL_COMMAND_FRAME_TAG;
    let mut wire = [0_u8; 2 + MIN_CONTROL_COMMAND_FRAME_LEN * 2];
    let len = selvage::kiss::encode_into(&frame, &mut wire).unwrap();
    let mut parser = ControlFrameStream::new();
    assert_eq!(parser.push(&wire[..len]), Some(ControlDemux::Command));
    assert_eq!(parser.frame(), &frame[..]);

    // Too short for a signature is not a command, and a foreign tag is not either.
    let mut short = [0x5a_u8; MIN_CONTROL_COMMAND_FRAME_LEN - 1];
    short[0] = CONTROL_COMMAND_FRAME_TAG;
    let len = selvage::kiss::encode_into(&short, &mut wire).unwrap();
    assert!(parser.push(&wire[..len]).is_none());
    frame[0] = 0x00;
    let len = selvage::kiss::encode_into(&frame, &mut wire).unwrap();
    assert!(parser.push(&wire[..len]).is_none());
}

#[test]
fn control_status_demux_preserves_coalesced_ordinary_bytes_and_payload_fend() {
    let mut request = [0_u8; CONTROL_STATUS_REQUEST_FRAME_LEN];
    let nonce = [0x91; radio_hand::control::CONTROL_STATUS_NONCE_LEN];
    ControlStatusRequestV1::new(nonce)
        .encode_frame(&mut request)
        .unwrap();
    let mut wire = [0_u8; 2 + CONTROL_STATUS_REQUEST_FRAME_LEN * 2];
    let len = selvage::kiss::encode_into(&request, &mut wire).unwrap();
    let mut parser = ControlFrameStream::new();
    for byte in b"pre" {
        assert_eq!(parser.demux_byte(true, *byte), ControlDemux::Ordinary);
    }
    assert_eq!(
        parser.demux_byte(false, selvage::kiss::FEND),
        ControlDemux::Ordinary
    );
    for (index, byte) in wire[..len].iter().copied().enumerate() {
        let demux = parser.demux_byte(true, byte);
        if index + 1 == len {
            assert_eq!(
                demux,
                ControlDemux::StatusRequest(ControlStatusRequestV1::new(nonce))
            );
        } else {
            assert_eq!(demux, ControlDemux::Consumed);
        }
    }
    for byte in b"post" {
        assert_eq!(parser.demux_byte(true, *byte), ControlDemux::Ordinary);
    }
}

#[test]
fn timebase_probe_defaults_to_the_shared_lease() {
    assert!(matches!(
        timebase_probe(b"timebase reserve\n"),
        Some(TimebaseProbe::Reserve(Ok(DEFAULT_LEASE)))
    ));
}

#[test]
fn timebase_probe_accepts_a_decimal_lease_and_rejects_bad_input() {
    assert!(matches!(
        timebase_probe(b"timebase reserve 123\r\n"),
        Some(TimebaseProbe::Reserve(Ok(123)))
    ));
    assert!(matches!(
        timebase_probe(b"timebase reserve nope\n"),
        Some(TimebaseProbe::Reserve(Err(())))
    ));
    assert!(matches!(
        timebase_probe(b"timebase reserve 123x\n"),
        Some(TimebaseProbe::Reserve(Err(())))
    ));
    assert!(timebase_probe(b"timebase reserver\n").is_none());
    assert!(matches!(
        timebase_probe(b"timebase\n"),
        Some(TimebaseProbe::Report)
    ));
}

#[test]
fn decimal_is_small_and_deterministic() {
    let mut out = [0_u8; 20];
    let len = decimal(65_536, &mut out);
    assert_eq!(&out[..len], b"65536");
    let len = decimal(0, &mut out);
    assert_eq!(&out[..len], b"0");
}
