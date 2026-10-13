// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The loopback receipt harness: two real carriers over one UDP pair.
//!
//! Nothing here is simulated. [`loopback_pair`] binds two ordinary UDP sockets
//! on `127.0.0.1`, exchanges a real SDP offer and answer, runs a real ICE and
//! DTLS handshake, and opens a real SCTP data channel — the closest a test can
//! get to "a headed browser opened it" without a browser. It began as the
//! private harness of this crate's own `native_loopback` suite and moved here
//! when Graphshell's WebRTC lane needed the same pair for *its* receipts;
//! one harness, however many crates take receipts over it.
//!
//! ## This is a harness, and it panics like one
//!
//! Every failure in here is a broken precondition of the receipt about to be
//! taken, not a condition a caller handles: a loopback socket that will not
//! bind, an answer that will not parse, two ends disagreeing about the DTLS
//! handshake they both ran. Each panic message says which. Production code
//! wanting a carrier goes through [`Answerer`](super::Answerer) and
//! [`serve`](super::serve) directly, where every one of these is a `Result`.

use std::net::SocketAddr;
use std::time::Instant;

use tokio::net::UdpSocket;

use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::channel::{ChannelConfig, Reliability};
use str0m::{Candidate, Rtc, RtcConfig};

use crate::FingerprintRole;
use crate::native::answerer::{Answerer, AnswererConfig};
use crate::native::session::{Carrier, CarrierConfig, serve};

/// A str0m offerer: bound socket, declared candidate, one data channel.
///
/// The browser's role, played by str0m — through this crate's own re-export,
/// so the engine is the exact version the adapter compiled against. Fields are
/// public because receipt code drives them directly: an offer posted over a
/// signaling fixture reads `offer`, and the peer end goes live by handing
/// `rtc` and `socket` to [`serve`].
pub struct LoopbackOfferer {
    /// The engine, mid-negotiation.
    pub rtc: Rtc,
    /// The bound loopback socket the offer's candidate names.
    pub socket: UdpSocket,
    pending: Option<SdpPendingOffer>,
    /// The offer SDP, complete with its host candidate.
    pub offer: String,
}

impl LoopbackOfferer {
    /// Bind a loopback socket and produce an offer with one reliable, ordered
    /// data channel under `label`.
    pub async fn create(label: &str) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("a loopback UDP port");
        let addr: SocketAddr = socket.local_addr().expect("a local address");

        let mut rtc = RtcConfig::new().clear_codecs().build(Instant::now());
        let candidate = Candidate::host(addr, "udp").expect("a host candidate");
        assert!(
            rtc.add_local_candidate(candidate).is_some(),
            "the loopback host candidate must be accepted, or nothing downstream tests anything"
        );

        let mut change = rtc.sdp_api();
        let _channel = change.add_channel_with_config(ChannelConfig {
            label: label.to_owned(),
            ordered: true,
            reliability: Reliability::Reliable,
            ..ChannelConfig::default()
        });
        let (offer, pending) = change.apply().expect("a data channel requires negotiation");
        let offer = offer.to_sdp_string();

        Self {
            rtc,
            socket,
            pending: Some(pending),
            offer,
        }
    }

    /// Apply the answer this offer earned.
    pub fn accept_answer(&mut self, answer: &str) {
        let answer = SdpAnswer::from_sdp_string(answer).expect("the answer parses");
        let pending = self.pending.take().expect("one answer per offer");
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .expect("the answer applies");
    }
}

/// Run one full handshake over loopback and return both live carriers.
///
/// `(answerer, offerer)`. On the way it asserts the facts every receipt over
/// the pair rests on: both ends observed the *same* DTLS handshake, the
/// negotiated server certificate is the one the answerer published before any
/// offer arrived, and the two role-tagged fingerprints are distinct.
pub async fn loopback_pair(config: CarrierConfig) -> (Carrier, Carrier) {
    let mut offerer = LoopbackOfferer::create(&config.channel_label).await;

    let mut answerer = Answerer::bind(AnswererConfig {
        bind: "127.0.0.1:0".parse().expect("a literal address"),
        advertise: Vec::new(),
        carrier: config.clone(),
    })
    .await
    .expect("the answerer binds");

    // The host can publish its own fingerprint before any offer arrives — the
    // certificate exists from construction, which is what lets an invitation
    // carry it.
    let declared = answerer
        .local_fingerprint()
        .expect("the host's own fingerprint is sha-256");
    assert_eq!(declared.role().name(), "server");

    let answer = answerer.answer(&offerer.offer).expect("an SDP answer");
    offerer.accept_answer(&answer);

    let peer_config = CarrierConfig {
        local_dtls_role: FingerprintRole::Client,
        ..config.clone()
    };
    let (answered, offered) = tokio::join!(
        answerer.accept(),
        serve(offerer.rtc, offerer.socket, peer_config)
    );

    let answered = answered.expect("the answerer's channel opens");
    let offered = offered.expect("the offerer's channel opens");

    // The two role-tagged halves the C2 link challenge binds, taken from the
    // handshake that actually ran rather than from the SDP's claims. Both ends
    // must agree on both, or no shared link could ever be derived.
    let host = answered
        .fingerprints()
        .expect("the host observed the handshake");
    let peer = offered
        .fingerprints()
        .expect("the peer observed the handshake");
    assert_eq!(host, peer, "the two ends disagree about the DTLS handshake");
    assert_eq!(
        host.server().digest(),
        declared.digest(),
        "the host's negotiated certificate is the one it published"
    );
    assert_ne!(
        host.client().canonical_bytes(),
        host.server().canonical_bytes(),
        "two distinct certificates must produce two distinct canonical forms"
    );

    (answered, offered)
}
