//! Upstream negotiation probes for Genet's WebRTC evaluation.
//! These are native protocol/API observations, not browser or WPT receipts.

#![forbid(unsafe_code)]

#[cfg(all(test, feature = "rtc-core"))]
mod rtc_core {
    use std::time::Instant;

    use rtc::data_channel::RTCDataChannelInit;
    use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
    use rtc::peer_connection::{RTCPeerConnection, RTCPeerConnectionBuilder};
    use rtc::sansio::Protocol;

    fn peer() -> RTCPeerConnection {
        RTCPeerConnectionBuilder::new()
            .build(Instant::now())
            .expect("a peer connection with the explicitly selected crypto provider")
    }

    #[test]
    fn offer_answer_preserves_pending_and_current_descriptions() {
        let now = Instant::now();
        let mut offerer = peer();
        let mut answerer = peer();
        offerer.create_data_channel("data", None).unwrap();
        assert!(offerer.local_description().is_none());

        let offer = offerer.create_offer(None).unwrap();
        assert!(
            offerer.local_description().is_none(),
            "creating is not applying"
        );
        offerer.set_local_description(now, offer.clone()).unwrap();
        assert!(offerer.current_local_description().is_none());
        assert_eq!(
            offerer.pending_local_description().unwrap().sdp_type,
            RTCSdpType::Offer
        );
        answerer.set_remote_description(now, offer).unwrap();

        let answer = answerer.create_answer(None).unwrap();
        answerer.set_local_description(now, answer.clone()).unwrap();
        offerer.set_remote_description(now, answer).unwrap();
        assert!(offerer.pending_local_description().is_none());
        assert!(answerer.pending_remote_description().is_none());
        assert_eq!(
            offerer.current_local_description().unwrap().sdp_type,
            RTCSdpType::Offer
        );
        assert_eq!(
            answerer.current_local_description().unwrap().sdp_type,
            RTCSdpType::Answer
        );
        offerer.close().unwrap();
        answerer.close().unwrap();
    }

    #[test]
    fn rollback_clears_pending_offer_and_allows_a_fresh_offer() {
        let now = Instant::now();
        let mut pc = peer();
        pc.create_data_channel("rollback", None).unwrap();
        let offer = pc.create_offer(None).unwrap();
        pc.set_local_description(now, offer).unwrap();
        let rollback = RTCSessionDescription::rollback(None).unwrap();
        pc.set_local_description(now, rollback).unwrap();
        assert!(pc.pending_local_description().is_none());
        assert!(pc.current_local_description().is_none());
        let next = pc.create_offer(None).unwrap();
        pc.set_local_description(now, next).unwrap();
        pc.close().unwrap();
    }

    #[test]
    fn invalid_channel_options_and_closed_connection_are_refused() {
        let mut pc = peer();
        let conflicting = RTCDataChannelInit {
            max_packet_life_time: Some(10),
            max_retransmits: Some(1),
            ..Default::default()
        };
        assert!(
            pc.create_data_channel("invalid", Some(conflicting))
                .is_err()
        );
        pc.close().unwrap();
        assert!(pc.create_offer(None).is_err());
        assert!(pc.create_data_channel("closed", None).is_err());
    }
}

#[cfg(all(test, feature = "str0m-core"))]
mod str0m_core {
    use std::time::Instant;

    use str0m::channel::ChannelConfig;
    use str0m::{Output, Rtc, RtcConfig};

    fn drain(pc: &mut Rtc) {
        for _ in 0..1000 {
            if matches!(pc.poll_output().unwrap(), Output::Timeout(_)) {
                return;
            }
        }
        panic!("bounded engine output drain did not reach its next timeout");
    }

    #[test]
    fn native_data_channel_offer_answer_uses_the_existing_crypto_configuration() {
        let now = Instant::now();
        let mut offerer = RtcConfig::new().clear_codecs().build(now);
        let mut answerer = RtcConfig::new().clear_codecs().build(now);
        let mut changes = offerer.sdp_api();
        changes.add_channel_with_config(ChannelConfig {
            label: "data".into(),
            ..Default::default()
        });
        let (offer, pending) = changes.apply().unwrap();
        drain(&mut offerer);
        let answer = answerer.sdp_api().accept_offer(offer).unwrap();
        drain(&mut answerer);
        offerer.sdp_api().accept_answer(pending, answer).unwrap();
        drain(&mut offerer);
        offerer.disconnect();
        drain(&mut offerer);
        answerer.disconnect();
        drain(&mut answerer);
    }
}

#[cfg(all(test, feature = "rtc-core", feature = "str0m-core"))]
mod sdp_interop {
    use std::time::Instant;

    use rtc::peer_connection::RTCPeerConnectionBuilder;
    use rtc::peer_connection::sdp::RTCSessionDescription;
    use rtc::sansio::Protocol;
    use str0m::change::{SdpAnswer, SdpOffer};
    use str0m::{Output, Rtc, RtcConfig};

    fn drain(pc: &mut Rtc) {
        for _ in 0..1000 {
            if matches!(pc.poll_output().unwrap(), Output::Timeout(_)) {
                return;
            }
        }
        panic!("bounded engine output drain did not reach its next timeout");
    }

    #[test]
    fn str0m_offer_is_accepted_by_rtc_and_its_answer_by_str0m() {
        let now = Instant::now();
        let mut offerer = RtcConfig::new().clear_codecs().build(now);
        let mut answerer = RTCPeerConnectionBuilder::new().build(now).unwrap();
        let mut changes = offerer.sdp_api();
        changes.add_channel("interop".into());
        let (offer, pending) = changes.apply().unwrap();
        drain(&mut offerer);

        answerer
            .set_remote_description(
                now,
                RTCSessionDescription::offer(offer.to_sdp_string()).unwrap(),
            )
            .unwrap();
        let answer = answerer.create_answer(None).unwrap();
        answerer.set_local_description(now, answer.clone()).unwrap();
        offerer
            .sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(&answer.sdp).unwrap())
            .unwrap();
        drain(&mut offerer);
        answerer.close().unwrap();
        offerer.disconnect();
        drain(&mut offerer);
    }

    #[test]
    fn rtc_offer_is_accepted_by_str0m_and_its_answer_by_rtc() {
        let now = Instant::now();
        let mut offerer = RTCPeerConnectionBuilder::new().build(now).unwrap();
        let mut answerer = RtcConfig::new().clear_codecs().build(now);
        offerer.create_data_channel("interop", None).unwrap();
        let offer = offerer.create_offer(None).unwrap();
        offerer.set_local_description(now, offer.clone()).unwrap();
        let answer = answerer
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&offer.sdp).unwrap())
            .unwrap();
        drain(&mut answerer);
        offerer
            .set_remote_description(
                now,
                RTCSessionDescription::answer(answer.to_sdp_string()).unwrap(),
            )
            .unwrap();
        offerer.close().unwrap();
        answerer.disconnect();
        drain(&mut answerer);
    }
}
