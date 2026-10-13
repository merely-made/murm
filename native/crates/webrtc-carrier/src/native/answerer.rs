// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The native answerer: bind a socket, answer one offer, serve one channel.
//!
//! The browser offers and the native host answers, which is why this half is an
//! answerer and not a symmetric peer. It does three things and stops:
//!
//! 1. binds a UDP socket and declares it as an ICE host candidate;
//! 2. turns one SDP offer into one SDP answer;
//! 3. hands the negotiated `Rtc` to the Sans-IO driver.
//!
//! Signalling is deliberately absent. C1 says to use copy/paste or a loopback
//! fixture, so [`answer`](Answerer::answer) takes and returns SDP *text* and
//! has no opinion about how it travels. A Worker relaying that text is not a
//! carrier fact, which is the rule the plan's boundaries section states.
//!
//! ## Interface discovery is the caller's
//!
//! str0m states plainly that it has no discovery of local or NATed addresses,
//! and this crate adds none: a dependency that enumerates interfaces would be
//! the first thing in here that cannot build for the browser half. Binding to a
//! concrete address gives you a candidate for free; binding to an unspecified
//! one requires [`AnswererConfig::advertise`], and binding to an unspecified
//! address with nothing to advertise is an error rather than a session that
//! silently never connects.

use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use str0m::change::SdpOffer;
use str0m::crypto::Fingerprint;
use str0m::{Candidate, Rtc, RtcConfig};
use tokio::net::UdpSocket;

use crate::error::FingerprintError;
use crate::fingerprint::{
    DTLS_FINGERPRINT_BYTES, DtlsFingerprint, FINGERPRINT_ALGORITHM, FingerprintRole,
};
use crate::native::error::NativeError;
use crate::native::session::{Carrier, CarrierConfig, serve_advertised};

/// How the answerer binds and what it advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswererConfig {
    /// The address to bind the carrier's UDP socket to.
    pub bind: SocketAddr,
    /// Extra IP addresses to declare as host candidates, on the bound port.
    ///
    /// Required when [`bind`](Self::bind) names an unspecified address.
    pub advertise: Vec<IpAddr>,
    /// How the session behaves once the channel is up.
    ///
    /// [`CarrierConfig::placement`] rides along untouched, so the answerer
    /// *allows* a dedicated driver thread and a raised SCTP window without
    /// choosing either: `carrier: CarrierConfig::dedicated(stack, window)` is
    /// the whole recipe, and leaving it at [`Default`] keeps today's
    /// `tokio::spawn` and today's 16 KiB window.
    pub carrier: CarrierConfig,
}

impl Default for AnswererConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            advertise: Vec::new(),
            carrier: CarrierConfig::default(),
        }
    }
}

/// Canonicalizes a str0m fingerprint into the core's role-tagged form.
///
/// str0m hands back an algorithm name and a digest; the core wants a role and
/// exactly 32 bytes. A fingerprint under any other hash function, or of any
/// other width, is refused here rather than truncated into a transcript — the
/// link challenge is the only thing standing between the session and a
/// signalling intermediary that terminated two DTLS sessions.
pub fn fingerprint_from_str0m(
    role: FingerprintRole,
    fingerprint: &Fingerprint,
) -> Result<DtlsFingerprint, FingerprintError> {
    if !fingerprint
        .hash_func
        .eq_ignore_ascii_case(FINGERPRINT_ALGORITHM)
    {
        return Err(FingerprintError::Algorithm {
            got: fingerprint.hash_func.clone(),
        });
    }
    let digest: [u8; DTLS_FINGERPRINT_BYTES] =
        fingerprint
            .bytes
            .as_slice()
            .try_into()
            .map_err(|_| FingerprintError::OctetCount {
                got: fingerprint.bytes.len(),
            })?;
    Ok(DtlsFingerprint::new(role, digest))
}

/// A bound, un-negotiated native answerer.
pub struct Answerer {
    rtc: Rtc,
    socket: UdpSocket,
    local_addr: SocketAddr,
    /// The first address actually accepted as a local ICE candidate.
    ///
    /// Kept because it, not [`local_addr`](Self::local_addr), is what str0m
    /// must be told a datagram was addressed to: the ICE agent matches a STUN
    /// request's destination against its local candidates by exact equality,
    /// and a socket bound to `0.0.0.0` is never one of them. See
    /// [`serve_advertised`].
    candidate_addr: SocketAddr,
    carrier: CarrierConfig,
    answered: bool,
}

impl Answerer {
    /// Binds the carrier's socket and prepares the peer connection.
    ///
    /// No packet is sent and no task is spawned: the answerer is inert until
    /// [`accept`](Self::accept) is called.
    pub async fn bind(config: AnswererConfig) -> Result<Self, NativeError> {
        let socket = UdpSocket::bind(config.bind)
            .await
            .map_err(|source| NativeError::Bind {
                addr: config.bind,
                source,
            })?;
        let local_addr = socket.local_addr().map_err(NativeError::Socket)?;

        // No media: this carrier exists for one data channel, and every codec
        // left enabled is an m-line in an SDP nobody reads.
        let mut rtc = RtcConfig::new().clear_codecs().build(Instant::now());

        let mut addresses: Vec<SocketAddr> = Vec::new();
        if !local_addr.ip().is_unspecified() {
            addresses.push(local_addr);
        }
        for ip in &config.advertise {
            addresses.push(SocketAddr::new(*ip, local_addr.port()));
        }
        if addresses.is_empty() {
            return Err(NativeError::NoCandidate(format!(
                "bound to the unspecified address {local_addr}: supply `advertise` addresses, \
                 this crate does no interface discovery"
            )));
        }

        let mut candidate_addr: Option<SocketAddr> = None;
        for address in &addresses {
            let candidate = Candidate::host(*address, "udp")
                .map_err(|err| NativeError::NoCandidate(err.to_string()))?;
            if rtc.add_local_candidate(candidate).is_some() {
                candidate_addr.get_or_insert(*address);
            }
        }
        let Some(candidate_addr) = candidate_addr else {
            return Err(NativeError::NoCandidate(format!(
                "the peer connection accepted none of {addresses:?}"
            )));
        };

        Ok(Self {
            rtc,
            socket,
            local_addr,
            candidate_addr,
            carrier: config.carrier,
            answered: false,
        })
    }

    /// The address the carrier's socket is actually bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The address this answerer's ICE candidates declare.
    ///
    /// The same as [`local_addr`](Self::local_addr) for a concrete bind. For a
    /// wildcard bind it is the first [`AnswererConfig::advertise`] address on
    /// the bound port, which is the address arriving datagrams are attributed
    /// to — see [`serve_advertised`] for what that costs when more than one
    /// address is advertised.
    pub fn candidate_addr(&self) -> SocketAddr {
        self.candidate_addr
    }

    /// The session policy this answerer will serve under.
    pub fn carrier_config(&self) -> &CarrierConfig {
        &self.carrier
    }

    /// This host's DTLS fingerprint, tagged
    /// [`Server`](FingerprintRole::Server).
    ///
    /// Available before any offer arrives: the certificate is generated when
    /// the peer connection is built, which is what lets a host publish its
    /// fingerprint as part of an invitation rather than only after signalling.
    ///
    /// There is deliberately no matching `remote_fingerprint` here. str0m
    /// publishes the peer's fingerprint only once its certificate has actually
    /// been presented and checked against what the SDP promised, so the value
    /// worth binding does not exist at signalling time. It arrives on the live
    /// session instead, as
    /// [`Carrier::fingerprints`](super::Carrier::fingerprints).
    pub fn local_fingerprint(&mut self) -> Result<DtlsFingerprint, NativeError> {
        let fingerprint = self.rtc.direct_api().local_dtls_fingerprint().clone();
        Ok(fingerprint_from_str0m(
            FingerprintRole::Server,
            &fingerprint,
        )?)
    }

    /// Turns one SDP offer into one SDP answer.
    ///
    /// The candidates declared at [`bind`](Self::bind) travel in the answer,
    /// and the peer's travel in the offer, so no trickle channel is needed for
    /// the direct case C1 covers.
    pub fn answer(&mut self, offer: &str) -> Result<String, NativeError> {
        let offer = SdpOffer::from_sdp_string(offer)
            .map_err(|err| NativeError::Signaling(err.to_string()))?;
        let answer = self
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|err| NativeError::Signaling(err.to_string()))?;
        self.answered = true;
        Ok(answer.to_sdp_string())
    }

    /// Spawns the driver and waits for the data channel.
    ///
    /// Fails with [`NativeError::OpenTimeout`] if no channel with the
    /// configured label opens in time, and cancels the driver rather than
    /// leaving it running.
    ///
    /// Where the driver is spawned is
    /// [`CarrierConfig::placement`](CarrierConfig::placement)'s to say; this
    /// call reads the same either way, and so does the
    /// [`CarrierControl`](super::CarrierControl) it hands back.
    pub async fn accept(self) -> Result<Carrier, NativeError> {
        if !self.answered {
            return Err(NativeError::Signaling(
                "no offer has been answered yet".to_owned(),
            ));
        }
        serve_advertised(self.rtc, self.socket, self.carrier, self.candidate_addr).await
    }
}
