//! The page half of the C1 headed receipt.
//!
//! Earns plan §4's first done-condition — *"a headed ordinary browser opens,
//! exchanges bounded ping frames, and closes cleanly against a native
//! process"* — by driving [`webrtc_carrier::BrowserInitiator`] end to end from
//! a real tab and writing every stage into the DOM in text a screenshot can
//! read.
//!
//! ## The sequence, and why it is this one
//!
//! 1. **Build the initiator**, then register the ICE-candidate callback
//!    *before* offering. Gathering does not start until `setLocalDescription`,
//!    so registering first is what makes the end-of-gathering `None` reachable
//!    rather than a race.
//! 2. **`create_offer()`**, which returns the SDP as it stood *before* any
//!    candidate was gathered.
//! 3. **Wait for gathering to finish**, then splice the gathered candidates
//!    into that SDP as `a=candidate:` lines. C1 has no trickle channel — one
//!    POST carries the offer and one response carries the answer — so the
//!    offer that travels has to be the complete one.
//! 4. **POST it** to the native fixture as `text/plain`, which keeps it a CORS
//!    *simple* request: no preflight, so the fixture needs no `OPTIONS`
//!    handling to work (it answers `OPTIONS` anyway, for anyone who changes
//!    the content type).
//! 5. **`set_remote_answer`**, **`wait_until_open`**, then bounded ping
//!    frames.
//!
//! ## Candidates this half deliberately drops
//!
//! Chrome obfuscates host candidates behind mDNS `.local` hostnames unless the
//! page holds a camera or microphone grant, and it gathers TCP candidates the
//! native answerer's UDP socket can never pair with. Neither is one str0m can
//! use: it resolves no hostnames and speaks UDP only.
//!
//! Measured, not assumed — str0m 0.23.1 answers a hand-posted offer carrying
//! an mDNS candidate, and answers one carrying no candidates at all, so this
//! filter is *not* what keeps the offer parseable. It is what keeps the page's
//! candidate accounting truthful: a `.local` line shipped to str0m is a
//! candidate the page would count as offered and the answerer could never
//! pair with.
//!
//! Dropping them costs nothing either way. The native end advertises its own
//! host candidates in the answer, the browser runs its checks against those,
//! and str0m learns this end's address peer-reflexively from the binding
//! request that arrives. The kept/dropped counts go on the page, because which
//! candidates were on the table is one of the facts the receipt has to record.
//!
//! ## Three rules the callbacks obey
//!
//! `on_frame` runs with the initiator's shared state already mutably borrowed
//! (`browser::initiator::on_channel_message` calls it inside the guard), so it
//! touches nothing but this module's own counters and the DOM.
//!
//! Nothing ever *drops* a live [`BrowserInitiator`]: its `Drop` closes the JS
//! objects but does not unregister their handlers first, and the `Closure`s go
//! with it, so a late `onclose` would be invoking a dropped closure. Retired
//! sessions are parked in `RETIRED` and kept alive for the life of the page —
//! a few hundred bytes per session in a probe, in exchange for a console that
//! stays clean enough to screenshot.
//!
//! Because retired sessions stay registered, every stats-writing callback is
//! gated on its own session's `active` flag. A page holds one subject at a
//! time; a session that is no longer it must not stamp `terminal`,
//! a revision, or a frame count onto the session that replaced it. Without the
//! gate a reconnect races its predecessor's `onclose`, and the loser is the
//! readout a screenshot records.
//!
//! ## Reconnect is a new link, not a restart
//!
//! [`webrtc_carrier::BrowserInitiator::create_restart_offer`] renegotiates ICE
//! in place and keeps the DTLS association, which would make a reconnect the
//! *same* carrier link. This probe cannot use it, because the fixture it talks
//! to answers every offer from a freshly bound `Answerer` — new socket, new
//! certificate, new fingerprint. A changed fingerprint tears the DTLS/SCTP
//! association down, and an `RTCDataChannel` is not recreated when SCTP
//! restarts; `BrowserInitiator` opens its channel once, in `new`. So the old
//! channel closes for good and there is no new one on that peer connection to
//! wait for.
//!
//! The `reconnect` export therefore stands up a whole new initiator and carries
//! only the revision across, which is what makes the resume handshake the thing
//! that provides continuity. This is plan §6's rule earning itself: a reconnect
//! here really is a new DTLS connection, so it really does need fresh
//! admission.
//!
//! ## No polling, no hidden hangs
//!
//! The gathering wait is a `Promise` resolved by the end-of-candidates
//! callback, raced against one `setTimeout` so a browser that never signals
//! completion degrades to "offer with whatever we have" instead of a page that
//! sits at *gathering* forever. Nothing here spins.

#![warn(unsafe_code)]

/// The offer-assembly rules, kept out of the wasm-only module on purpose.
///
/// They are pure string work with no browser in them, so they compile and
/// their tests *run* on an ordinary `cargo test` — which is the difference
/// between an assertion and a comment. Getting either wrong is a silent
/// failure in the headed run (an offer str0m rejects, or one carrying no
/// candidates at all), so they are the two pieces of this probe worth pinning
/// down before a browser is ever opened.
pub mod sdp {
    /// Whether this candidate is one the native answerer can actually parse.
    ///
    /// The candidate-attribute grammar is `candidate:<foundation> <component>
    /// <transport> <priority> <address> <port> typ <type> ...`, so field 2 is
    /// the transport and field 4 the connection address. A `.local` mDNS name
    /// or a non-UDP transport is dropped — see the crate doc for why that
    /// costs nothing.
    pub fn candidate_is_usable(candidate: &str) -> bool {
        let fields: Vec<&str> = candidate.split_whitespace().collect();
        if fields.len() < 6 || !fields[2].eq_ignore_ascii_case("udp") {
            return false;
        }
        let address = fields[4];
        !address.to_ascii_lowercase().ends_with(".local")
            && address.parse::<std::net::IpAddr>().is_ok()
    }

    /// Splices gathered candidates into the offer as media-level attributes.
    ///
    /// There is exactly one m-section (the data channel), so appending is
    /// unambiguous: every attribute after the last `m=` line belongs to it.
    pub fn offer_with_candidates(offer: &str, candidates: &[String]) -> String {
        let mut sdp = offer.to_owned();
        if !sdp.ends_with('\n') {
            sdp.push_str("\r\n");
        }
        for candidate in candidates {
            sdp.push_str("a=");
            sdp.push_str(candidate);
            sdp.push_str("\r\n");
        }
        sdp
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mdns_and_tcp_candidates_are_dropped() {
            assert!(candidate_is_usable(
                "candidate:1 1 udp 2130706431 192.168.1.20 51820 typ host"
            ));
            assert!(candidate_is_usable(
                "candidate:1 1 UDP 2130706431 127.0.0.1 51820 typ host"
            ));
            // Chrome's default host-candidate obfuscation.
            assert!(!candidate_is_usable(
                "candidate:1 1 udp 2130706431 8f0a2c1d-4e5a.local 51820 typ host"
            ));
            // The answerer's socket is UDP; a TCP pair can never form.
            assert!(!candidate_is_usable(
                "candidate:1 1 tcp 2130706431 192.168.1.20 9 typ host tcptype active"
            ));
            assert!(!candidate_is_usable("candidate:1 1 udp"));
        }

        #[test]
        fn candidates_land_in_the_single_media_section() {
            let offer =
                "v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=setup:actpass\r\n";
            let spliced = offer_with_candidates(
                offer,
                &[
                    "candidate:1 1 udp 1 127.0.0.1 5000 typ host".to_owned(),
                    "candidate:2 1 udp 1 192.168.1.20 5000 typ host".to_owned(),
                ],
            );
            assert!(spliced.starts_with(offer));
            assert!(spliced.ends_with("a=candidate:2 1 udp 1 192.168.1.20 5000 typ host\r\n"));
            assert_eq!(spliced.matches("a=candidate:").count(), 2);
            // Every candidate must follow the one m= line, or it is a
            // session-level attribute the answerer will ignore.
            let m_line = spliced.find("m=application").expect("one media section");
            assert!(spliced.find("a=candidate:").expect("candidates") > m_line);
        }

        #[test]
        fn an_offer_without_a_trailing_newline_still_yields_valid_lines() {
            let spliced = offer_with_candidates(
                "m=application 9 UDP/DTLS/SCTP webrtc-datachannel",
                &["candidate:1 1 udp 1 127.0.0.1 5000 typ host".to_owned()],
            );
            assert_eq!(
                spliced,
                "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
                 a=candidate:1 1 udp 1 127.0.0.1 5000 typ host\r\n"
            );
        }
    }
}

/// The revisioned ping protocol: a resume-by-diff-or-snapshot stand-in that
/// rides *above* the carrier as application framing.
///
/// The C1 protocol was stateless echo. C3's plan calls for *"resume by diff
/// when the retained revision is valid, otherwise by snapshot"* — the branch
/// a reconnect has to pick between once there is session state worth not
/// re-sending. This module is that branch and nothing else: the "state" is a
/// running byte count and a monotonic revision, not Graphshell's real
/// document (which is C4). It exists to exercise the choice, and to pin it
/// with tests that run on an ordinary `cargo test`, before a browser or a
/// TURN server is ever in the picture.
///
/// It touches no carrier frame type. Every message here is one carrier frame
/// *payload*: a one-byte tag and a few big-endian `u64`s. The carrier still
/// frames and delivers the bytes; this is what the two ends agree the bytes
/// *mean*.
///
/// ```text
/// browser -> host   HELLO    [0x01][u64 resume_from]
/// browser -> host   PING     [0x02][u64 client_seq][body…]
/// host -> browser   SNAPSHOT [0x81][u64 revision][u64 total_bytes]
/// host -> browser   DIFF     [0x82][u64 from][u64 to][u64 bytes]   (from>to ⇒ empty/up-to-date)
/// host -> browser   PONG     [0x83][u64 revision][u64 client_seq]
/// ```
///
/// The host answers a HELLO by consulting its [`RevisionLog`]: a DIFF when the
/// client's last revision is still inside the retained window, a SNAPSHOT when
/// it has fallen out (or the client claims a revision the host never issued).
/// A PING advances the log by its body length and comes back as a PONG
/// stamped with the new revision.
pub mod proto {
    use std::collections::VecDeque;

    /// `HELLO` — browser announces the last revision it acknowledged.
    pub const TAG_HELLO: u8 = 0x01;
    /// `PING` — browser sends one bounded frame to be echoed.
    pub const TAG_PING: u8 = 0x02;
    /// `SNAPSHOT` — host replies with the full current state.
    pub const TAG_SNAPSHOT: u8 = 0x81;
    /// `DIFF` — host replies with the revision range the client is missing.
    pub const TAG_DIFF: u8 = 0x82;
    /// `PONG` — host's stamped echo of one `PING`.
    pub const TAG_PONG: u8 = 0x83;

    /// Wire length of a `HELLO`.
    pub const HELLO_LEN: usize = 1 + 8;
    /// Fixed header length of a `PING`, before its body.
    pub const PING_HEADER_LEN: usize = 1 + 8;
    /// Wire length of a `SNAPSHOT`.
    pub const SNAPSHOT_LEN: usize = 1 + 8 + 8;
    /// Wire length of a `DIFF`.
    pub const DIFF_LEN: usize = 1 + 8 + 8 + 8;
    /// Wire length of a `PONG`.
    pub const PONG_LEN: usize = 1 + 8 + 8;

    /// How many recent revisions the host keeps diff-able. Older than this and
    /// a resuming client is answered with a snapshot instead.
    pub const DEFAULT_RETAIN: usize = 64;

    fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
        bytes.get(at..at + 8)?.try_into().ok().map(u64::from_be_bytes)
    }

    /// Encodes a `HELLO` carrying the last revision the browser acknowledged.
    pub fn encode_hello(resume_from: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(HELLO_LEN);
        out.push(TAG_HELLO);
        out.extend_from_slice(&resume_from.to_be_bytes());
        out
    }

    /// Encodes a `PING` of `total_len` carrier-payload bytes: the tag, an
    /// 8-byte client sequence number, and deterministic filler out to
    /// `total_len` (never shorter than [`PING_HEADER_LEN`]).
    pub fn encode_ping(client_seq: u64, total_len: usize) -> Vec<u8> {
        let total = total_len.max(PING_HEADER_LEN);
        let mut out = vec![0u8; total];
        out[0] = TAG_PING;
        out[1..9].copy_from_slice(&client_seq.to_be_bytes());
        for (offset, byte) in out.iter_mut().enumerate().skip(PING_HEADER_LEN) {
            *byte = (offset % 251) as u8;
        }
        out
    }

    /// Encodes a `SNAPSHOT` of the host's whole current state.
    pub fn encode_snapshot(revision: u64, total_bytes: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(SNAPSHOT_LEN);
        out.push(TAG_SNAPSHOT);
        out.extend_from_slice(&revision.to_be_bytes());
        out.extend_from_slice(&total_bytes.to_be_bytes());
        out
    }

    /// Encodes a `DIFF` for revisions `from..=to` totalling `bytes`. A `from`
    /// greater than `to` is the empty diff: the client is already current.
    pub fn encode_diff(from: u64, to: u64, bytes: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(DIFF_LEN);
        out.push(TAG_DIFF);
        out.extend_from_slice(&from.to_be_bytes());
        out.extend_from_slice(&to.to_be_bytes());
        out.extend_from_slice(&bytes.to_be_bytes());
        out
    }

    /// Encodes a `PONG`: the new revision a `PING` produced, and the client
    /// sequence number it answers.
    pub fn encode_pong(revision: u64, client_seq: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(PONG_LEN);
        out.push(TAG_PONG);
        out.extend_from_slice(&revision.to_be_bytes());
        out.extend_from_slice(&client_seq.to_be_bytes());
        out
    }

    /// A message from the browser to the host.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ClientMsg {
        /// Resume request carrying the browser's last acknowledged revision.
        Hello {
            /// The last revision the browser acknowledged (0 if none).
            resume_from: u64,
        },
        /// One bounded ping frame to be echoed.
        Ping {
            /// The browser's own per-frame sequence number.
            client_seq: u64,
            /// How many payload bytes the ping carried in total.
            body_len: usize,
        },
    }

    /// Decodes one browser→host message, or `None` if the tag or length is
    /// unrecognised (the host echoes such frames raw rather than trusting a
    /// misparse).
    pub fn decode_client(bytes: &[u8]) -> Option<ClientMsg> {
        match *bytes.first()? {
            TAG_HELLO if bytes.len() >= HELLO_LEN => Some(ClientMsg::Hello {
                resume_from: read_u64(bytes, 1)?,
            }),
            TAG_PING if bytes.len() >= PING_HEADER_LEN => Some(ClientMsg::Ping {
                client_seq: read_u64(bytes, 1)?,
                body_len: bytes.len(),
            }),
            _ => None,
        }
    }

    /// A message from the host to the browser.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ServerMsg {
        /// The host's full current state.
        Snapshot {
            /// Current revision.
            revision: u64,
            /// Total state bytes accumulated so far.
            total_bytes: u64,
        },
        /// The revision range the client was missing. `from > to` is the empty
        /// diff — the client is already current.
        Diff {
            /// First revision in the diff.
            from: u64,
            /// Last revision in the diff (the host's current revision).
            to: u64,
            /// Total bytes across the diffed revisions.
            bytes: u64,
        },
        /// A stamped echo of one ping.
        Pong {
            /// The revision the ping produced.
            revision: u64,
            /// The client sequence number this answers.
            client_seq: u64,
        },
    }

    /// Decodes one host→browser message, or `None` if unrecognised.
    pub fn decode_server(bytes: &[u8]) -> Option<ServerMsg> {
        match *bytes.first()? {
            TAG_SNAPSHOT if bytes.len() >= SNAPSHOT_LEN => Some(ServerMsg::Snapshot {
                revision: read_u64(bytes, 1)?,
                total_bytes: read_u64(bytes, 9)?,
            }),
            TAG_DIFF if bytes.len() >= DIFF_LEN => Some(ServerMsg::Diff {
                from: read_u64(bytes, 1)?,
                to: read_u64(bytes, 9)?,
                bytes: read_u64(bytes, 17)?,
            }),
            TAG_PONG if bytes.len() >= PONG_LEN => Some(ServerMsg::Pong {
                revision: read_u64(bytes, 1)?,
                client_seq: read_u64(bytes, 9)?,
            }),
            _ => None,
        }
    }

    /// What the host decided a resuming client needs: the diff it can replay,
    /// or the snapshot it must fall back to.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ResumeReply {
        /// Replay revisions `from..=to` (empty when `from > to`).
        Diff {
            /// First revision to replay.
            from: u64,
            /// Last revision to replay.
            to: u64,
            /// Total bytes across those revisions.
            bytes: u64,
        },
        /// Re-send the whole state; the client's revision is too old or unknown.
        Snapshot {
            /// Current revision.
            revision: u64,
            /// Total state bytes.
            total_bytes: u64,
        },
    }

    /// The host's per-logical-session state: a monotonic revision, the running
    /// state size, and a bounded window of recent revisions it can still diff.
    ///
    /// It is the whole of the "retained window" idea — nothing is persisted, no
    /// real document is stored, and the window is a plain ring of
    /// `(revision, bytes_added)` pairs. [`resume`](Self::resume) is the pure
    /// decision the tests pin.
    #[derive(Debug, Clone)]
    pub struct RevisionLog {
        current: u64,
        total_bytes: u64,
        window: VecDeque<(u64, u64)>,
        retain: usize,
    }

    impl RevisionLog {
        /// A fresh log retaining the last `retain` revisions for diffing.
        pub fn new(retain: usize) -> Self {
            Self {
                current: 0,
                total_bytes: 0,
                window: VecDeque::new(),
                retain: retain.max(1),
            }
        }

        /// The current (latest) revision number.
        pub fn current(&self) -> u64 {
            self.current
        }

        /// Total state bytes accumulated across every revision.
        pub fn total_bytes(&self) -> u64 {
            self.total_bytes
        }

        /// Applies one change of `bytes`, bumping the revision, and returns the
        /// new current revision. Evicts the oldest retained revision once the
        /// window is full.
        pub fn apply(&mut self, bytes: u64) -> u64 {
            self.current += 1;
            self.total_bytes += bytes;
            self.window.push_back((self.current, bytes));
            while self.window.len() > self.retain {
                self.window.pop_front();
            }
            self.current
        }

        /// The oldest revision still in the retained window, or `current + 1`
        /// when nothing is retained (so no revision counts as resumable).
        fn oldest_retained(&self) -> u64 {
            self.window
                .front()
                .map(|(rev, _)| *rev)
                .unwrap_or(self.current + 1)
        }

        /// Decides how to resume a client that last acknowledged revision `r`.
        ///
        /// - `r > current` — the client claims a revision this host never
        ///   issued: reset it with a [`Snapshot`](ResumeReply::Snapshot).
        /// - `r == current` — already current: an empty
        ///   [`Diff`](ResumeReply::Diff) (`from == current + 1 > to`).
        /// - `r + 1` still retained — replay it as a
        ///   [`Diff`](ResumeReply::Diff) of `r+1..=current`.
        /// - otherwise `r` has aged out of the window: a
        ///   [`Snapshot`](ResumeReply::Snapshot).
        pub fn resume(&self, r: u64) -> ResumeReply {
            if r > self.current {
                return ResumeReply::Snapshot {
                    revision: self.current,
                    total_bytes: self.total_bytes,
                };
            }
            if r == self.current {
                return ResumeReply::Diff {
                    from: self.current + 1,
                    to: self.current,
                    bytes: 0,
                };
            }
            if r + 1 >= self.oldest_retained() {
                let bytes = self
                    .window
                    .iter()
                    .filter(|(rev, _)| *rev > r)
                    .map(|(_, added)| *added)
                    .sum();
                ResumeReply::Diff {
                    from: r + 1,
                    to: self.current,
                    bytes,
                }
            } else {
                ResumeReply::Snapshot {
                    revision: self.current,
                    total_bytes: self.total_bytes,
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn client_messages_round_trip() {
            assert_eq!(
                decode_client(&encode_hello(42)),
                Some(ClientMsg::Hello { resume_from: 42 })
            );
            let ping = encode_ping(7, 100);
            assert_eq!(ping.len(), 100);
            assert_eq!(
                decode_client(&ping),
                Some(ClientMsg::Ping {
                    client_seq: 7,
                    body_len: 100
                })
            );
            // A ping is never shorter than its header, even when asked for less.
            assert_eq!(encode_ping(1, 0).len(), PING_HEADER_LEN);
            // An unknown tag decodes to None so the host can fall back to echo.
            assert_eq!(decode_client(&[0xEE, 0, 0]), None);
            assert_eq!(decode_client(&[]), None);
        }

        #[test]
        fn server_messages_round_trip() {
            assert_eq!(
                decode_server(&encode_snapshot(9, 900)),
                Some(ServerMsg::Snapshot {
                    revision: 9,
                    total_bytes: 900
                })
            );
            assert_eq!(
                decode_server(&encode_diff(8, 10, 300)),
                Some(ServerMsg::Diff {
                    from: 8,
                    to: 10,
                    bytes: 300
                })
            );
            assert_eq!(
                decode_server(&encode_pong(11, 7)),
                Some(ServerMsg::Pong {
                    revision: 11,
                    client_seq: 7
                })
            );
        }

        /// Revision within the retained window resumes by DIFF of `R+1..=cur`.
        #[test]
        fn resume_within_window_is_a_diff() {
            let mut log = RevisionLog::new(4);
            for _ in 0..10 {
                log.apply(100); // revisions 1..=10; window retains 7,8,9,10
            }
            assert_eq!(log.current(), 10);
            match log.resume(7) {
                ResumeReply::Diff { from, to, bytes } => {
                    assert_eq!((from, to), (8, 10));
                    assert_eq!(bytes, 300); // revisions 8, 9, 10
                }
                other => panic!("expected a diff, got {other:?}"),
            }
            // The window's oldest edge (revision 6 -> 6+1=7 retained) still diffs.
            match log.resume(6) {
                ResumeReply::Diff { from, to, bytes } => {
                    assert_eq!((from, to), (7, 10));
                    assert_eq!(bytes, 400);
                }
                other => panic!("expected a diff at the window edge, got {other:?}"),
            }
        }

        /// Revision aged out of the window resumes by SNAPSHOT.
        #[test]
        fn resume_too_old_is_a_snapshot() {
            let mut log = RevisionLog::new(4);
            for _ in 0..10 {
                log.apply(100);
            }
            match log.resume(3) {
                ResumeReply::Snapshot {
                    revision,
                    total_bytes,
                } => {
                    assert_eq!(revision, 10);
                    assert_eq!(total_bytes, 1000);
                }
                other => panic!("expected a snapshot, got {other:?}"),
            }
        }

        /// `R == current` resumes by an empty DIFF (`from > to`), not a snapshot.
        #[test]
        fn resume_at_current_is_an_empty_diff() {
            let mut log = RevisionLog::new(4);
            for _ in 0..10 {
                log.apply(100);
            }
            match log.resume(10) {
                ResumeReply::Diff { from, to, bytes } => {
                    assert_eq!(from, 11);
                    assert_eq!(to, 10);
                    assert!(from > to, "an up-to-date client gets an empty diff");
                    assert_eq!(bytes, 0);
                }
                other => panic!("expected an empty diff, got {other:?}"),
            }
        }

        /// A client claiming a revision the host never issued is reset by SNAPSHOT.
        #[test]
        fn resume_ahead_of_host_is_a_snapshot() {
            let mut log = RevisionLog::new(4);
            for _ in 0..10 {
                log.apply(100);
            }
            match log.resume(99) {
                ResumeReply::Snapshot { revision, .. } => assert_eq!(revision, 10),
                other => panic!("expected a snapshot, got {other:?}"),
            }
        }

        /// A brand-new session (nothing applied) answers an empty diff.
        #[test]
        fn resume_on_a_fresh_log_is_an_empty_diff() {
            let log = RevisionLog::new(4);
            assert_eq!(
                log.resume(0),
                ResumeReply::Diff {
                    from: 1,
                    to: 0,
                    bytes: 0
                }
            );
        }
    }
}

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
mod probe {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use js_sys::{Array, Function, Promise};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    use crate::proto::{self, ServerMsg};
    use crate::sdp::{candidate_is_usable, offer_with_candidates};
    use webrtc_carrier::{
        BrowserInitiator, BrowserInitiatorConfig, IceServer, IceTransportPolicy,
        MAX_FRAME_PAYLOAD_BYTES, VERSION,
    };

    /// How long to wait for gathering to finish before offering whatever has
    /// been gathered so far.
    const GATHER_TIMEOUT_MS: i32 = 5_000;

    // ── DOM element ids, shared with index.html ─────────────────────────────
    const ID_STATE: &str = "state";
    const ID_ERROR: &str = "error";
    const ID_SENT: &str = "frames-sent";
    const ID_RECEIVED: &str = "frames-received";
    const ID_BYTES_SENT: &str = "bytes-sent";
    const ID_BYTES_RECEIVED: &str = "bytes-received";
    const ID_BUFFERED: &str = "buffered";
    const ID_CANDIDATES: &str = "candidates";
    const ID_ELAPSED: &str = "elapsed";
    const ID_RESUME: &str = "resume-mode";
    const ID_REVISION: &str = "revision";
    const ID_MARKS: &str = "marks";

    /// Everything the page displays, as plain numbers.
    ///
    /// Kept deliberately separate from the initiator: see the crate doc's
    /// first callback rule.
    #[derive(Default)]
    struct Stats {
        state: String,
        error: String,
        frames_sent: u64,
        frames_received: u64,
        bytes_sent: u64,
        bytes_received: u64,
        buffered: u32,
        candidates_kept: u32,
        candidates_dropped: u32,
        first_send_ms: Option<f64>,
        last_receive_ms: Option<f64>,
        terminal: bool,
        /// How the last resume handshake was answered — `"snapshot …"`,
        /// `"diff …"`, or empty before the first HELLO reply. Surfaced so a
        /// screenshot records which branch a reconnect took.
        resume_mode: String,
        /// The host's revision this end has last seen (from a PONG, SNAPSHOT,
        /// or DIFF). Also what the next HELLO resumes from.
        current_revision: u64,
        /// Whether this session forced relay-only ICE. Remembered so a
        /// reconnect can phrase a zero-candidate failure the same way.
        relay_only: bool,
    }

    /// One session: the initiator, plus the flag its callbacks consult before
    /// writing anything into [`Stats`].
    ///
    /// See the crate doc's third callback rule. Cloning shares both, so a
    /// clone handed to a closure silences with the original.
    #[derive(Clone)]
    struct SessionHandle {
        initiator: Rc<BrowserInitiator>,
        active: Rc<Cell<bool>>,
    }

    impl SessionHandle {
        /// Stops this session's callbacks from touching the page's stats.
        fn deactivate(&self) {
            self.active.set(false);
        }

        /// Lets them through again — for the one path that silences a session
        /// speculatively and then finds it is still the live one.
        fn reactivate(&self) {
            self.active.set(true);
        }
    }

    /// The connection knobs [`connect`] was given, kept so [`reconnect`] can
    /// stand a new session up on the same TURN configuration without asking
    /// the page for them a second time.
    #[derive(Clone, Default)]
    struct Knobs {
        turn_url: String,
        turn_username: String,
        turn_credential: String,
        relay_only: bool,
    }

    thread_local! {
        static STATS: RefCell<Stats> = RefCell::new(Stats::default());
        static SESSION: RefCell<Option<SessionHandle>> = const { RefCell::new(None) };
        static KNOBS: RefCell<Knobs> = RefCell::new(Knobs::default());
        /// Closed sessions, kept alive so their event closures outlive every
        /// event the browser might still deliver to them.
        static RETIRED: RefCell<Vec<SessionHandle>> = const { RefCell::new(Vec::new()) };
    }

    /// Silences, closes, and parks whatever session is live.
    ///
    /// Park, never drop: see the crate doc's second callback rule.
    fn retire_current() {
        if let Some(handle) = SESSION.with(|session| session.borrow_mut().take()) {
            handle.deactivate();
            handle.initiator.close();
            RETIRED.with(|retired| retired.borrow_mut().push(handle));
        }
    }

    // ── DOM ─────────────────────────────────────────────────────────────────

    fn set_text(id: &str, text: &str) {
        if let Some(document) = web_sys::window().and_then(|window| window.document())
            && let Some(element) = document.get_element_by_id(id)
        {
            element.set_text_content(Some(text));
        }
    }

    fn now_ms() -> f64 {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .unwrap_or(0.0)
    }

    fn with_stats<R>(f: impl FnOnce(&mut Stats) -> R) -> R {
        STATS.with(|stats| f(&mut stats.borrow_mut()))
    }

    fn set_state(state: &str) {
        with_stats(|stats| stats.state = state.to_owned());
        render();
    }

    fn set_error(error: &str) {
        with_stats(|stats| stats.error = error.to_owned());
        web_sys::console::error_1(&JsValue::from_str(error));
        render();
    }

    fn render() {
        STATS.with(|stats| {
            let stats = stats.borrow();
            set_text(ID_STATE, &stats.state);
            set_text(ID_ERROR, &stats.error);
            set_text(ID_SENT, &stats.frames_sent.to_string());
            set_text(ID_RECEIVED, &stats.frames_received.to_string());
            set_text(ID_BYTES_SENT, &stats.bytes_sent.to_string());
            set_text(ID_BYTES_RECEIVED, &stats.bytes_received.to_string());
            set_text(ID_BUFFERED, &stats.buffered.to_string());
            set_text(
                ID_CANDIDATES,
                &format!(
                    "{} kept / {} dropped",
                    stats.candidates_kept, stats.candidates_dropped
                ),
            );
            let elapsed = match (stats.first_send_ms, stats.last_receive_ms) {
                (Some(start), Some(end)) if end >= start => format!("{:.1} ms", end - start),
                _ => "—".to_owned(),
            };
            set_text(ID_ELAPSED, &elapsed);
            set_text(
                ID_RESUME,
                if stats.resume_mode.is_empty() {
                    "—"
                } else {
                    &stats.resume_mode
                },
            );
            set_text(ID_REVISION, &stats.current_revision.to_string());
        });
    }

    // ── promises ────────────────────────────────────────────────────────────

    /// A promise plus the function that resolves it.
    ///
    /// A `Promise` executor runs synchronously, which is what makes taking the
    /// resolver back out of the closure sound.
    fn resolver() -> (Promise, Function) {
        let mut slot: Option<Function> = None;
        let promise = Promise::new(&mut |resolve, _reject| slot = Some(resolve));
        (promise, slot.expect("a Promise executor runs synchronously"))
    }

    fn timeout(ms: i32) -> Result<Promise, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        Ok(Promise::new(&mut |resolve, _reject| {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }))
    }

    // ── signalling ──────────────────────────────────────────────────────────

    /// One POST of `text/plain`, one response body. That is all of C1's
    /// signalling.
    async fn post_text(url: &str, body: &str) -> Result<String, JsValue> {
        let init = RequestInit::new();
        init.set_method("POST");
        init.set_mode(RequestMode::Cors);
        init.set_body(&JsValue::from_str(body));
        let request = Request::new_with_str_and_init(url, &init)?;
        // Keeps it a CORS *simple* request: no preflight for the fixture to
        // answer.
        request.headers().set("content-type", "text/plain")?;

        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let response: Response = JsFuture::from(window.fetch_with_request(&request))
            .await?
            .dyn_into()?;
        let text = JsFuture::from(response.text()?)
            .await?
            .as_string()
            .unwrap_or_default();
        if !response.ok() {
            return Err(JsValue::from_str(&format!(
                "signalling fixture returned {} {}: {text}",
                response.status(),
                response.status_text()
            )));
        }
        Ok(text)
    }

    // ── exports ─────────────────────────────────────────────────────────────

    /// Puts the carrier's version and its bounds on the page.
    #[wasm_bindgen]
    pub fn boot() {
        let config = BrowserInitiatorConfig::default();
        set_text(
            ID_MARKS,
            &format!(
                "webrtc-carrier {VERSION} · label {} · protocol {} · high water {} B · low water {} B · max frame payload {} B",
                config.channel_label,
                config.protocol,
                config.backpressure.high_water_bytes(),
                config.backpressure.low_water_bytes(),
                MAX_FRAME_PAYLOAD_BYTES,
            ),
        );
        set_state("idle");
    }

    /// Builds the initiator config from the page's TURN and relay controls.
    ///
    /// An empty `turn_url` leaves `ice_servers` empty — the C1 default, still
    /// guarded against gathering nothing usable. A non-empty one becomes a
    /// single [`IceServer`]; a blank username or credential is `None` (a bare
    /// STUN/unauthenticated server) rather than an empty string. `relay_only`
    /// selects [`IceTransportPolicy::Relay`] — C3's forced-relay knob — over
    /// the default `All`.
    fn build_config(
        turn_url: &str,
        turn_username: &str,
        turn_credential: &str,
        relay_only: bool,
    ) -> BrowserInitiatorConfig {
        let mut config = BrowserInitiatorConfig::default();
        let turn_url = turn_url.trim();
        if !turn_url.is_empty() {
            let username = {
                let u = turn_username.trim();
                (!u.is_empty()).then(|| u.to_owned())
            };
            let credential = {
                let c = turn_credential.trim();
                (!c.is_empty()).then(|| c.to_owned())
            };
            config.ice_servers = vec![IceServer {
                urls: vec![turn_url.to_owned()],
                username,
                credential,
            }];
        }
        config.ice_transport_policy = if relay_only {
            IceTransportPolicy::Relay
        } else {
            IceTransportPolicy::All
        };
        config
    }

    /// Registers a fresh ICE-candidate collector and returns the bucket usable
    /// candidates land in plus the promise that resolves at end-of-gathering.
    ///
    /// Must be called *before* the offer that starts gathering (an initial
    /// `create_offer` or a `create_restart_offer`). Resets the kept/dropped
    /// counters, since each offer gathers anew.
    fn install_candidate_collector(
        session: &SessionHandle,
    ) -> (Rc<RefCell<Vec<String>>>, Promise) {
        with_stats(|stats| {
            stats.candidates_kept = 0;
            stats.candidates_dropped = 0;
        });
        let candidates: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let (gathered, finish_gathering) = resolver();
        {
            let candidates = candidates.clone();
            let active = session.active.clone();
            session
                .initiator
                .on_local_ice_candidate(move |candidate| match candidate {
                    Some(candidate) => {
                        // A silenced session still collects into its own
                        // bucket — the gathering it is in the middle of stays
                        // coherent — but stops writing the page's counters.
                        if candidate_is_usable(&candidate.candidate) {
                            candidates.borrow_mut().push(candidate.candidate);
                            if active.get() {
                                with_stats(|stats| stats.candidates_kept += 1);
                            }
                        } else if active.get() {
                            with_stats(|stats| stats.candidates_dropped += 1);
                        }
                        if active.get() {
                            render();
                        }
                    }
                    None => {
                        let _ = finish_gathering.call0(&JsValue::NULL);
                    }
                });
        }
        (candidates, gathered)
    }

    /// Decodes every incoming frame as a [`proto`] message and folds it into
    /// the page's stats: a SNAPSHOT/DIFF sets the resume-mode readout, a PONG
    /// advances the revision and echo counters, and anything unrecognised is
    /// still counted as a received frame.
    fn install_frame_handler(session: &SessionHandle) {
        let active = session.active.clone();
        session.initiator.on_frame(move |payload| {
            if !active.get() {
                return;
            }
            match proto::decode_server(&payload) {
                Some(ServerMsg::Snapshot {
                    revision,
                    total_bytes,
                }) => {
                    with_stats(|stats| {
                        stats.resume_mode = format!("snapshot · rev {revision} · {total_bytes} B");
                        stats.current_revision = revision;
                    });
                }
                Some(ServerMsg::Diff { from, to, bytes }) => {
                    with_stats(|stats| {
                        stats.resume_mode = if from > to {
                            format!("diff · already current at rev {to}")
                        } else {
                            format!("diff · rev {from}..={to} · {bytes} B")
                        };
                        stats.current_revision = to;
                    });
                }
                Some(ServerMsg::Pong { revision, .. }) => {
                    with_stats(|stats| {
                        stats.frames_received += 1;
                        stats.bytes_received += payload.len() as u64;
                        stats.last_receive_ms = Some(now_ms());
                        stats.current_revision = revision;
                    });
                }
                None => {
                    with_stats(|stats| {
                        stats.frames_received += 1;
                        stats.bytes_received += payload.len() as u64;
                        stats.last_receive_ms = Some(now_ms());
                    });
                }
            }
            render();
        });
    }

    /// Races gathering to a deadline and enforces the zero-candidate guard.
    ///
    /// Split from [`signal_offer`] so a caller can act in the gap between
    /// them: gathering is still reversible — nothing has left this machine —
    /// while posting the offer is not, because the fixture preempts its
    /// current session the instant one arrives.
    async fn gather(gathered: Promise, relay_only: bool) -> Result<(), JsValue> {
        set_state("gathering ICE candidates");
        let deadline = timeout(GATHER_TIMEOUT_MS)?;
        let race = Array::new();
        race.push(&gathered);
        race.push(&deadline);
        JsFuture::from(Promise::race(&JsValue::from(race))).await?;

        let candidates_kept = with_stats(|stats| stats.candidates_kept);
        let candidates_dropped = with_stats(|stats| stats.candidates_dropped);
        if candidates_kept == 0 {
            // Still guarded even with a bare STUN-less `All` config: the C1
            // failure (all candidates were mDNS `.local`) and the C3 failure
            // (forced-relay gathered nothing because no TURN server answered)
            // read differently, so the hint does too.
            let message = if relay_only {
                format!(
                    "offer has no usable ICE candidates ({candidates_dropped} dropped) — forced relay-only gathered no relay candidate: the TURN server is unreachable, its credentials are stale, or none was configured"
                )
            } else {
                format!(
                    "offer has no usable ICE candidates (all {candidates_dropped} were mDNS .local names str0m cannot resolve) — disable chrome://flags/#enable-webrtc-hide-local-ips-with-mdns or supply an ICE server"
                )
            };
            set_error(&message);
            set_state("gathering failed");
            return Err(JsValue::from_str(&message));
        }
        Ok(())
    }

    /// Splices the gathered candidates into `offer`, posts it, and applies the
    /// answer. Shared by [`connect`] and [`reconnect`].
    async fn signal_offer(
        initiator: &Rc<BrowserInitiator>,
        signal_url: &str,
        offer: String,
        candidates: &Rc<RefCell<Vec<String>>>,
    ) -> Result<(), JsValue> {
        let full_offer = offer_with_candidates(&offer, &candidates.borrow());
        set_state("posting the offer to the signalling fixture");
        let answer = match post_text(signal_url, &full_offer).await {
            Ok(answer) => answer,
            Err(err) => {
                let message = format!(
                    "POST {signal_url} failed: {} — is the answerer running?",
                    describe(&err)
                );
                set_error(&message);
                set_state("signalling failed");
                return Err(JsValue::from_str(&message));
            }
        };

        set_state("applying the answer");
        initiator
            .set_remote_answer(&answer)
            .await
            .map_err(|err| JsValue::from_str(&format!("set_remote_answer: {err}")))?;
        Ok(())
    }

    /// Sends a resume HELLO carrying the last revision this end has seen (0 on
    /// a fresh session). The host answers diff or snapshot through `on_frame`.
    async fn send_hello(initiator: &Rc<BrowserInitiator>) -> Result<(), JsValue> {
        let resume_from = with_stats(|stats| stats.current_revision);
        initiator
            .send_frame(&proto::encode_hello(resume_from))
            .await
            .map_err(|err| JsValue::from_str(&format!("resume hello: {err}")))
    }

    /// Clears the counters a new link starts over, keeping what carries across
    /// one.
    ///
    /// Deliberately not `Stats::default()`. Three fields have to survive: the
    /// revision the next HELLO resumes from — the whole point of a reconnect —
    /// how this page is connected, and the candidate accounting for the offer
    /// that is already gathered and about to be posted.
    fn reset_for_new_link() {
        with_stats(|stats| {
            stats.error.clear();
            stats.frames_sent = 0;
            stats.frames_received = 0;
            stats.bytes_sent = 0;
            stats.bytes_received = 0;
            stats.buffered = 0;
            stats.first_send_ms = None;
            stats.last_receive_ms = None;
            stats.terminal = false;
            stats.resume_mode.clear();
        });
    }

    /// Builds an initiator, offers, signals, waits for the data channel, and
    /// runs the resume handshake — everything [`connect`] and [`reconnect`]
    /// share.
    ///
    /// `commit` runs exactly once, after gathering has produced at least one
    /// usable candidate and immediately *before* the offer is posted. That
    /// instant is the point of no return: the fixture preempts whatever session
    /// it is running the moment an offer lands, so a caller replacing a live
    /// session does it there and not a step earlier.
    ///
    /// The returned handle is not yet the live session. Installing it is the
    /// caller's, because what to retire and when differs between opening the
    /// first session and replacing one.
    async fn open_session(
        signal_url: &str,
        knobs: &Knobs,
        commit: impl FnOnce(),
    ) -> Result<SessionHandle, JsValue> {
        set_state(&format!(
            "building the peer connection ({})",
            if knobs.relay_only { "relay-only" } else { "all" }
        ));

        let handle = SessionHandle {
            initiator: Rc::new(
                BrowserInitiator::new(build_config(
                    &knobs.turn_url,
                    &knobs.turn_username,
                    &knobs.turn_credential,
                    knobs.relay_only,
                ))
                .map_err(|err| JsValue::from_str(&format!("initiator: {err}")))?,
            ),
            active: Rc::new(Cell::new(true)),
        };

        match offer_and_open(&handle, signal_url, knobs.relay_only, commit).await {
            Ok(()) => Ok(handle),
            Err(err) => {
                // Park, never drop, even half-built: the handlers registered
                // below are live from the moment they are registered.
                handle.deactivate();
                handle.initiator.close();
                RETIRED.with(|retired| retired.borrow_mut().push(handle));
                Err(err)
            }
        }
    }

    /// The fallible tail of [`open_session`], split out so every exit from it
    /// runs that function's park-never-drop cleanup.
    async fn offer_and_open(
        handle: &SessionHandle,
        signal_url: &str,
        relay_only: bool,
        commit: impl FnOnce(),
    ) -> Result<(), JsValue> {
        // Registered before `create_offer`, which is what starts gathering.
        let (candidates, gathered) = install_candidate_collector(handle);
        install_frame_handler(handle);
        {
            let active = handle.active.clone();
            handle.initiator.on_closed(move |err| {
                if !active.get() {
                    return;
                }
                with_stats(|stats| stats.terminal = true);
                set_state(&format!("closed — {err}"));
            });
        }

        set_state("creating the offer");
        let offer = handle
            .initiator
            .create_offer()
            .await
            .map_err(|err| JsValue::from_str(&format!("create_offer: {err}")))?;

        gather(gathered, relay_only).await?;
        commit();
        signal_offer(&handle.initiator, signal_url, offer, &candidates).await?;

        set_state("waiting for the data channel to open");
        handle
            .initiator
            .wait_until_open()
            .await
            .map_err(|err| JsValue::from_str(&format!("wait_until_open: {err}")))?;

        set_state("resuming (sending HELLO)");
        send_hello(&handle.initiator).await
    }

    /// Offers, signals, opens the data channel, and runs the resume handshake.
    ///
    /// `signal_url` is the native fixture's `/offer` endpoint. A non-empty
    /// `turn_url` (plus optional `turn_username`/`turn_credential`) configures
    /// a TURN server; `relay_only` forces relay-only ICE (C3's forced-relay
    /// receipt). All four connection knobs come from the page, and are kept so
    /// [`reconnect`] can rebuild on the same ones.
    #[wasm_bindgen]
    pub async fn connect(
        signal_url: String,
        turn_url: String,
        turn_username: String,
        turn_credential: String,
        relay_only: bool,
    ) -> Result<(), JsValue> {
        let live = SESSION.with(|session| session.borrow().is_some())
            && !with_stats(|stats| stats.terminal);
        if live {
            return Err(JsValue::from_str(
                "already connected — close this session before opening another",
            ));
        }
        retire_current();

        let knobs = Knobs {
            turn_url,
            turn_username,
            turn_credential,
            relay_only,
        };
        KNOBS.with(|stored| *stored.borrow_mut() = knobs.clone());
        with_stats(|stats| {
            *stats = Stats::default();
            stats.relay_only = relay_only;
        });

        // CONNECT starts from nothing, so there is no live session to replace
        // and nothing to do at the point of no return.
        let handle = open_session(&signal_url, &knobs, || ()).await?;
        SESSION.with(|session| *session.borrow_mut() = Some(handle));
        set_state("open");
        refresh();
        Ok(())
    }

    /// Replaces the current session with a new one and resumes where it left
    /// off.
    ///
    /// Not an ICE restart, and the crate doc's *"Reconnect is a new link"*
    /// section is why: this fixture answers every offer from a freshly bound
    /// answerer with a new DTLS certificate, which closes the browser's data
    /// channel for good rather than renegotiating it. A whole new initiator is
    /// the honest response, and the C3 receipt's defect 3 is the observation
    /// that named it — the old code posted a new offer and then wrote its
    /// resume HELLO into the channel that offer had just condemned.
    ///
    /// Only the revision crosses: the new session's HELLO carries it, and the
    /// host answers diff or snapshot, which is what the page's resume-mode
    /// readout records. Everything else — DTLS, admission, the frame counters —
    /// starts over, which is plan §6's rule behaving as written rather than
    /// being worked around.
    ///
    /// Works on a session that has already closed. Recovering a dropped link
    /// while keeping your place is what this button is for, so a `terminal`
    /// session is a reason to press it rather than a reason to refuse.
    #[wasm_bindgen]
    pub async fn reconnect(signal_url: String) -> Result<(), JsValue> {
        let previous = SESSION
            .with(|session| session.borrow().clone())
            .ok_or_else(|| JsValue::from_str("not connected — CONNECT before RECONNECT"))?;
        let knobs = KNOBS.with(|knobs| knobs.borrow().clone());

        set_state("reconnecting (standing up a new link)");

        // Until the offer is posted the old session is still the page's
        // subject: a reconnect that fails while gathering has taken nothing
        // away, and the session it would have replaced is still there to
        // report on itself. Past that point the fixture has preempted it.
        let committed = Rc::new(Cell::new(false));
        let outcome = {
            let committed = committed.clone();
            let previous = previous.clone();
            open_session(&signal_url, &knobs, move || {
                committed.set(true);
                previous.deactivate();
                reset_for_new_link();
            })
            .await
        };

        match outcome {
            Ok(handle) => {
                retire_current();
                SESSION.with(|session| *session.borrow_mut() = Some(handle));
                set_state("open (reconnected)");
                refresh();
                Ok(())
            }
            Err(err) => {
                if committed.get() {
                    // The offer reached the fixture, so the old session was
                    // preempted there and is not coming back. Clear it out, or
                    // CONNECT would refuse as "already connected" on the
                    // strength of a session that no longer exists.
                    retire_current();
                    with_stats(|stats| stats.terminal = true);
                } else {
                    previous.reactivate();
                }
                Err(err)
            }
        }
    }

    /// Sends `count` revisioned ping frames of `payload_bytes` each.
    ///
    /// Each frame is a [`proto`] PING: a tag, an 8-byte sequence number, and
    /// filler out to the requested size. The host advances its revision per
    /// ping and echoes a PONG stamped with the new revision, which `on_frame`
    /// folds into the revision readout.
    #[wasm_bindgen]
    pub async fn send_frames(count: u32, payload_bytes: u32) -> Result<(), JsValue> {
        let initiator = SESSION
            .with(|session| session.borrow().as_ref().map(|s| s.initiator.clone()))
            .ok_or_else(|| JsValue::from_str("not connected"))?;

        let size = (payload_bytes as usize).clamp(proto::PING_HEADER_LEN, MAX_FRAME_PAYLOAD_BYTES);
        set_state(&format!("sending {count} ping frames of {size} B"));
        for index in 0..count {
            let payload = proto::encode_ping(u64::from(index), size);
            if let Err(err) = initiator.send_frame(&payload).await {
                let message = format!("send_frame({index}): {err}");
                set_error(&message);
                set_state("send failed");
                return Err(JsValue::from_str(&message));
            }
            let buffered = initiator.buffered_amount();
            with_stats(|stats| {
                if stats.first_send_ms.is_none() {
                    stats.first_send_ms = Some(now_ms());
                }
                stats.frames_sent += 1;
                stats.bytes_sent += payload.len() as u64;
                stats.buffered = buffered;
            });
            render();
        }
        set_state("open");
        Ok(())
    }

    /// Closes the data channel and the peer connection from the page side.
    ///
    /// The initiator itself stays alive; `on_closed` reports the terminal
    /// reason, and [`connect`] retires this one when a new session starts.
    #[wasm_bindgen]
    pub fn close_session() {
        match SESSION.with(|session| session.borrow().as_ref().map(|s| s.initiator.clone())) {
            Some(initiator) => {
                set_state("closing");
                // Left active on purpose: this close is the page's own, and
                // `on_closed` reporting it is the point.
                initiator.close();
            }
            None => set_state("nothing to close"),
        }
    }

    /// Re-reads `bufferedAmount` and repaints.
    ///
    /// Called from a page timer, never from inside a carrier callback.
    #[wasm_bindgen]
    pub fn refresh() {
        let buffered = SESSION.with(|session| {
            session
                .borrow()
                .as_ref()
                .map(|session| session.initiator.buffered_amount())
                .unwrap_or(0)
        });
        with_stats(|stats| stats.buffered = buffered);
        render();
    }

    fn describe(value: &JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                value
                    .dyn_ref::<js_sys::Error>()
                    .map(|error| String::from(error.message()))
            })
            .unwrap_or_else(|| format!("{value:?}"))
    }
}
