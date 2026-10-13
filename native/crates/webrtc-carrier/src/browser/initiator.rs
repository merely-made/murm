// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The browser initiator: a `web_sys::RtcPeerConnection` driven from Rust.
//!
//! The browser IS the WebRTC implementation here — this module does not ship
//! a Rust WebRTC stack into Wasm, it drives the one already inside the tab.
//! Measured at 116 KB gzipped versus 1.57 MB for a compiled-in stack
//! (Findings, C0), which is the whole reason the direct route was chosen
//! over the Iroh-in-WebRTC donor.
//!
//! [`BrowserInitiator`] owns one `RtcPeerConnection` and the one ordered,
//! reliable `RtcDataChannel` it creates on it (browser WebRTC carrier plan
//! §4: "an ordered, reliable data channel"). Per [`crate::fingerprint`]'s
//! role split, the browser is always the DTLS client: it offers, so this
//! module exposes [`create_offer`](BrowserInitiator::create_offer) and
//! [`set_remote_answer`](BrowserInitiator::set_remote_answer), never an
//! answer-creating path. Signaling itself — carrying that offer, the native
//! host's answer, and ICE candidates between the two — is deliberately
//! outside this crate for C1: "local copy/paste or a tiny loopback signaling
//! fixture" per plan §4. What crosses that boundary is exactly what
//! [`create_offer`](BrowserInitiator::create_offer),
//! [`set_remote_answer`](BrowserInitiator::set_remote_answer),
//! [`add_remote_ice_candidate`](BrowserInitiator::add_remote_ice_candidate),
//! and [`on_local_ice_candidate`](BrowserInitiator::on_local_ice_candidate)
//! hand to and take from the caller.
//!
//! ## No polling
//!
//! The one thing plan §4 calls out as a hard failure: "a spin/poll loop is a
//! C1 failure condition." [`send_frame`](BrowserInitiator::send_frame) never
//! loops on a timer. When [`SendGate`] says pause, the send future parks on
//! a oneshot registered in [`Inner::wake_waiters`], and the *one* place that
//! resolves it is the `bufferedamountlow` handler set up in
//! [`new`](BrowserInitiator::new) — reachable only because
//! `set_buffered_amount_low_threshold` was set to the policy's low-water
//! mark at construction. No timer exists anywhere in this module.
//!
//! ## No silent hang
//!
//! `onclose`, `onerror`, and a `connectionState` transition to
//! `failed`/`disconnected`/`closed` all funnel through the single
//! [`set_terminal`] function, which does three things exactly once, the
//! first time any of them fires: records the error, wakes every pending
//! `wake_waiters`/`open_waiters` oneshot with it, and invokes the caller's
//! [`on_closed`](BrowserInitiator::on_closed) callback. A send parked
//! waiting for buffer space or channel-open therefore fails with a named
//! [`BrowserError`] the moment the channel dies, rather than parking
//! forever.

use std::cell::RefCell;
use std::rc::Rc;

use futures_channel::oneshot;
use js_sys::Uint8Array;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Event, MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelInit, RtcDataChannelState,
    RtcDataChannelType, RtcIceCandidateInit, RtcIceServer, RtcIceTransportPolicy, RtcOfferOptions,
    RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit,
};

use crate::{Backpressure, encode_frame};

use super::error::BrowserError;
use super::frame_assembler::FrameAssembler;
use super::send_gate::SendGate;
use super::state::{channel_not_open_error, connection_state_error};

/// One remote or local ICE candidate, in the plain form signaling carries.
///
/// A `None` from
/// [`on_local_ice_candidate`](BrowserInitiator::on_local_ice_candidate) is
/// end-of-candidates for this gathering pass — the browser's own
/// `icecandidate` event carries that as a `null` candidate, and this type
/// preserves the distinction rather than collapsing it into an empty
/// string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    /// The `candidate:` SDP attribute line's value.
    pub candidate: String,
    /// The media stream identification tag, if the browser supplied one.
    pub sdp_mid: Option<String>,
    /// The zero-based media section index, if the browser supplied one.
    pub sdp_m_line_index: Option<u16>,
}

/// One STUN/TURN server this connection may use, with optional credentials.
///
/// A bare STUN server needs neither `username` nor `credential`. A TURN
/// server minted with short-term credentials (RFC 8489-style time-limited
/// tokens — e.g. Cloudflare TURN — is the expected C3 case) needs both.
/// `urls` matches `RTCIceServer.urls`'s own grammar of one entry with
/// possibly-redundant URLs for the same server, not one entry per URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceServer {
    /// The server's URL(s) — `stun:`, `turn:`, or `turns:`.
    pub urls: Vec<String>,
    /// TURN username, if the server requires one.
    pub username: Option<String>,
    /// TURN credential (password or short-term token), if the server
    /// requires one.
    pub credential: Option<String>,
}

impl IceServer {
    /// A bare STUN (or unauthenticated TURN) server: one URL, no
    /// credentials.
    pub fn url(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }
}

/// Which ICE candidate types the connection's ICE agent may gather and
/// offer.
///
/// `Relay` is C3's forced-relay knob (plan §6, `2026-08-25_browser_webrtc_carrier_plan.md`:
/// "the selected candidate pair is demonstrably relay-only in the forced
/// case"). With it set, the browser's own ICE agent never gathers a host or
/// server-reflexive candidate in the first place — this is not a filter
/// applied to the candidate list after gathering, it changes what the
/// browser gathers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IceTransportPolicy {
    /// Gather host, server-reflexive, and relay candidates — the ordinary
    /// case, and the browser's own default.
    #[default]
    All,
    /// Gather relay candidates only.
    Relay,
}

impl IceTransportPolicy {
    fn to_web_sys(self) -> RtcIceTransportPolicy {
        match self {
            Self::All => RtcIceTransportPolicy::All,
            Self::Relay => RtcIceTransportPolicy::Relay,
        }
    }
}

/// What [`BrowserInitiator::new`] needs to build one connection.
#[derive(Debug, Clone)]
pub struct BrowserInitiatorConfig {
    /// The data channel's label.
    pub channel_label: String,
    /// The data channel's SCTP subprotocol string — carried in the DCEP
    /// `DATA_CHANNEL_OPEN` message, and the same string
    /// [`LinkChallenge`](crate::LinkChallenge) binds as `protocol` once C2
    /// wires admission in above this transport. Left unset on the wire
    /// (`RtcDataChannelInit` default, the empty string) if empty here.
    pub protocol: String,
    /// STUN/TURN servers, with credentials where the server needs them.
    /// Empty is correct for C1: plan §4 runs against local signaling with no
    /// public infrastructure. C3 is what needs entries here — a same-network
    /// headed receipt may still need a STUN server to learn its own
    /// reflexive candidate, and a forced-relay receipt needs a TURN server
    /// with credentials — left configurable rather than hard-coded to
    /// nothing, because that choice belongs to whoever is running it.
    pub ice_servers: Vec<IceServer>,
    /// Which candidate types the ICE agent may gather. Defaults to
    /// [`IceTransportPolicy::All`]; set to
    /// [`IceTransportPolicy::Relay`] for C3's forced-relay receipt.
    pub ice_transport_policy: IceTransportPolicy,
    /// The backpressure policy this channel enforces.
    pub backpressure: Backpressure,
}

impl Default for BrowserInitiatorConfig {
    fn default() -> Self {
        Self {
            channel_label: "mere-graphshell".to_string(),
            protocol: "mere/graphshell/v1".to_string(),
            ice_servers: Vec::new(),
            ice_transport_policy: IceTransportPolicy::default(),
            backpressure: Backpressure::default(),
        }
    }
}

/// State shared between [`BrowserInitiator`]'s methods and its persistent
/// event closures.
///
/// `Rc<RefCell<_>>` because both sides need to reach it: the closures live as
/// long as the `RtcDataChannel`/`RtcPeerConnection` keep them registered
/// (the lifetime of [`BrowserInitiator`] itself), and they run from JS's
/// event loop, not from a call stack this module controls.
struct Inner {
    gate: SendGate,
    assembler: FrameAssembler,
    /// Set once, by whichever terminal condition fires first. Sticky: a
    /// second `onclose` after `onerror` (or any other combination) must not
    /// overwrite the first reason or fire the close callback twice.
    terminal: Option<BrowserError>,
    wake_waiters: Vec<oneshot::Sender<Result<(), BrowserError>>>,
    open_waiters: Vec<oneshot::Sender<Result<(), BrowserError>>>,
    on_frame: Option<Box<dyn FnMut(Vec<u8>)>>,
    on_closed: Option<Box<dyn FnMut(BrowserError)>>,
    on_local_ice_candidate: Option<Box<dyn FnMut(Option<IceCandidate>)>>,
}

/// A browser-side WebRTC initiator: one `RtcPeerConnection`, one ordered
/// reliable `RtcDataChannel`, framed on the carrier core.
///
/// See the module doc for the no-polling and no-silent-hang guarantees; see
/// [`new`](Self::new) for what it wires up.
pub struct BrowserInitiator {
    peer: RtcPeerConnection,
    channel: RtcDataChannel,
    inner: Rc<RefCell<Inner>>,
    // Closures must outlive every `set_on*` registration that references
    // them, so they live here for the struct's whole lifetime rather than
    // being dropped at the end of `new`.
    _on_channel_open: Closure<dyn FnMut(Event)>,
    _on_channel_close: Closure<dyn FnMut(Event)>,
    _on_channel_error: Closure<dyn FnMut(Event)>,
    _on_channel_message: Closure<dyn FnMut(MessageEvent)>,
    _on_buffered_amount_low: Closure<dyn FnMut(Event)>,
    _on_connection_state_change: Closure<dyn FnMut(Event)>,
    _on_ice_candidate: Closure<dyn FnMut(RtcPeerConnectionIceEvent)>,
}

impl BrowserInitiator {
    /// Builds the peer connection and its one data channel.
    ///
    /// The channel is created here, synchronously, with `ordered(true)` and
    /// neither `maxRetransmits` nor `maxPacketLifeTime` set — the
    /// `RTCDataChannelInit` grammar's only way to ask for "ordered,
    /// reliable" (plan §4). `binary_type` is set to `arraybuffer` so
    /// `onmessage` always hands this module bytes directly, never a `Blob`
    /// that would need an extra async read to get at.
    pub fn new(config: BrowserInitiatorConfig) -> Result<Self, BrowserError> {
        let rtc_config = RtcConfiguration::new();
        if !config.ice_servers.is_empty() {
            // `set_ice_servers` takes one `&JsValue` holding a JS array, not
            // a Rust slice — confirmed by the actual `web-sys` 0.3.104
            // binding (`fn set_ice_servers(this: &RtcConfiguration, val:
            // &JsValue)`), not by the docs.rs summary this was first written
            // against, which described it as `&[JsValue]` and did not
            // compile.
            let servers = js_sys::Array::new();
            for entry in &config.ice_servers {
                servers.push(&JsValue::from(build_rtc_ice_server(entry)));
            }
            rtc_config.set_ice_servers(&JsValue::from(servers));
        }
        // Always set, not just under `Relay`: `All` is the browser's own
        // default, so setting it there is a no-op, and always calling this
        // means the policy in the config and the policy on the wire never
        // drift apart depending on which variant happened to be chosen.
        rtc_config.set_ice_transport_policy(config.ice_transport_policy.to_web_sys());
        let peer = RtcPeerConnection::new_with_configuration(&rtc_config)?;

        let dc_init = RtcDataChannelInit::new();
        dc_init.set_ordered(true);
        if !config.protocol.is_empty() {
            dc_init.set_protocol(&config.protocol);
        }
        let channel =
            peer.create_data_channel_with_data_channel_dict(&config.channel_label, &dc_init);
        channel.set_binary_type(RtcDataChannelType::Arraybuffer);
        channel.set_buffered_amount_low_threshold(config.backpressure.low_water_bytes() as u32);

        let inner = Rc::new(RefCell::new(Inner {
            gate: SendGate::new(config.backpressure),
            assembler: FrameAssembler::new(),
            terminal: None,
            wake_waiters: Vec::new(),
            open_waiters: Vec::new(),
            on_frame: None,
            on_closed: None,
            on_local_ice_candidate: None,
        }));

        let on_open = {
            let inner = inner.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                let waiters = std::mem::take(&mut inner.borrow_mut().open_waiters);
                for waiter in waiters {
                    let _ = waiter.send(Ok(()));
                }
            })
        };
        channel.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let on_close = {
            let inner = inner.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                set_terminal(&inner, BrowserError::ChannelClosed);
            })
        };
        channel.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        let on_error = {
            let inner = inner.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                set_terminal(&inner, BrowserError::ChannelError);
            })
        };
        channel.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        let on_message = {
            let inner = inner.clone();
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                on_channel_message(&inner, event);
            })
        };
        channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_buffered_amount_low = {
            let inner = inner.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                let waiters = {
                    let mut inner = inner.borrow_mut();
                    inner.gate.note_buffered_amount_low();
                    std::mem::take(&mut inner.wake_waiters)
                };
                for waiter in waiters {
                    let _ = waiter.send(Ok(()));
                }
            })
        };
        channel.set_onbufferedamountlow(Some(on_buffered_amount_low.as_ref().unchecked_ref()));

        let on_connection_state_change = {
            let inner = inner.clone();
            let peer = peer.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                if let Some(err) = connection_state_error(peer.connection_state()) {
                    set_terminal(&inner, err);
                }
            })
        };
        peer.set_onconnectionstatechange(Some(on_connection_state_change.as_ref().unchecked_ref()));

        let on_ice_candidate = {
            let inner = inner.clone();
            Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(
                move |event: RtcPeerConnectionIceEvent| {
                    let candidate = event.candidate().map(|c| IceCandidate {
                        candidate: c.candidate(),
                        sdp_mid: c.sdp_mid(),
                        sdp_m_line_index: c.sdp_m_line_index(),
                    });
                    // Take the callback OUT before invoking it. Holding the
                    // `RefMut` across the call keeps `Inner` mutably borrowed
                    // while user code runs, and a browser fires
                    // `onicecandidate` repeatedly during gathering — the
                    // second firing then hits wasm-bindgen's reentrancy guard
                    // ("closure invoked recursively or after being dropped")
                    // and gathering stops dead. `set_terminal` already takes
                    // this shape; this site did not.
                    let mut taken = inner.borrow_mut().on_local_ice_candidate.take();
                    if let Some(callback) = taken.as_mut() {
                        callback(candidate);
                    }
                    let mut guard = inner.borrow_mut();
                    if guard.on_local_ice_candidate.is_none() {
                        guard.on_local_ice_candidate = taken;
                    }
                },
            )
        };
        peer.set_onicecandidate(Some(on_ice_candidate.as_ref().unchecked_ref()));

        Ok(Self {
            peer,
            channel,
            inner,
            _on_channel_open: on_open,
            _on_channel_close: on_close,
            _on_channel_error: on_error,
            _on_channel_message: on_message,
            _on_buffered_amount_low: on_buffered_amount_low,
            _on_connection_state_change: on_connection_state_change,
            _on_ice_candidate: on_ice_candidate,
        })
    }

    /// Creates an offer, sets it as the local description, and returns its
    /// SDP for the signaling channel to carry to the native host.
    pub async fn create_offer(&self) -> Result<String, BrowserError> {
        let value = JsFuture::from(self.peer.create_offer()).await?;
        let sdp = offer_sdp_field(&value)?;
        let local = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        local.set_sdp(&sdp);
        JsFuture::from(self.peer.set_local_description(&local)).await?;
        Ok(sdp)
    }

    /// Creates an ICE-restart offer — a fresh ICE ufrag/pwd for this same
    /// connection — sets it as the local description, and returns its SDP.
    ///
    /// An ICE restart renegotiates only the ICE transport; it does not tear
    /// down or recreate the DTLS association `RTCPeerConnection` already
    /// holds. That makes the result the *same* carrier link under C2's
    /// new-DTLS-new-link rule (`design_docs/mere_docs/implementation_strategy/
    /// 2026-08-25_browser_webrtc_carrier_plan.md` §6: "Treat a new DTLS
    /// connection as a new carrier link and run the host challenge plus
    /// Notochord admission again") — no new DTLS connection is created here,
    /// so no fresh admission is needed. The answering str0m side detects the
    /// changed ufrag/pwd in `accept_offer` and restarts its own ICE agent
    /// transparently (str0m 0.23.1 `src/change/sdp.rs:715-756`).
    ///
    /// The caller carries the returned SDP to the native host and drives new
    /// local/remote ICE candidates the same way an initial
    /// [`create_offer`](Self::create_offer) already requires — a restart
    /// re-gathers candidates the same way the first offer did.
    pub async fn create_restart_offer(&self) -> Result<String, BrowserError> {
        let options = RtcOfferOptions::new();
        options.set_ice_restart(true);
        let value = JsFuture::from(self.peer.create_offer_with_rtc_offer_options(&options)).await?;
        let sdp = offer_sdp_field(&value)?;
        let local = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        local.set_sdp(&sdp);
        JsFuture::from(self.peer.set_local_description(&local)).await?;
        Ok(sdp)
    }

    /// Applies the native host's answer SDP as the remote description.
    pub async fn set_remote_answer(&self, sdp: &str) -> Result<(), BrowserError> {
        let remote = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        remote.set_sdp(sdp);
        JsFuture::from(self.peer.set_remote_description(&remote)).await?;
        Ok(())
    }

    /// Adds one ICE candidate the signaling channel delivered from the
    /// native host.
    pub async fn add_remote_ice_candidate(
        &self,
        candidate: &IceCandidate,
    ) -> Result<(), BrowserError> {
        let init = RtcIceCandidateInit::new(&candidate.candidate);
        init.set_sdp_mid(candidate.sdp_mid.as_deref());
        init.set_sdp_m_line_index(candidate.sdp_m_line_index);
        JsFuture::from(
            self.peer
                .add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init)),
        )
        .await?;
        Ok(())
    }

    /// Tells the peer connection the native host has no more ICE candidates
    /// to offer for this gathering pass.
    pub async fn end_of_remote_candidates(&self) -> Result<(), BrowserError> {
        JsFuture::from(
            self.peer
                .add_ice_candidate_with_opt_rtc_ice_candidate_init(None),
        )
        .await?;
        Ok(())
    }

    /// Registers the callback for locally gathered ICE candidates.
    ///
    /// Fires once per candidate as `onicecandidate` delivers them, and once
    /// more with `None` for end-of-candidates. Replaces any previously
    /// registered callback; only one may be registered at a time, since only
    /// one signaling channel is carrying this connection's candidates out.
    pub fn on_local_ice_candidate(&self, callback: impl FnMut(Option<IceCandidate>) + 'static) {
        self.inner.borrow_mut().on_local_ice_candidate = Some(Box::new(callback));
    }

    /// Registers the callback for received, reassembled frame payloads.
    ///
    /// Called once per complete frame, in arrival order — never with a
    /// declared-oversize frame, which is instead surfaced through
    /// [`on_closed`](Self::on_closed) as `BrowserError::Frame`, because a
    /// peer sending one is a protocol violation this end tears the
    /// connection down over, not a message to hand to the application.
    ///
    /// The callback must not call back into this `BrowserInitiator`
    /// synchronously — it runs with the initiator's shared state already
    /// borrowed, and a reentrant borrow panics. Queue anything of that shape
    /// instead.
    pub fn on_frame(&self, callback: impl FnMut(Vec<u8>) + 'static) {
        self.inner.borrow_mut().on_frame = Some(Box::new(callback));
    }

    /// Registers the callback for the connection's terminal condition.
    ///
    /// Fires exactly once, however it happens — `onclose`, `onerror`, or a
    /// `connectionState` change to `failed`/`disconnected`/`closed` — with
    /// whichever fired first. If the connection is already terminal when
    /// this is called, it fires immediately with that reason, so a late
    /// registration still learns why rather than waiting on an event that
    /// already happened.
    pub fn on_closed(&self, mut callback: impl FnMut(BrowserError) + 'static) {
        let already = self.inner.borrow().terminal.clone();
        match already {
            Some(err) => callback(err),
            None => self.inner.borrow_mut().on_closed = Some(Box::new(callback)),
        }
    }

    /// Waits until the data channel's `readyState` is `open`.
    ///
    /// Resolves immediately if it already is. Resolves with an error —
    /// never hangs — if the connection reaches a terminal condition first.
    pub async fn wait_until_open(&self) -> Result<(), BrowserError> {
        let receiver = {
            let mut inner = self.inner.borrow_mut();
            if let Some(err) = inner.terminal.clone() {
                return Err(err);
            }
            if self.channel.ready_state() == RtcDataChannelState::Open {
                return Ok(());
            }
            let (sender, receiver) = oneshot::channel();
            inner.open_waiters.push(sender);
            receiver
        };
        match receiver.await {
            Ok(result) => result,
            Err(_) => Err(BrowserError::ChannelClosed),
        }
    }

    /// Frames `payload`, waits out backpressure if the gate is paused, and
    /// sends it.
    ///
    /// The wait is the one described in the module doc: never a poll loop,
    /// always a park on the `bufferedamountlow` event. A close or error that
    /// arrives while parked resolves the wait with that error already; the
    /// explicit `readyState` check after the loop covers the remaining race
    /// where the channel closes between the wait clearing and the send call
    /// itself.
    pub async fn send_frame(&self, payload: &[u8]) -> Result<(), BrowserError> {
        let framed = encode_frame(payload)?;
        loop {
            if let Some(err) = self.inner.borrow().terminal.clone() {
                return Err(err);
            }
            let buffered = self.channel.buffered_amount();
            let ready = self.inner.borrow_mut().gate.may_send_now(&buffered);
            if ready {
                break;
            }
            self.wait_for_buffer_space().await?;
        }
        if let Some(err) = channel_not_open_error(self.channel.ready_state()) {
            return Err(err);
        }
        self.channel
            .send_with_u8_array(&framed)
            .map_err(BrowserError::from)
    }

    async fn wait_for_buffer_space(&self) -> Result<(), BrowserError> {
        let receiver = {
            let mut inner = self.inner.borrow_mut();
            if let Some(err) = inner.terminal.clone() {
                return Err(err);
            }
            // Race guard: `buffered_amount` may already have crossed the low
            // mark between the caller's check and this registration, since
            // nothing makes the two atomic with the event that would
            // otherwise wake this. Re-check before parking, or a wait that
            // arrived just after the one `bufferedamountlow` firing it
            // needed would park forever.
            let buffered = self.channel.buffered_amount();
            if inner.gate.may_send_now(&buffered) {
                return Ok(());
            }
            let (sender, receiver) = oneshot::channel();
            inner.wake_waiters.push(sender);
            receiver
        };
        match receiver.await {
            Ok(result) => result,
            Err(_) => Err(BrowserError::ChannelClosed),
        }
    }

    /// Bytes presently queued on the data channel.
    pub fn buffered_amount(&self) -> u32 {
        self.channel.buffered_amount()
    }

    /// Whether [`send_frame`](Self::send_frame) would have to wait right
    /// now.
    pub fn is_send_paused(&self) -> bool {
        self.inner.borrow().gate.is_paused()
    }

    /// Closes the data channel and the peer connection.
    ///
    /// Triggers the same `onclose`/`connectionstatechange` path an ordinary
    /// remote close does, so [`on_closed`](Self::on_closed) still fires
    /// exactly once with a reason — here, `ChannelClosed`, since this is the
    /// local, deliberate half of "close... must... propagate honestly."
    pub fn close(&self) {
        self.channel.close();
        self.peer.close();
    }
}

impl Drop for BrowserInitiator {
    fn drop(&mut self) {
        // Best-effort: a normal drop (the caller replacing or discarding
        // this value while the page keeps running) should not leak the
        // underlying JS objects. This is not what makes a browser refresh
        // clean — refresh tears down the whole JS realm directly, which is
        // also what the *native* side observes, as its own connection-state
        // change; no Rust destructor runs on that path, on this side or any
        // other.
        self.channel.close();
        self.peer.close();
    }
}

/// Reads the `sdp` field off whatever `createOffer()` resolved to.
///
/// `createOffer()` — with or without `RtcOfferOptions` — resolves to an
/// **RTCSessionDescriptionInit**, a plain dictionary `{sdp, type}`, not an
/// `RTCSessionDescription` instance. `dyn_into` checks the prototype, so it
/// fails here and, because the failure carries the value, reports a
/// perfectly good offer as "browser WebRTC call failed". Read the field
/// instead — shared by [`BrowserInitiator::create_offer`] and
/// [`BrowserInitiator::create_restart_offer`], since a restart offer
/// resolves the same dictionary shape.
fn offer_sdp_field(value: &JsValue) -> Result<String, BrowserError> {
    js_sys::Reflect::get(value, &JsValue::from_str("sdp"))
        .map_err(BrowserError::from)?
        .as_string()
        .ok_or_else(|| BrowserError::Js("createOffer resolved without an sdp string".to_owned()))
}

/// Builds one `RtcIceServer` from an [`IceServer`] entry.
fn build_rtc_ice_server(entry: &IceServer) -> RtcIceServer {
    let server = RtcIceServer::new();
    let urls = js_sys::Array::new();
    for url in &entry.urls {
        urls.push(&JsValue::from_str(url));
    }
    server.set_urls(&JsValue::from(urls));
    if let Some(username) = &entry.username {
        server.set_username(username);
    }
    if let Some(credential) = &entry.credential {
        server.set_credential(credential);
    }
    server
}

fn on_channel_message(inner: &Rc<RefCell<Inner>>, event: MessageEvent) {
    let buffer = match event.data().dyn_into::<js_sys::ArrayBuffer>() {
        Ok(buffer) => buffer,
        Err(_) => {
            set_terminal(inner, BrowserError::UnexpectedMessageType);
            return;
        },
    };
    let array = Uint8Array::new(&buffer);
    let mut bytes = vec![0u8; array.length() as usize];
    array.copy_to(&mut bytes);

    let mut guard = inner.borrow_mut();
    let frames = match guard.assembler.push(&bytes) {
        Ok(frames) => frames,
        Err(frame_error) => {
            drop(guard);
            set_terminal(inner, BrowserError::from(frame_error));
            return;
        },
    };
    // Same rule as the ICE handler: the borrow must end before user code
    // runs. A frame callback that touches the initiator would otherwise
    // re-enter a live `borrow_mut` and panic instead of merely being
    // discouraged by a doc comment.
    let mut taken = guard.on_frame.take();
    drop(guard);
    if let Some(callback) = taken.as_mut() {
        for frame in frames {
            callback(frame);
        }
    }
    let mut guard = inner.borrow_mut();
    if guard.on_frame.is_none() {
        guard.on_frame = taken;
    }
}

/// Records the first terminal condition, wakes every parked waiter with it,
/// and fires the close callback — all exactly once, however many of
/// `onclose`/`onerror`/`connectionstatechange` end up firing.
fn set_terminal(inner: &Rc<RefCell<Inner>>, err: BrowserError) {
    let (wake_waiters, open_waiters, on_closed) = {
        let mut guard = inner.borrow_mut();
        if guard.terminal.is_some() {
            return;
        }
        guard.terminal = Some(err.clone());
        (
            std::mem::take(&mut guard.wake_waiters),
            std::mem::take(&mut guard.open_waiters),
            guard.on_closed.take(),
        )
    };
    for waiter in wake_waiters {
        let _ = waiter.send(Err(err.clone()));
    }
    for waiter in open_waiters {
        let _ = waiter.send(Err(err.clone()));
    }
    if let Some(mut callback) = on_closed {
        callback(err);
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn ice_server_maps_urls_username_and_credential() {
        let entry = IceServer {
            urls: vec!["turn:turn.example.invalid:3478".to_string()],
            username: Some("mere-c3".to_string()),
            credential: Some("s3cr3t".to_string()),
        };
        let built = build_rtc_ice_server(&entry);

        let urls =
            js_sys::Array::from(&js_sys::Reflect::get(&built, &JsValue::from_str("urls")).unwrap());
        assert_eq!(urls.length(), 1);
        assert_eq!(
            urls.get(0).as_string().as_deref(),
            Some("turn:turn.example.invalid:3478")
        );

        let username = js_sys::Reflect::get(&built, &JsValue::from_str("username")).unwrap();
        assert_eq!(username.as_string().as_deref(), Some("mere-c3"));

        let credential = js_sys::Reflect::get(&built, &JsValue::from_str("credential")).unwrap();
        assert_eq!(credential.as_string().as_deref(), Some("s3cr3t"));
    }

    #[wasm_bindgen_test]
    fn bare_stun_server_carries_no_credentials() {
        let entry = IceServer::url("stun:stun.example.invalid:19302");
        assert_eq!(entry.urls, vec!["stun:stun.example.invalid:19302"]);
        let built = build_rtc_ice_server(&entry);

        let username = js_sys::Reflect::get(&built, &JsValue::from_str("username")).unwrap();
        assert!(username.is_undefined());
        let credential = js_sys::Reflect::get(&built, &JsValue::from_str("credential")).unwrap();
        assert!(credential.is_undefined());
    }

    #[wasm_bindgen_test]
    fn ice_transport_policy_maps_to_web_sys() {
        assert_eq!(
            IceTransportPolicy::All.to_web_sys(),
            RtcIceTransportPolicy::All
        );
        assert_eq!(
            IceTransportPolicy::Relay.to_web_sys(),
            RtcIceTransportPolicy::Relay
        );
    }

    #[wasm_bindgen_test]
    fn ice_transport_policy_defaults_to_all() {
        assert_eq!(IceTransportPolicy::default(), IceTransportPolicy::All);
        assert_eq!(
            BrowserInitiatorConfig::default().ice_transport_policy,
            IceTransportPolicy::All
        );
    }

    // Compile-level receipt that `create_restart_offer` is exported with the
    // signature callers expect. Never called: a real ICE restart needs an
    // `RtcPeerConnection` that already has a local and remote description
    // set, which is a headed two-peer receipt (module doc), not something a
    // unit test can stand up.
    #[allow(dead_code)]
    fn create_restart_offer_is_exported(
        initiator: &BrowserInitiator,
    ) -> impl std::future::Future<Output = Result<String, BrowserError>> + '_ {
        initiator.create_restart_offer()
    }
}
