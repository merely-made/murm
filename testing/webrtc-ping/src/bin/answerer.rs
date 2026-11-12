//! The host half of the C1 headed receipt: a real native process for a real
//! browser to talk to.
//!
//! Earns plan §4's first done-condition together with `src/lib.rs`. This half
//! binds [`webrtc_carrier::native::Answerer`], answers exactly one offer per
//! session, echoes every frame the browser sends, and prints one
//! machine-readable line per session so the receipt can quote measured
//! numbers rather than describe them.
//!
//! ## The signalling fixture is nine handlers wide on purpose
//!
//! Plan §4: *"Use local copy/paste or a tiny loopback signaling fixture for
//! this phase. Public infrastructure is deliberately absent so transport
//! behaviour can be inspected without mixing it with service deployment."* So
//! there is no HTTP crate here — a `TcpListener`, a request line, a
//! `Content-Length`, and a response. Three endpoints:
//!
//! ```text
//! POST /offer             text/plain   browser offer SDP  ->  answer SDP
//! GET  /health            text/plain   ->  "ok"
//! GET  /turn-credentials  application/json  ->  {username, credential, ttl, urls, realm?}
//! ```
//!
//! `Access-Control-Allow-Origin: *`, because the page is served from a
//! different port than this fixture listens on. `OPTIONS` is answered too,
//! even though the page's `text/plain` POST is a CORS *simple* request that
//! never preflights — costing four lines beats a puzzling failure for whoever
//! changes the content type later.
//!
//! It binds loopback only. Nothing here is a signalling *service*; a Worker
//! relaying SDP is not a carrier fact, which is the boundary the plan states.
//!
//! ## Minting ephemeral TURN credentials — the server-side half of C3
//!
//! `/turn-credentials` implements the coturn REST convention (`use-auth-secret`):
//! the host holds a long-term shared secret and hands the browser a *short-lived*
//! `username:credential` pair, where
//!
//! ```text
//! username   = "<unix_expiry>:mere"
//! credential = base64( HMAC-SHA1( secret, username ) )
//! ```
//!
//! The long-term secret is read from this process's own config
//! (`--turn-secret`, or `WEBRTC_PING_TURN_SECRET`) and **never reaches the
//! browser** — that is the entire security point: the page fetches a
//! time-limited token it cannot forge more of. With a real TURN server URL
//! configured (`--turn-url`, Mark's live server), this endpoint *is* the whole
//! server-side minting story; the page fetches from it before connecting and
//! feeds the pair straight into the browser's `RTCIceServer`. Absent a secret,
//! the endpoint answers `503` so the fixture still runs for direct receipts.
//!
//! ## The revisioned ping protocol
//!
//! The frames are no longer a stateless echo. Each session shares one
//! process-wide [`RevisionLog`] (see `webrtc_ping::proto`): a browser HELLO
//! carrying its last acknowledged revision is answered by a DIFF when that
//! revision is still in the retained window or a SNAPSHOT when it has aged out,
//! and each PING advances the log and comes back as a stamped PONG. This is the
//! *"resume by diff when the retained revision is valid, otherwise by snapshot"*
//! branch, exercised as application framing above the carrier — the carrier's
//! own frame types are untouched.
//!
//! ## Sessions, refresh, and preemption
//!
//! Each offer gets a freshly bound `Answerer` — new socket, new certificate,
//! new fingerprint. The session then runs in its own task, so the main loop
//! stays on the offer channel: when a refreshed page posts a second offer, the
//! previous session is *preempted* (a `oneshot` it selects on, biased ahead of
//! the read) rather than left for the 30-second idle timeout to reap. That is
//! plan §4's *"browser refresh and native cancellation terminate both tasks
//! without a hot loop"* exercised from the browser's side; `iterations` in the
//! receipt line is the instrument for the "without a hot loop" half.
//!
//! ## Interface discovery
//!
//! `webrtc-carrier` does none, deliberately — it is the crate that has to
//! build for the browser. So this probe does it, in the one dependency-free
//! way there is: bind a scratch UDP socket, `connect` it (no packet is sent, a
//! route is looked up), and read back the local address. Loopback plus that
//! address are advertised, because a browser on this same machine reaches a
//! `127.0.0.1` candidate only from a loopback-bound local candidate, which
//! Chrome does not ordinarily gather.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc as blocking_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};

use webrtc_carrier::native::{
    Answerer, AnswererConfig, Carrier, CarrierConfig, CarrierStats, NativeError,
    SessionFingerprints,
};
use webrtc_ping::proto::{self, ClientMsg, ResumeReply, RevisionLog};

// A crate root's submodules live beside it, and `src/bin/*.rs` is where Cargo
// looks for *binaries*: a plain `src/bin/turn_client.rs` would be auto-
// discovered as a second (main-less) bin target. `#[path]` into a directory
// Cargo does not scan — it only treats `src/bin/<name>/main.rs` as a target —
// keeps both halves happy.
// `relay_sdp`, not `sdp`: `webrtc_ping::sdp` is the *page's* half (assembling
// an offer from trickled candidates) and these are the host's relay edits.
// Two modules of the same name in two halves of one crate is a reading tax.
#[path = "answerer/relay_sdp.rs"]
mod relay_sdp;
#[path = "answerer/turn_client.rs"]
mod turn_client;

/// The name component minted into every ephemeral TURN username
/// (`<expiry>:<name>`). Fixed rather than flagged — a probe, not a user store.
const TURN_USERNAME_NAME: &str = "mere";

/// One offer waiting for an answer, and the socket thread to send it back on.
struct OfferRequest {
    sdp: String,
    reply: blocking_mpsc::Sender<Result<String, String>>,
}

/// How this run is configured. Hand-parsed: a probe that reaches for `clap`
/// has started becoming a product.
struct Args {
    signal_port: u16,
    bind: SocketAddr,
    advertise: Vec<IpAddr>,
    once: bool,
    turn: TurnConfig,
}

/// What the `/turn-credentials` endpoint needs to mint a token, all optional.
///
/// The `secret` is the long-term coturn `static-auth-secret`; it stays on this
/// host. `urls` are the TURN server(s) the minted pair authenticates against,
/// echoed back so the page can hand them straight to the browser. `realm` is
/// returned for the caller's convenience (it is *not* part of the REST-style
/// username hash). Absent a `secret`, the endpoint answers `503`.
#[derive(Clone)]
struct TurnConfig {
    secret: Option<String>,
    realm: Option<String>,
    ttl: u64,
    urls: Vec<String>,
    /// Whether this process allocates its own relay address and offers it as a
    /// `typ relay` candidate, as opposed to only minting credentials for the
    /// browser.
    ///
    /// On by default once a `--turn-url` is configured — a browser held to
    /// `iceTransportPolicy: "relay"` discards every non-relay remote
    /// candidate, so an answer with only `typ host` in it has nothing to pair
    /// with and ICE never starts. `--no-turn-relay` keeps the minting endpoint
    /// while leaving this half out, which is what the direct receipts were
    /// taken under.
    relay: bool,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            secret: None,
            realm: None,
            ttl: 0,
            urls: Vec::new(),
            relay: true,
        }
    }
}

impl TurnConfig {
    /// The default TTL when neither `--turn-ttl` nor the env var sets one.
    const DEFAULT_TTL: u64 = 3600;

    /// The shortest credential this host mints *for itself*.
    ///
    /// The browser's token is deliberately short-lived, but RFC 5766 §4 binds
    /// an allocation to the username that created it, so the host cannot
    /// rotate credentials under a live allocation: every Refresh re-
    /// authenticates with the original pair, and a 300-second token means the
    /// allocation dies at the first refresh. The host's own credential is
    /// therefore minted for at least an hour. It never leaves this process.
    const MIN_HOST_TTL: u64 = 3600;
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Args {
            signal_port: 8787,
            // Unspecified, so datagrams sent to either advertised address
            // arrive on the one socket. Port 0: a fixed port cannot be rebound
            // while the previous session still holds it.
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            advertise: Vec::new(),
            once: false,
            turn: TurnConfig::default(),
        };
        let mut ttl_set = false;
        let mut raw = std::env::args().skip(1);
        while let Some(flag) = raw.next() {
            let mut value = || {
                raw.next()
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag.as_str() {
                "--signal-port" => {
                    args.signal_port = value()?.parse().map_err(|_| "bad --signal-port")?;
                }
                "--udp-port" => {
                    let port: u16 = value()?.parse().map_err(|_| "bad --udp-port")?;
                    args.bind.set_port(port);
                }
                "--bind" => {
                    let ip: IpAddr = value()?.parse().map_err(|_| "bad --bind address")?;
                    args.bind.set_ip(ip);
                }
                "--advertise" => {
                    let ip: IpAddr = value()?.parse().map_err(|_| "bad --advertise address")?;
                    args.advertise.push(ip);
                }
                "--turn-secret" => args.turn.secret = Some(value()?),
                "--turn-realm" => args.turn.realm = Some(value()?),
                "--turn-ttl" => {
                    args.turn.ttl = value()?.parse().map_err(|_| "bad --turn-ttl")?;
                    ttl_set = true;
                }
                "--turn-url" => args.turn.urls.push(value()?),
                "--no-turn-relay" => args.turn.relay = false,
                "--once" => args.once = true,
                "-h" | "--help" => {
                    println!(
                        "answerer [--signal-port 8787] [--udp-port 0] [--bind 0.0.0.0] \
                         [--advertise IP]... [--once]\n         \
                         [--turn-secret S] [--turn-realm R] [--turn-ttl 3600] [--turn-url URL]... \
                         [--no-turn-relay]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }

        // Env fallback for anything a flag did not set, so `run.ps1` can pass
        // these through from the environment without every caller repeating
        // them. Flags win; env fills the gaps.
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if args.turn.secret.is_none() {
            args.turn.secret = env("WEBRTC_PING_TURN_SECRET");
        }
        if args.turn.realm.is_none() {
            args.turn.realm = env("WEBRTC_PING_TURN_REALM");
        }
        if !ttl_set && let Some(raw) = env("WEBRTC_PING_TURN_TTL") {
            args.turn.ttl = raw.parse().map_err(|_| "bad WEBRTC_PING_TURN_TTL")?;
            ttl_set = true;
        }
        if args.turn.urls.is_empty() && let Some(raw) = env("WEBRTC_PING_TURN_URL") {
            args.turn.urls = raw
                .split([';', ','])
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if !ttl_set {
            args.turn.ttl = TurnConfig::DEFAULT_TTL;
        }
        Ok(args)
    }
}

// ── ephemeral TURN credentials (coturn REST convention) ─────────────────────

/// The credential minting the `/turn-credentials` endpoint hands out, and the
/// HMAC-SHA1 plus base64 it is built from.
///
/// Kept as free functions so the coturn-interop vector can be pinned by a unit
/// test that needs no socket and no clock — the security-critical byte-for-byte
/// agreement with coturn is asserted, not eyeballed against a running server.
mod turn {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    type HmacSha1 = Hmac<Sha1>;

    /// Standard-alphabet base64 (RFC 4648, with `=` padding). Hand-rolled
    /// rather than pulled: no base64 crate is in the probe's lock, and the
    /// digest is 20 fixed bytes.
    pub fn base64_standard(input: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((n >> 6) & 0x3f) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(n & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    /// Mints one coturn-REST ephemeral credential.
    ///
    /// `username = "<expiry_unix>:<name>"` (or just `"<expiry_unix>"` when
    /// `name` is empty), and `credential = base64(HMAC-SHA1(secret, username))`.
    /// The secret is the long-term key; the returned pair is what the browser
    /// uses and can safely see.
    pub fn mint(secret: &str, name: &str, expiry_unix: u64) -> (String, String) {
        let username = if name.is_empty() {
            format!("{expiry_unix}")
        } else {
            format!("{expiry_unix}:{name}")
        };
        // `new_from_slice` accepts any key length, so this never errors.
        let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
            .expect("HMAC-SHA1 accepts a key of any length");
        mac.update(username.as_bytes());
        let credential = base64_standard(&mac.finalize().into_bytes());
        (username, credential)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The HMAC-SHA1 primitive, against RFC 2202 test case 2. This is the
        /// standard vector for HMAC-SHA-1 itself, independent of base64 or the
        /// coturn framing — if this drifts, everything above it is suspect.
        #[test]
        fn hmac_sha1_matches_rfc2202_case2() {
            let mut mac = HmacSha1::new_from_slice(b"Jefe").unwrap();
            mac.update(b"what do ya want for nothing?");
            let hex: String = mac
                .finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(hex, "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
        }

        /// The full coturn REST shape, against a fixed secret + username whose
        /// base64 credential was computed by an independent implementation
        /// (Python `hmac`/`hashlib`/`base64`, the reference coturn agrees with).
        /// Proves byte-for-byte interop: a real coturn given this secret would
        /// accept this exact `username:credential`.
        #[test]
        fn coturn_rest_vector_secret_north() {
            let mut mac = HmacSha1::new_from_slice(b"north").unwrap();
            mac.update(b"12334939:mbzrxpgjys");
            let credential = base64_standard(&mac.finalize().into_bytes());
            assert_eq!(credential, "Iq7YXkRon8YXJfdN1Ke9EZOw1UE=");
        }

        /// `mint` end to end: username assembly and credential together, at a
        /// fixed expiry so the vector is deterministic.
        #[test]
        fn mint_end_to_end_at_fixed_expiry() {
            let (username, credential) = mint("mere-probe-secret", "mere", 1_700_000_000);
            assert_eq!(username, "1700000000:mere");
            assert_eq!(credential, "q20qgHstfrKn455bP1Q9dB1xV+U=");
        }

        /// An empty name yields the bare-timestamp username coturn also accepts.
        #[test]
        fn mint_without_a_name_is_the_bare_timestamp() {
            let (username, _) = mint("s", "", 1_700_000_000);
            assert_eq!(username, "1700000000");
        }

        #[test]
        fn base64_padding_is_rfc4648() {
            assert_eq!(base64_standard(b""), "");
            assert_eq!(base64_standard(b"f"), "Zg==");
            assert_eq!(base64_standard(b"fo"), "Zm8=");
            assert_eq!(base64_standard(b"foo"), "Zm9v");
            assert_eq!(base64_standard(b"foob"), "Zm9vYg==");
            assert_eq!(base64_standard(b"fooba"), "Zm9vYmE=");
        }
    }
}

// ── the loopback signalling fixture ─────────────────────────────────────────

fn signalling_thread(listener: TcpListener, offers: mpsc::Sender<OfferRequest>, turn: TurnConfig) {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(err) = serve_one(&mut stream, &offers, &turn) {
                    eprintln!("[signal] {err}");
                }
            }
            Err(err) => eprintln!("[signal] accept: {err}"),
        }
    }
}

fn serve_one(
    stream: &mut TcpStream,
    offers: &mpsc::Sender<OfferRequest>,
    turn: &TurnConfig,
) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or("/").to_owned();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) => respond(stream, "204 No Content", ""),
        ("GET", "/health") => respond(stream, "200 OK", "ok"),
        ("GET", "/turn-credentials") => turn_credentials(stream, turn),
        ("POST", "/offer") => {
            let sdp = String::from_utf8_lossy(&body).into_owned();
            if sdp.trim().is_empty() {
                return respond(stream, "400 Bad Request", "empty offer");
            }
            println!("[signal] offer received, {} bytes", sdp.len());
            let (reply, answers) = blocking_mpsc::channel();
            if offers.blocking_send(OfferRequest { sdp, reply }).is_err() {
                return respond(stream, "503 Service Unavailable", "answerer is shutting down");
            }
            match answers.recv() {
                Ok(Ok(answer)) => respond(stream, "200 OK", &answer),
                Ok(Err(err)) => respond(stream, "500 Internal Server Error", &err),
                Err(_) => respond(stream, "500 Internal Server Error", "answerer dropped the offer"),
            }
        }
        _ => respond(stream, "404 Not Found", "try POST /offer or GET /health"),
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    respond_with(stream, status, "text/plain; charset=utf-8", body)
}

/// The minted-credentials endpoint speaks JSON; everything else is `text/plain`.
fn respond_json(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    respond_with(stream, status, "application/json; charset=utf-8", body)
}

fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Max-Age: 600\r\n\
         Cache-Control: no-store\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// Mints one short-lived TURN credential and returns it as JSON.
///
/// `{"username","credential","ttl","urls":[…]}`, plus `"realm"` when one is
/// configured. Answers `503` when no `--turn-secret`/`WEBRTC_PING_TURN_SECRET`
/// is set, so the fixture still serves `/offer` for direct receipts. The
/// long-term secret is used here to compute the HMAC and is never placed in the
/// response.
fn turn_credentials(stream: &mut TcpStream, turn: &TurnConfig) -> std::io::Result<()> {
    let Some(secret) = turn.secret.as_deref() else {
        return respond(
            stream,
            "503 Service Unavailable",
            "no TURN configured — set --turn-secret (or WEBRTC_PING_TURN_SECRET)",
        );
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let expiry = now.saturating_add(turn.ttl);
    let (username, credential) = turn::mint(secret, TURN_USERNAME_NAME, expiry);

    let urls = turn
        .urls
        .iter()
        .map(|u| format!("\"{}\"", json_escape(u)))
        .collect::<Vec<_>>()
        .join(",");
    let realm = match &turn.realm {
        Some(realm) => format!(",\"realm\":\"{}\"", json_escape(realm)),
        None => String::new(),
    };
    let body = format!(
        "{{\"username\":\"{}\",\"credential\":\"{}\",\"ttl\":{},\"urls\":[{}]{}}}",
        json_escape(&username),
        json_escape(&credential),
        turn.ttl,
        urls,
        realm,
    );
    respond_json(stream, "200 OK", &body)
}

// ── interface discovery ─────────────────────────────────────────────────────

/// Resolves a `turn:` URL to the one UDP address this client will allocate on.
///
/// Deliberately narrow: `turn:` only (a `turns:` URL is TLS and this client
/// speaks plain UDP), `?transport=udp` or nothing, and the RFC 7065 default
/// port 3478 when none is given. Hostnames resolve; the first UDP-capable
/// address wins.
async fn resolve_turn_url(url: &str) -> Result<SocketAddr, String> {
    let rest = url
        .strip_prefix("turn:")
        .ok_or_else(|| format!("{url}: only turn: URLs are supported (turns: is TLS)"))?;
    let (authority, query) = match rest.split_once('?') {
        Some((authority, query)) => (authority, Some(query)),
        None => (rest, None),
    };
    if let Some(query) = query {
        for parameter in query.split('&') {
            if let Some(transport) = parameter.strip_prefix("transport=")
                && !transport.eq_ignore_ascii_case("udp")
            {
                return Err(format!("{url}: transport={transport} is not supported"));
            }
        }
    }
    // An IPv6 literal is bracketed, so its own colons are not a port; anything
    // else carries a port if it has a colon at all.
    let has_port = if authority.starts_with('[') {
        authority.contains("]:")
    } else {
        authority.contains(':')
    };
    let with_port = if has_port {
        authority.to_owned()
    } else {
        format!("{authority}:3478")
    };
    tokio::net::lookup_host(&with_port)
        .await
        .map_err(|err| format!("{url}: {err}"))?
        .next()
        .ok_or_else(|| format!("{url}: resolved to no address"))
}

/// The address the OS would use to reach the wider network, learned by route
/// lookup rather than by enumerating interfaces. No packet is sent.
fn primary_local_ip() -> Option<IpAddr> {
    for probe in ["8.8.8.8:53", "203.0.113.1:53"] {
        if let Ok(socket) = std::net::UdpSocket::bind(("0.0.0.0", 0))
            && socket.connect(probe).is_ok()
            && let Ok(addr) = socket.local_addr()
            && !addr.ip().is_unspecified()
            && !addr.ip().is_loopback()
        {
            return Some(addr.ip());
        }
    }
    None
}

// ── the receipt line ────────────────────────────────────────────────────────

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// One line, one session, every number the receipt needs.
fn receipt_line(
    session: u64,
    frames_echoed: u64,
    bytes_echoed: u64,
    ended: &str,
    local_addr: SocketAddr,
    channel_label: &str,
    stats: &CarrierStats,
    fingerprints: Option<&SessionFingerprints>,
) -> String {
    let fingerprints = match fingerprints {
        Some(pair) => format!(
            "{{\"client\":\"{}\",\"server\":\"{}\"}}",
            pair.client().to_sdp_hex(),
            pair.server().to_sdp_hex()
        ),
        None => "null".to_owned(),
    };
    format!(
        "WEBRTC_PING_RECEIPT {{\
\"session\":{session},\
\"local_addr\":\"{local_addr}\",\
\"channel_label\":\"{}\",\
\"ended\":\"{}\",\
\"frames_echoed\":{frames_echoed},\
\"bytes_echoed\":{bytes_echoed},\
\"frames_received\":{},\"frames_sent\":{},\"frames_enqueued\":{},\"frames_dropped\":{},\
\"bytes_received\":{},\"bytes_sent\":{},\
\"datagrams_received\":{},\"datagrams_sent\":{},\"malformed_datagrams\":{},\"socket_resets\":{},\
\"peak_queued\":{},\"queued_outbound\":{},\"sctp_buffered\":{},\
\"pauses\":{},\"resumes\":{},\"low_water_events\":{},\"write_waits\":{},\
\"iterations\":{},\"timeouts\":{},\
\"fingerprints\":{fingerprints}}}",
        json_escape(channel_label),
        json_escape(ended),
        stats.frames_received,
        stats.frames_sent,
        stats.frames_enqueued,
        stats.frames_dropped,
        stats.bytes_received,
        stats.bytes_sent,
        stats.datagrams_received,
        stats.datagrams_sent,
        stats.malformed_datagrams,
        stats.socket_resets,
        stats.peak_queued,
        stats.queued_outbound,
        stats.sctp_buffered,
        stats.pauses,
        stats.resumes,
        stats.low_water_events,
        stats.write_waits,
        stats.iterations,
        stats.timeouts,
    )
}

#[cfg(test)]
mod receipt_tests {
    use super::*;

    /// The receipt line is the probe's whole output contract. A line that does
    /// not parse is a silent failure — the headed run would look successful and
    /// the receipt would have nothing to quote — so its shape is asserted here
    /// rather than eyeballed once.
    #[test]
    fn the_receipt_line_is_one_line_of_balanced_json() {
        let line = receipt_line(
            7,
            16,
            16_384,
            "peer \"closed\"\n\tthe\\channel",
            "127.0.0.1:51820".parse().unwrap(),
            "mere-graphshell",
            &CarrierStats::default(),
            None,
        );
        assert!(line.starts_with("WEBRTC_PING_RECEIPT {"));
        assert!(!line.contains('\n'), "the receipt must stay on one line");
        let json = &line["WEBRTC_PING_RECEIPT ".len()..];
        assert_eq!(json.matches('{').count(), json.matches('}').count());
        assert!(json.ends_with('}'));
        // Every control character and quote in `ended` survived as an escape.
        assert!(json.contains(r#""ended":"peer \"closed\"\n\tthe\\channel""#));
        assert!(json.contains(r#""fingerprints":null"#));
        assert!(json.contains(r#""session":7"#));
        assert!(json.contains(r#""frames_echoed":16"#));
        assert!(json.contains(r#""iterations":0"#));
    }

    #[test]
    fn json_escape_covers_the_control_range() {
        assert_eq!(json_escape("\u{1}"), "\\u0001");
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    }
}

// ── the session ─────────────────────────────────────────────────────────────

/// Waits for the channel, speaks the revisioned ping protocol, and prints the
/// receipt line.
///
/// `preempt` fires when a refreshed page posts a new offer. It is `biased`
/// first in both `select!`s so a preempted session ends promptly instead of
/// waiting on a peer that is already gone.
///
/// `log` is the process-wide revision state, shared across sessions so a
/// reconnecting browser can resume against revisions an earlier connection
/// advanced. A browser HELLO is answered by DIFF or SNAPSHOT; a PING advances
/// the log and comes back as a stamped PONG; an unrecognised frame is echoed
/// raw. Only echoes (PONGs and raw echoes) count toward the receipt's
/// `frames_echoed`/`bytes_echoed`, since a resume reply is not an echo.
async fn run_session(
    session: u64,
    answerer: Answerer,
    mut preempt: oneshot::Receiver<()>,
    log: Arc<Mutex<RevisionLog>>,
) {
    let carrier: Carrier = tokio::select! {
        biased;
        _ = &mut preempt => {
            println!("[session {session}] preempted before the data channel opened");
            return;
        }
        opened = answerer.accept() => match opened {
            Ok(carrier) => carrier,
            Err(err) => {
                println!("[session {session}] the data channel never opened: {err}");
                return;
            }
        }
    };

    let local_addr = carrier.local_addr();
    let channel_label = carrier.channel_label().to_owned();
    println!(
        "[session {session}] OPEN  label {channel_label:?}  udp {local_addr}  \
         high water {} B  low water {} B",
        carrier.backpressure().high_water_bytes(),
        carrier.backpressure().low_water_bytes(),
    );
    if let Some(pair) = carrier.fingerprints() {
        println!(
            "[session {session}] dtls client {} ",
            pair.client().to_sdp_hex()
        );
        println!(
            "[session {session}] dtls server {} ",
            pair.server().to_sdp_hex()
        );
    }

    let (mut reader, writer, control) = carrier.into_parts();
    let mut frames_echoed = 0u64;
    let mut bytes_echoed = 0u64;
    let mut ended = String::from("peer closed the channel");

    loop {
        let frame = tokio::select! {
            biased;
            _ = &mut preempt => {
                ended = "preempted by a new offer".to_owned();
                break;
            }
            frame = reader.recv_frame() => frame,
        };
        match frame {
            Ok(Some(payload)) => {
                // The reply for one incoming frame, decided under the lock but
                // sent outside it — the guard never spans the `.await`.
                let reply = match proto::decode_client(&payload) {
                    Some(ClientMsg::Hello { resume_from }) => {
                        let decision = log
                            .lock()
                            .expect("revision log mutex poisoned")
                            .resume(resume_from);
                        let bytes = match decision {
                            ResumeReply::Diff { from, to, bytes } => {
                                println!(
                                    "[session {session}] resume from {resume_from} -> diff {from}..={to} ({bytes} B){}",
                                    if from > to { " (already current)" } else { "" }
                                );
                                proto::encode_diff(from, to, bytes)
                            }
                            ResumeReply::Snapshot {
                                revision,
                                total_bytes,
                            } => {
                                println!(
                                    "[session {session}] resume from {resume_from} -> snapshot (rev {revision}, {total_bytes} B)"
                                );
                                proto::encode_snapshot(revision, total_bytes)
                            }
                        };
                        // A resume reply is not an echo; it does not count.
                        bytes
                    }
                    Some(ClientMsg::Ping { client_seq, body_len }) => {
                        let revision = log
                            .lock()
                            .expect("revision log mutex poisoned")
                            .apply(body_len as u64);
                        let pong = proto::encode_pong(revision, client_seq);
                        frames_echoed += 1;
                        bytes_echoed += pong.len() as u64;
                        pong
                    }
                    // Unrecognised framing: echo it back verbatim, as C1 did,
                    // so a non-protocol sender still sees a working channel.
                    None => {
                        frames_echoed += 1;
                        bytes_echoed += payload.len() as u64;
                        payload
                    }
                };
                if let Err(err) = writer.send_frame(&reply).await {
                    ended = format!("reply write failed: {err}");
                    break;
                }
            }
            Ok(None) => break,
            Err(err) => {
                ended = err.to_string();
                break;
            }
        }
    }

    // Captured before `close` consumes the control handle.
    let fingerprints = control.fingerprints();
    drop(writer);
    if let Err(err) = control.close().await {
        println!("[session {session}] close: {err}");
    }
    // Read after the driver has joined, so the counters are final.
    let stats = reader.stats();
    println!(
        "{}",
        receipt_line(
            session,
            frames_echoed,
            bytes_echoed,
            &ended,
            local_addr,
            &channel_label,
            &stats,
            fingerprints.as_ref(),
        )
    );
}

// ── main ────────────────────────────────────────────────────────────────────

async fn bind_and_report(config: &AnswererConfig) -> Result<Answerer, NativeError> {
    let mut answerer = Answerer::bind(config.clone()).await?;
    println!("  carrier UDP   {}", answerer.local_addr());
    println!(
        "  advertising   {}",
        config
            .advertise
            .iter()
            .map(|ip| format!("{ip}:{}", answerer.local_addr().port()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    match answerer.local_fingerprint() {
        Ok(fingerprint) => println!(
            "  local DTLS    {} {}",
            fingerprint.role(),
            fingerprint.to_sdp_hex()
        ),
        Err(err) => println!("  local DTLS    unavailable: {err}"),
    }
    Ok(answerer)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse().map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, args.signal_port))?;
    let signal_addr = listener.local_addr()?;
    let turn = args.turn.clone();
    let turn_enabled = turn.secret.is_some();
    let (offers_tx, mut offers) = mpsc::channel::<OfferRequest>(4);
    std::thread::Builder::new()
        .name("webrtc-ping-signalling".to_owned())
        .spawn(move || signalling_thread(listener, offers_tx, turn))?;

    // One revision log for the whole process: reconnecting browsers resume
    // against revisions an earlier connection advanced.
    let log = Arc::new(Mutex::new(RevisionLog::new(proto::DEFAULT_RETAIN)));

    let mut advertise = args.advertise.clone();
    if advertise.is_empty() {
        advertise.push(IpAddr::V4(Ipv4Addr::LOCALHOST));
        if let Some(ip) = primary_local_ip() {
            advertise.push(ip);
        }
    }
    let config = AnswererConfig {
        bind: args.bind,
        advertise,
        carrier: CarrierConfig::default(),
    };

    println!("webrtc-ping answerer  (testing/webrtc-ping)");
    println!("  offer endpoint  POST http://{signal_addr}/offer   (text/plain: offer SDP in, answer SDP out)");
    println!("  health          GET  http://{signal_addr}/health");
    if turn_enabled {
        println!(
            "  turn creds      GET  http://{signal_addr}/turn-credentials   (json: minted username/credential, ttl {}s, {} url(s))",
            args.turn.ttl,
            args.turn.urls.len()
        );
    } else {
        println!(
            "  turn creds      GET  http://{signal_addr}/turn-credentials   (503: no --turn-secret set)"
        );
    }
    // The relay half of C3. Allocating here rather than per session means the
    // relay address is a property of the *process*, so it can be printed in
    // the banner and quoted in a receipt before any browser shows up — and one
    // allocation is refreshed for the whole run instead of one per refresh of
    // the page.
    let relay = match relay_if_configured(&args).await {
        Ok(relay) => relay,
        Err(err) => {
            eprintln!("  turn relay    unavailable: {err}");
            None
        }
    };
    if let Some(relay) = &relay {
        println!(
            "  turn relay    {} on {}   (lifetime {}s, mapped {})",
            relay.relayed(),
            relay.server(),
            relay.lifetime().as_secs(),
            relay
                .mapped()
                .map_or_else(|| "unknown".to_owned(), |addr| addr.to_string()),
        );
    }

    let mut pending = Some(bind_and_report(&config).await?);
    println!("READY — waiting for a browser offer. Ctrl-C to stop.");

    let mut session = 0u64;
    let mut inflight: Option<(oneshot::Sender<()>, tokio::task::JoinHandle<()>)> = None;

    while let Some(request) = offers.recv().await {
        if let Some((preempt, handle)) = inflight.take() {
            let _ = preempt.send(());
            let _ = handle.await;
        }
        session += 1;
        println!("[session {session}] answering");
        let mut answerer = match pending.take() {
            Some(answerer) => answerer,
            None => match bind_and_report(&config).await {
                Ok(answerer) => answerer,
                Err(err) => {
                    let _ = request.reply.send(Err(format!("bind: {err}")));
                    continue;
                }
            },
        };

        // Relay mode. Every peer candidate is re-pointed at a local shim
        // socket, so the packets str0m sends to the peer land on a socket this
        // process owns and get framed toward the relay; the answer then gains
        // the `typ relay` line the browser needs to pair at all. When no
        // relay is configured the offer travels to str0m verbatim, which is
        // the direct path the C1 receipts were taken on.
        let offer_for_str0m = match &relay {
            None => request.sdp.clone(),
            Some(relay) => {
                let peers = relay_sdp::peer_candidates(&request.sdp);
                match relay.open(peers, answerer.candidate_addr()).await {
                    Ok(maps) => {
                        for map in &maps {
                            println!(
                                "[session {session}] relay {} <- channel 0x{:04x} -> peer {} \
                                 (shim {})",
                                relay.relayed(),
                                map.channel,
                                map.peer,
                                map.shim,
                            );
                        }
                        if maps.is_empty() {
                            println!(
                                "[session {session}] the offer named no usable UDP candidate; \
                                 relaying to whatever the allocation delivers"
                            );
                        }
                        let shims: HashMap<SocketAddr, SocketAddr> =
                            maps.iter().map(|map| (map.peer, map.shim)).collect();
                        relay_sdp::rewrite_candidates(&request.sdp, &shims)
                    }
                    Err(err) => {
                        println!("[session {session}] relay setup failed: {err}");
                        request.sdp.clone()
                    }
                }
            }
        };

        match answerer.answer(&offer_for_str0m) {
            Ok(answer) => {
                let answer = match &relay {
                    Some(relay) => {
                        relay_sdp::inject_relay_candidate(&answer, relay.relayed(), relay.mapped())
                    }
                    None => answer,
                };
                let _ = request.reply.send(Ok(answer));
            }
            Err(err) => {
                println!("[session {session}] refused the offer: {err}");
                let _ = request.reply.send(Err(err.to_string()));
                // The `Rtc` has seen a bad offer; start the next one clean.
                continue;
            }
        }
        let (preempt_tx, preempt_rx) = oneshot::channel();
        if args.once {
            run_session(session, answerer, preempt_rx, log.clone()).await;
            drop(preempt_tx);
            break;
        }
        inflight = Some((
            preempt_tx,
            tokio::spawn(run_session(session, answerer, preempt_rx, log.clone())),
        ));
    }

    if let Some((preempt, handle)) = inflight {
        let _ = preempt.send(());
        let _ = handle.await;
    }
    if let Some(relay) = &relay {
        relay.close().await;
    }
    Ok(())
}

/// Allocates a relay address, if this run is configured for one.
///
/// `None` — not an error — when there is no `--turn-url`, no `--turn-secret`
/// to mint against, or `--no-turn-relay`. An allocation that *fails* is an
/// error, because a run asked for relay mode and did not get it should say so
/// rather than quietly hand out `typ host` answers a relay-policy browser will
/// throw away.
async fn relay_if_configured(args: &Args) -> Result<Option<turn_client::RelayHandle>, String> {
    let (Some(secret), Some(url)) = (args.turn.secret.as_deref(), args.turn.urls.first()) else {
        return Ok(None);
    };
    if !args.turn.relay {
        return Ok(None);
    }
    let server = resolve_turn_url(url).await?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ttl = args.turn.ttl.max(TurnConfig::MIN_HOST_TTL);
    let (username, credential) = turn::mint(secret, TURN_USERNAME_NAME, now.saturating_add(ttl));

    // The TURN socket binds on the same interface the carrier does when one
    // was named, so a host with several routes relays over the one whose
    // address the answer advertises.
    let local_ip = if args.bind.ip().is_unspecified() {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        args.bind.ip()
    };
    turn_client::RelayHandle::start(server, local_ip, &username, &credential)
        .await
        .map(Some)
        .map_err(|err| err.to_string())
}
