// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! # Mere-WebRTC-Carrier
//!
//! The Wasm-clean core of the browser WebRTC carrier: the part a browser and a
//! native host must agree on byte-for-byte before either of them has opened a
//! socket. Package name is `mere-webrtc-carrier`; the lib is `webrtc_carrier`,
//! so consumers write `use webrtc_carrier::...`.
//!
//! It contains five things and deliberately nothing else:
//!
//! - **bounded frames** — a length-prefixed frame with one explicit ceiling,
//!   whose decoder rejects an oversize frame from the length prefix alone;
//! - **invitation identifiers** — [`InviteId`], opaque and fixed width;
//! - **role-tagged fingerprints** — [`DtlsFingerprint`], canonicalized so the
//!   client and server halves of a DTLS handshake are not interchangeable;
//! - **the link challenge** — [`LinkChallenge`], the transcript that binds one
//!   WebRTC connection to one invitation and derives Notochord's 16-byte
//!   `shared_link`;
//! - **the invitation payload and its signing transcripts** — [`InviteV1`],
//!   the bounded, versioned fragment payload the browser WebRTC carrier plan's
//!   C2 phase defines, plus [`challenge_signature_bytes`] and
//!   [`redemption_signing_bytes`], the two domain-separated byte strings a
//!   host signature and a browser redemption proof are computed over.
//!
//! ## Design
//!
//! - **The core computes; it does not generate.** Every nonce and every
//!   fingerprint arrives as an argument. There is no RNG here, no clock, and
//!   no OS: freshness is the runtime adapter's obligation, stated in its
//!   signature rather than hidden behind a core that quietly reaches for
//!   `getrandom`. That is also what makes the default build compile for
//!   `wasm32-unknown-unknown` with no feature coaxing.
//! - **One hash, one domain string.** `sha2` is the only dependency doing
//!   work. [`SHARED_LINK_DOMAIN`] separates the link derivation from every
//!   other use of the same transcript bytes, and
//!   [`LINK_CHALLENGE_VERSION`] versions the transcript itself.
//! - **Both ends share vectors, not a runtime.** The native answerer and the
//!   browser initiator are separate adapters over this one core, and what
//!   holds them together is `tests/vectors.rs` — a fixed transcript with a
//!   fixed expected link, which is the artifact that proves native and Wasm
//!   agree rather than merely both compiling.
//! - **Wrong is refused, not repaired.** A fingerprint in the wrong role
//!   slot, a lowercase SDP fingerprint, an empty protocol, an oversize
//!   frame: each is an error. Nothing here truncates, pads, or coerces its
//!   way to a plausible-looking transcript.
//!
//! ## Layering
//!
//! ```text
//! browser initiator (C1)        native answerer (C1)
//!            \                        /
//!             `--- webrtc-carrier ---'      <- this crate
//!                        |
//!             mere-transport AcceptedSession
//!                 IngressContext::webrtc(shared_link)
//!                        |
//!             Notochord SessionHello / SessionReply
//! ```
//!
//! This crate does not depend on `mere-transport`, `notochord`, or `personae`,
//! and must not grow such a dependency: it is the half that has to build for
//! the browser, and the admission layers above it are reached by handing them
//! a `[u8; 16]`, not by linking them in.
//!
//! ## Status
//!
//! Pre-1.0 (`STAGE = "pre-alpha"`). C0, C1, and C2's core wire types are here:
//! the frame/identifier/fingerprint/challenge core, its vectors, the two
//! feature-gated runtime adapters — `native` (a str0m answerer) and `browser`
//! (a web-sys initiator), both off by default so the default build stays the
//! Wasm-clean core — and [`InviteV1`] plus the host-challenge-signature and
//! redemption-proof transcripts. What C2 still needs above this crate —
//! actually redeeming an invite into a Notochord delegation, one-use
//! enforcement, and the browser/native admission flow itself — is Graphshell
//! and Notochord wiring, not carrier-core wire types, so it is not here.
//! Forced-relay reconnect (C3) is not here yet either.

#![doc(html_root_url = "https://docs.rs/webrtc-carrier/0.0.1")]
#![warn(missing_docs)]
#![warn(unsafe_code)]

mod backpressure;
mod challenge;
mod codec;
mod error;
mod fingerprint;
mod frame;
mod invite;

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
mod browser;
/// The native runtime adapter: a str0m answerer over a tokio UDP socket.
///
/// Feature-gated on `native`, which is off by default. Its dependencies are
/// declared for `cfg(not(target_arch = "wasm32"))` only, so the module is
/// compiled out on a wasm32 target even with the feature on — the same
/// discipline the `browser` half uses in the other direction.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;

pub use crate::backpressure::{
    Backpressure, DEFAULT_HIGH_WATER_BYTES, DEFAULT_LOW_WATER_BYTES, MAX_QUEUED_BYTES,
};
#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub use crate::browser::{
    BrowserError, BrowserInitiator, BrowserInitiatorConfig, BufferedAmountSource, FrameAssembler,
    IceCandidate, IceServer, IceTransportPolicy, SendGate,
};
pub use crate::challenge::{
    LINK_CHALLENGE_VERSION, LinkChallenge, MAX_TRANSCRIPT_FIELD_BYTES, NONCE_BYTES,
    SHARED_LINK_BYTES, SHARED_LINK_DOMAIN,
};
pub use crate::error::{
    BackpressureError, ChallengeError, FingerprintError, FrameError, InviteError, InviteIdError,
};
pub use crate::fingerprint::{
    CANONICAL_FINGERPRINT_BYTES, DTLS_FINGERPRINT_BYTES, DtlsFingerprint, FINGERPRINT_ALGORITHM,
    FingerprintRole,
};
pub use crate::frame::{
    FRAME_HEADER_BYTES, FrameHeader, MAX_FRAME_BYTES, MAX_FRAME_PAYLOAD_BYTES, decode_frame,
    encode_frame, encode_frame_into,
};
pub use crate::invite::{
    HOST_CHALLENGE_SIGNATURE_DOMAIN, INVITE_DESCRIPTOR_DOMAIN, INVITE_FRAGMENT_PREFIX,
    INVITE_ID_BYTES, INVITE_V1_VERSION, InviteId, InviteV1, MAX_INVITE_BYTES,
    REDEMPTION_PROOF_DOMAIN, challenge_signature_bytes, redemption_signing_bytes,
};

/// Release identity, re-exported from Luggage, which owns it.
///
/// Re-exported rather than merely used so a consumer holding an
/// [`InviteV1`] can name the type its
/// [`release`](InviteV1::release) accessor returns without adding a
/// dependency on Luggage. The definition lives in `luggage::release`; this is
/// the same type, not a copy.
pub use luggage::ReleaseRefV1;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lifecycle stage marker.
pub const STAGE: &str = "pre-alpha";
