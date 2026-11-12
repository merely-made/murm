//! A hand-rolled TURN client: enough of RFC 5389/5766 to allocate a relay
//! address, permit a peer, bind a channel, and pump packets across it.
//!
//! ## Why hand-rolled
//!
//! The str0m bake-off rejected pulling webrtc-rs's stack, and `rtc-turn` is
//! not a dependency of this probe. What C3 actually needs is four request
//! types and one framing header, so the whole client is this file. Nothing
//! here is a general TURN library: it speaks UDP to one server, holds one
//! allocation, and exists to put a `typ relay` candidate in an answer.
//!
//! ## The wire sequence, as validated against coturn 4.17.2
//!
//! ```text
//! 1. Allocate (0x0003) + REQUESTED-TRANSPORT(UDP)
//!      -> 0x0113 error 401, carrying REALM + NONCE
//! 2. Allocate (0x0003) + REQUESTED-TRANSPORT, USERNAME, REALM, NONCE,
//!    MESSAGE-INTEGRITY (last)
//!      -> 0x0103 success, carrying XOR-RELAYED-ADDRESS + LIFETIME
//! ```
//!
//! The subtlety that makes or breaks step 2, and the one every from-scratch
//! implementation gets wrong once: the header's LENGTH field must *already*
//! count the 24-byte MESSAGE-INTEGRITY attribute when the HMAC is computed,
//! but the HMAC covers only the bytes *before* MI is appended. See
//! [`MessageBuilder::finish_with_integrity`], which is the only place that
//! ordering is expressed, and [`tests::rfc5769_long_term_credential_request`],
//! which pins it against RFC 5769 §2.4's published vector.
//!
//! The long-term credential key is `MD5("username:realm:password")`. No MD5
//! crate is in this probe's lock and none is being added for ~70 lines of
//! well-specified arithmetic, so [`md5`] is here too, pinned against RFC 1321.
//!
//! ## The data path is ChannelData, not Send/Data indications
//!
//! Both are implemented on the receive side — a server may deliver a Data
//! indication (0x0017) for a permitted peer before a channel exists, and
//! dropping those would lose the first packets of a session — but everything
//! this client *sends* to a peer goes out as ChannelData: a 4-byte header
//! against a Send indication's 36-plus bytes of STUN header and attributes,
//! on every packet, for the whole session.
//!
//! ## The shim sockets, and why the relay is invisible to str0m
//!
//! `webrtc-carrier` owns its UDP socket and has no seam for a relayed
//! transport, so the relay is arranged *around* it rather than inside it.
//! Each peer address gets a local "shim" UDP socket bound next to the
//! carrier's own address:
//!
//! ```text
//!   browser <--> TURN server <--UDP--> turn socket ) pump ( shim_P <--UDP--> carrier/str0m
//! ```
//!
//! The offer handed to str0m has that peer's candidate rewritten to `shim_P`,
//! so every packet str0m sends to the peer lands on a socket this module owns
//! and gets framed toward the relay; every ChannelData the relay delivers is
//! de-framed and sent from `shim_P` to the carrier, which attributes it to its
//! own advertised address exactly as if it had arrived directly. str0m never
//! learns a relay exists, and `webrtc-carrier` is not touched.

// The wire constants below are a table, not a menu: the ones this client never
// sends are still what a capture has to be read against, and the parser is
// written to be complete rather than to be exactly as large as today's caller.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha1::Sha1;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

type HmacSha1 = Hmac<Sha1>;

// ── wire constants ──────────────────────────────────────────────────────────

/// RFC 5389's magic cookie. Present in every message and the XOR key for
/// every XOR-`*`-ADDRESS attribute.
pub const MAGIC_COOKIE: u32 = 0x2112_A442;
const COOKIE_BYTES: [u8; 4] = MAGIC_COOKIE.to_be_bytes();

/// Type (2) + length (2) + cookie (4) + transaction id (12).
const HEADER_LEN: usize = 20;
/// A MESSAGE-INTEGRITY attribute on the wire: 4 header bytes + a 20-byte HMAC.
const MI_TLV_LEN: usize = 24;

// Methods (RFC 5389 §18.1, RFC 5766 §13).
pub const METHOD_BINDING: u16 = 0x001;
pub const METHOD_ALLOCATE: u16 = 0x003;
pub const METHOD_REFRESH: u16 = 0x004;
pub const METHOD_SEND: u16 = 0x006;
pub const METHOD_DATA: u16 = 0x007;
pub const METHOD_CREATE_PERMISSION: u16 = 0x008;
pub const METHOD_CHANNEL_BIND: u16 = 0x009;

// Classes, as the two-bit value the interleave in `message_type` scatters.
pub const CLASS_REQUEST: u16 = 0b00;
pub const CLASS_INDICATION: u16 = 0b01;
pub const CLASS_SUCCESS: u16 = 0b10;
pub const CLASS_ERROR: u16 = 0b11;

// Attributes.
pub const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_ERROR_CODE: u16 = 0x0009;
pub const ATTR_CHANNEL_NUMBER: u16 = 0x000C;
pub const ATTR_LIFETIME: u16 = 0x000D;
pub const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;
pub const ATTR_DATA: u16 = 0x0013;
pub const ATTR_REALM: u16 = 0x0014;
pub const ATTR_NONCE: u16 = 0x0015;
pub const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
pub const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// The IANA protocol number for UDP, as REQUESTED-TRANSPORT carries it.
const TRANSPORT_UDP: [u8; 4] = [17, 0, 0, 0];

/// Channel numbers a client may bind (RFC 5766 §11). `0x4000..=0x7FFF`, which
/// is also exactly the first-byte range that tells ChannelData apart from a
/// STUN message (whose first two bits are zero).
const CHANNEL_MIN: u16 = 0x4000;
const CHANNEL_MAX: u16 = 0x7FFF;

/// How long to wait for one response before retransmitting.
const RTO: Duration = Duration::from_millis(500);
/// How many times a request is sent before it is called lost.
const MAX_ATTEMPTS: u32 = 5;

/// Assembles a STUN message type from its method and class.
///
/// The class bits are *scattered* through the type rather than sitting in a
/// field of their own, which is why `Allocate` success is `0x0103` and not
/// `0x0203`. Pinned by [`tests::message_types_match_the_published_constants`].
pub fn message_type(method: u16, class: u16) -> u16 {
    (method & 0x000F)
        | ((method & 0x0070) << 1)
        | ((method & 0x0F80) << 2)
        | ((class & 0b01) << 4)
        | ((class & 0b10) << 7)
}

/// The method half of a message type.
pub fn method_of(msg_type: u16) -> u16 {
    (msg_type & 0x000F) | ((msg_type & 0x00E0) >> 1) | ((msg_type & 0x3E00) >> 2)
}

/// The class half of a message type, as one of the `CLASS_*` values.
pub fn class_of(msg_type: u16) -> u16 {
    ((msg_type & 0x0010) >> 4) | ((msg_type & 0x0100) >> 7)
}

// ── MD5, for the long-term credential key ───────────────────────────────────

/// RFC 1321 MD5.
///
/// Here because `MD5("username:realm:password")` *is* the long-term credential
/// key TURN authenticates with, no MD5 crate is in this probe's `Cargo.lock`,
/// and the alternative to seventy lines of arithmetic is a new dependency in a
/// crate whose whole point is having few. Not used for anything but that key —
/// MD5's collision weakness is irrelevant to an HMAC key derivation, which is
/// why RFC 5389 still specifies it.
pub fn md5(input: &[u8]) -> [u8; 16] {
    #[rustfmt::skip]
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
        5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    /// `floor(2^32 * abs(sin(i + 1)))`, the table RFC 1321 tabulates.
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
        0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
        0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
        0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
        0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
        0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
        0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];

    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

    let mut message = Vec::with_capacity(input.len() + 72);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(input.len() as u64).wrapping_mul(8).to_le_bytes());

    for block in message.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (word, bytes) in m.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64usize {
            let (mix, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let mix = mix
                .wrapping_add(a)
                .wrapping_add(K[i])
                .wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(mix.rotate_left(S[i]));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut out = [0u8; 16];
    for (chunk, word) in out.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// The long-term credential key: `MD5(username ":" realm ":" password)`.
pub fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    md5(format!("{username}:{realm}:{password}").as_bytes())
}

// ── message building ────────────────────────────────────────────────────────

/// Builds one STUN/TURN message, in attribute order, with the length field
/// patched at the end.
pub struct MessageBuilder {
    buf: Vec<u8>,
}

impl MessageBuilder {
    /// A message with the given type and transaction id and no attributes.
    pub fn new(msg_type: u16, txid: &[u8; 12]) -> Self {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&msg_type.to_be_bytes());
        buf.extend_from_slice(&[0, 0]); // patched by `finish*`
        buf.extend_from_slice(&COOKIE_BYTES);
        buf.extend_from_slice(txid);
        Self { buf }
    }

    /// Appends one attribute, zero-padded to the next 4-byte boundary.
    ///
    /// RFC 5389 lets padding take any value; zero is what coturn emits and
    /// what makes a captured message reproducible byte for byte.
    pub fn push(&mut self, attr: u16, value: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(&attr.to_be_bytes());
        self.buf
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.buf.extend_from_slice(value);
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
        self
    }

    /// Writes the header LENGTH as *this* body plus `extra` further bytes.
    fn set_length(&mut self, extra: usize) {
        let length = (self.buf.len() - HEADER_LEN + extra) as u16;
        self.buf[2..4].copy_from_slice(&length.to_be_bytes());
    }

    /// The finished message, unauthenticated.
    pub fn finish(mut self) -> Vec<u8> {
        self.set_length(0);
        self.buf
    }

    /// The finished message with MESSAGE-INTEGRITY appended.
    ///
    /// **The ordering is the whole trick.** The header LENGTH is set to
    /// include the 24-byte MI attribute *first*, then the HMAC is taken over
    /// the bytes as they stand — header (with the already-inflated length) plus
    /// every attribute, but *not* MI itself — and only then is MI appended. A
    /// client that hashes with the pre-MI length gets a 401 back from every
    /// server and no clue why.
    pub fn finish_with_integrity(mut self, key: &[u8]) -> Vec<u8> {
        self.set_length(MI_TLV_LEN);
        let mut mac =
            HmacSha1::new_from_slice(key).expect("HMAC-SHA1 accepts a key of any length");
        mac.update(&self.buf);
        let digest = mac.finalize().into_bytes();
        self.buf
            .extend_from_slice(&ATTR_MESSAGE_INTEGRITY.to_be_bytes());
        self.buf.extend_from_slice(&20u16.to_be_bytes());
        self.buf.extend_from_slice(&digest);
        self.buf
    }
}

// ── message parsing ─────────────────────────────────────────────────────────

/// A parsed view over one received message. Borrows the receive buffer.
#[derive(Debug)]
pub struct Message<'a> {
    pub msg_type: u16,
    pub txid: [u8; 12],
    attrs: Vec<(u16, &'a [u8])>,
}

impl<'a> Message<'a> {
    /// Parses a datagram, or `None` if it is not a well-formed STUN message.
    ///
    /// Refuses anything whose first two bits are not zero, which is what keeps
    /// ChannelData framing (first byte `0x40..=0x7F`) from being mistaken for
    /// a message.
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || bytes[0] & 0xC0 != 0 {
            return None;
        }
        let msg_type = u16::from_be_bytes([bytes[0], bytes[1]]);
        let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if bytes[4..8] != COOKIE_BYTES || bytes.len() < HEADER_LEN + length {
            return None;
        }
        let mut txid = [0u8; 12];
        txid.copy_from_slice(&bytes[8..20]);

        let mut attrs = Vec::new();
        let body = &bytes[HEADER_LEN..HEADER_LEN + length];
        let mut at = 0usize;
        while at + 4 <= body.len() {
            let attr = u16::from_be_bytes([body[at], body[at + 1]]);
            let len = u16::from_be_bytes([body[at + 2], body[at + 3]]) as usize;
            let start = at + 4;
            let end = start.checked_add(len)?;
            if end > body.len() {
                return None;
            }
            attrs.push((attr, &body[start..end]));
            at = end + ((4 - len % 4) % 4);
        }
        Some(Self {
            msg_type,
            txid,
            attrs,
        })
    }

    pub fn method(&self) -> u16 {
        method_of(self.msg_type)
    }

    pub fn class(&self) -> u16 {
        class_of(self.msg_type)
    }

    /// The first occurrence of an attribute.
    pub fn attr(&self, attr: u16) -> Option<&'a [u8]> {
        self.attrs
            .iter()
            .find(|(kind, _)| *kind == attr)
            .map(|(_, value)| *value)
    }

    /// Every occurrence of an attribute, in wire order. CreatePermission and
    /// its kin repeat XOR-PEER-ADDRESS.
    pub fn attrs(&self, attr: u16) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.attrs
            .iter()
            .filter(move |(kind, _)| *kind == attr)
            .map(|(_, value)| *value)
    }

    /// Decodes one of the XOR-`*`-ADDRESS attributes.
    pub fn xor_address(&self, attr: u16) -> Option<SocketAddr> {
        decode_xor_address(self.attr(attr)?, &self.txid)
    }

    /// A `(code, reason)` pair from ERROR-CODE.
    pub fn error_code(&self) -> Option<(u16, String)> {
        let value = self.attr(ATTR_ERROR_CODE)?;
        if value.len() < 4 {
            return None;
        }
        let code = u16::from(value[2] & 0x07) * 100 + u16::from(value[3]);
        Some((code, String::from_utf8_lossy(&value[4..]).into_owned()))
    }

    /// LIFETIME, in seconds.
    pub fn lifetime(&self) -> Option<u32> {
        let value = self.attr(ATTR_LIFETIME)?;
        let bytes: [u8; 4] = value.get(..4)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    fn text(&self, attr: u16) -> Option<String> {
        self.attr(attr)
            .map(|v| String::from_utf8_lossy(v).into_owned())
    }
}

/// Decodes an XOR-MAPPED / XOR-PEER / XOR-RELAYED address value.
///
/// The port is XORed with the top half of the cookie, an IPv4 address with the
/// whole cookie, an IPv6 address with the cookie followed by the transaction
/// id. XOR is its own inverse, so [`encode_xor_address`] is the same operation.
pub fn decode_xor_address(value: &[u8], txid: &[u8; 12]) -> Option<SocketAddr> {
    if value.len() < 4 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]) ^ (MAGIC_COOKIE >> 16) as u16;
    match value[1] {
        0x01 if value.len() >= 8 => {
            let mut octets = [0u8; 4];
            for (out, (byte, key)) in octets
                .iter_mut()
                .zip(value[4..8].iter().zip(COOKIE_BYTES.iter()))
            {
                *out = byte ^ key;
            }
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port))
        }
        0x02 if value.len() >= 20 => {
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&COOKIE_BYTES);
            key[4..].copy_from_slice(txid);
            let mut octets = [0u8; 16];
            for (out, (byte, key)) in octets
                .iter_mut()
                .zip(value[4..20].iter().zip(key.iter()))
            {
                *out = byte ^ key;
            }
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        _ => None,
    }
}

/// Encodes an address as an XOR-`*`-ADDRESS attribute value.
pub fn encode_xor_address(addr: SocketAddr, txid: &[u8; 12]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.push(0);
    let port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
    match addr.ip() {
        IpAddr::V4(v4) => {
            out.push(0x01);
            out.extend_from_slice(&port.to_be_bytes());
            for (byte, key) in v4.octets().iter().zip(COOKIE_BYTES.iter()) {
                out.push(byte ^ key);
            }
        }
        IpAddr::V6(v6) => {
            out.push(0x02);
            out.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&COOKIE_BYTES);
            key[4..].copy_from_slice(txid);
            for (byte, key) in v6.octets().iter().zip(key.iter()) {
                out.push(byte ^ key);
            }
        }
    }
    out
}

// ── ChannelData framing ─────────────────────────────────────────────────────

/// Wraps a payload in the 4-byte ChannelData header (RFC 5766 §11.4).
///
/// No padding: it is required over stream transports and merely permitted over
/// UDP, and this client is UDP only.
pub fn channel_data(channel: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&channel.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Splits a ChannelData datagram into `(channel, payload)`.
pub fn parse_channel_data(bytes: &[u8]) -> Option<(u16, &[u8])> {
    if bytes.len() < 4 {
        return None;
    }
    let channel = u16::from_be_bytes([bytes[0], bytes[1]]);
    if !(CHANNEL_MIN..=CHANNEL_MAX).contains(&channel) {
        return None;
    }
    let len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    bytes.get(4..4 + len).map(|payload| (channel, payload))
}

// ── errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TurnError {
    Io(std::io::Error),
    /// The server answered, and said no.
    Rejected { method: u16, code: u16, reason: String },
    /// Nothing came back within [`MAX_ATTEMPTS`] retransmissions.
    Timeout(u16),
    /// Something arrived that the protocol does not allow.
    Protocol(String),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "turn socket: {err}"),
            Self::Rejected {
                method,
                code,
                reason,
            } => write!(f, "turn method 0x{method:03x} refused: {code} {reason}"),
            Self::Timeout(method) => {
                write!(f, "turn method 0x{method:03x} got no response")
            }
            Self::Protocol(what) => write!(f, "turn protocol: {what}"),
        }
    }
}

impl std::error::Error for TurnError {}

impl From<std::io::Error> for TurnError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

// ── the client ──────────────────────────────────────────────────────────────

/// One authenticated allocation on one TURN server.
pub struct TurnClient {
    socket: Arc<UdpSocket>,
    server: SocketAddr,
    username: String,
    realm: String,
    nonce: Vec<u8>,
    key: [u8; 16],
    /// The relayed transport address the server allocated: what goes in the
    /// answer as `typ relay`.
    relayed: SocketAddr,
    /// This client's server-reflexive address, as the Allocate response saw
    /// it. Only used for the candidate line's `raddr`/`rport`.
    mapped: Option<SocketAddr>,
    lifetime: Duration,
    txid_counter: u64,
}

impl TurnClient {
    /// Runs the two-step Allocate handshake and returns a live allocation.
    pub async fn allocate(
        server: SocketAddr,
        local_ip: IpAddr,
        username: &str,
        password: &str,
    ) -> Result<Self, TurnError> {
        let socket = UdpSocket::bind(SocketAddr::new(local_ip, 0)).await?;
        socket.connect(server).await?;
        let socket = Arc::new(socket);

        let mut client = Self {
            socket,
            server,
            username: username.to_owned(),
            realm: String::new(),
            nonce: Vec::new(),
            key: [0u8; 16],
            relayed: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            mapped: None,
            lifetime: Duration::from_secs(600),
            txid_counter: 0,
        };

        // Step 1: an unauthenticated Allocate, purely to be told the realm and
        // a nonce. A 401 here is the success case.
        let txid = client.next_txid();
        let mut probe = MessageBuilder::new(
            message_type(METHOD_ALLOCATE, CLASS_REQUEST),
            &txid,
        );
        probe.push(ATTR_REQUESTED_TRANSPORT, &TRANSPORT_UDP);
        let raw = client.exchange(&probe.finish(), &txid, METHOD_ALLOCATE).await?;
        let challenge = Message::parse(&raw)
            .ok_or_else(|| TurnError::Protocol("unparseable Allocate challenge".into()))?;
        if challenge.class() != CLASS_ERROR {
            return Err(TurnError::Protocol(
                "the server allocated without asking for credentials; refusing an \
                 unauthenticated allocation"
                    .into(),
            ));
        }
        client.realm = challenge
            .text(ATTR_REALM)
            .ok_or_else(|| TurnError::Protocol("the 401 carried no REALM".into()))?;
        client.nonce = challenge
            .attr(ATTR_NONCE)
            .ok_or_else(|| TurnError::Protocol("the 401 carried no NONCE".into()))?
            .to_vec();
        client.key = long_term_key(username, &client.realm, password);

        // Step 2: the same request, signed.
        let response = client
            .transact(METHOD_ALLOCATE, &[(ATTR_REQUESTED_TRANSPORT, TRANSPORT_UDP.to_vec())])
            .await?;
        let message = Message::parse(&response)
            .ok_or_else(|| TurnError::Protocol("unparseable Allocate response".into()))?;
        client.relayed = message
            .xor_address(ATTR_XOR_RELAYED_ADDRESS)
            .ok_or_else(|| {
                TurnError::Protocol("the Allocate success carried no XOR-RELAYED-ADDRESS".into())
            })?;
        client.mapped = message.xor_address(ATTR_XOR_MAPPED_ADDRESS);
        if let Some(seconds) = message.lifetime() {
            client.lifetime = Duration::from_secs(u64::from(seconds));
        }
        Ok(client)
    }

    pub fn relayed(&self) -> SocketAddr {
        self.relayed
    }

    pub fn mapped(&self) -> Option<SocketAddr> {
        self.mapped
    }

    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    pub fn server(&self) -> SocketAddr {
        self.server
    }

    /// Transaction ids only have to be unpredictable *within* a client's own
    /// traffic here — the association is over a connected socket to one server
    /// and every reply is authenticated — so a counter mixed with the clock is
    /// enough, and beats adding a random-number dependency.
    fn next_txid(&mut self) -> [u8; 12] {
        self.txid_counter = self.txid_counter.wrapping_add(1);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut txid = [0u8; 12];
        txid[..8].copy_from_slice(&(now ^ self.txid_counter.rotate_left(17)).to_be_bytes());
        txid[8..].copy_from_slice(&(self.txid_counter as u32).to_be_bytes());
        txid
    }

    /// Sends one already-built message and waits for the matching response,
    /// retransmitting on a fixed RTO.
    async fn exchange(
        &self,
        request: &[u8],
        txid: &[u8; 12],
        method: u16,
    ) -> Result<Vec<u8>, TurnError> {
        let mut buf = vec![0u8; 2048];
        for _ in 0..MAX_ATTEMPTS {
            self.socket.send(request).await?;
            let deadline = tokio::time::Instant::now() + RTO;
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(remaining, self.socket.recv(&mut buf)).await {
                    // Retransmit on timeout.
                    Err(_) => break,
                    Ok(Err(err)) => return Err(err.into()),
                    Ok(Ok(len)) => {
                        let Some(message) = Message::parse(&buf[..len]) else {
                            continue; // ChannelData or noise; not our reply.
                        };
                        if &message.txid == txid && message.method() == method {
                            return Ok(buf[..len].to_vec());
                        }
                    }
                }
            }
        }
        Err(TurnError::Timeout(method))
    }

    /// One authenticated request/response, with the stale-nonce dance.
    ///
    /// A 438 (or a second 401) carries a fresh NONCE; the request is rebuilt
    /// and re-signed with it exactly once, which is what RFC 5766 §4 asks for
    /// and what a long-lived allocation runs into every few minutes.
    async fn transact(
        &mut self,
        method: u16,
        attrs: &[(u16, Vec<u8>)],
    ) -> Result<Vec<u8>, TurnError> {
        for attempt in 0..2 {
            let txid = self.next_txid();
            let mut builder =
                MessageBuilder::new(message_type(method, CLASS_REQUEST), &txid);
            for (attr, value) in attrs {
                // XOR-* attribute values are transaction-scoped, so they are
                // re-encoded against this attempt's transaction id rather than
                // carried over from the caller's.
                if matches!(*attr, ATTR_XOR_PEER_ADDRESS) {
                    let addr = decode_xor_address(value, &[0u8; 12])
                        .ok_or_else(|| TurnError::Protocol("bad peer address".into()))?;
                    builder.push(*attr, &encode_xor_address(addr, &txid));
                } else {
                    builder.push(*attr, value);
                }
            }
            builder.push(ATTR_USERNAME, self.username.as_bytes());
            builder.push(ATTR_REALM, self.realm.as_bytes());
            let nonce = self.nonce.clone();
            builder.push(ATTR_NONCE, &nonce);
            let request = builder.finish_with_integrity(&self.key);

            let raw = self.exchange(&request, &txid, method).await?;
            let message = Message::parse(&raw)
                .ok_or_else(|| TurnError::Protocol("unparseable response".into()))?;
            if message.class() == CLASS_SUCCESS {
                return Ok(raw);
            }
            let (code, reason) = message.error_code().unwrap_or((0, String::new()));
            let stale = code == 438 || code == 401;
            if stale && attempt == 0 {
                if let Some(nonce) = message.attr(ATTR_NONCE) {
                    self.nonce = nonce.to_vec();
                }
                if let Some(realm) = message.text(ATTR_REALM) {
                    // A realm change invalidates the derived key.
                    if realm != self.realm {
                        self.realm = realm;
                    }
                }
                continue;
            }
            return Err(TurnError::Rejected {
                method,
                code,
                reason,
            });
        }
        Err(TurnError::Timeout(method))
    }

    /// Extends the allocation's lifetime. Returns the lifetime the server
    /// granted, which may be shorter than the one asked for.
    pub async fn refresh(&mut self, seconds: u32) -> Result<Duration, TurnError> {
        let raw = self
            .transact(
                METHOD_REFRESH,
                &[(ATTR_LIFETIME, seconds.to_be_bytes().to_vec())],
            )
            .await?;
        let granted = Message::parse(&raw)
            .and_then(|m| m.lifetime())
            .unwrap_or(seconds);
        self.lifetime = Duration::from_secs(u64::from(granted));
        Ok(self.lifetime)
    }

    /// Installs (or renews) send permission for each peer. Permissions are
    /// per-IP and last 300 seconds.
    pub async fn create_permission(&mut self, peers: &[SocketAddr]) -> Result<(), TurnError> {
        if peers.is_empty() {
            return Ok(());
        }
        // Encoded against the all-zero transaction id here and re-encoded per
        // attempt inside `transact`; only the address survives the trip.
        let attrs: Vec<(u16, Vec<u8>)> = peers
            .iter()
            .map(|peer| (ATTR_XOR_PEER_ADDRESS, encode_xor_address(*peer, &[0u8; 12])))
            .collect();
        self.transact(METHOD_CREATE_PERMISSION, &attrs).await?;
        Ok(())
    }

    /// Binds a channel number to a peer. Implies a permission, and lasts 600
    /// seconds.
    pub async fn channel_bind(
        &mut self,
        peer: SocketAddr,
        channel: u16,
    ) -> Result<(), TurnError> {
        self.transact(
            METHOD_CHANNEL_BIND,
            &[
                (ATTR_CHANNEL_NUMBER, {
                    let mut value = channel.to_be_bytes().to_vec();
                    value.extend_from_slice(&[0, 0]); // RFFU
                    value
                }),
                (ATTR_XOR_PEER_ADDRESS, encode_xor_address(peer, &[0u8; 12])),
            ],
        )
        .await?;
        Ok(())
    }
}

// ── the relay: shim sockets, framing, and the pump ──────────────────────────

/// What a session gets back for one peer: where to point str0m, and which
/// channel that shim's traffic is framed onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerMap {
    /// The peer's real transport address, as the offer named it.
    pub peer: SocketAddr,
    /// The local address str0m should be told the peer lives at.
    pub shim: SocketAddr,
    pub channel: u16,
}

enum Command {
    Open {
        peers: Vec<SocketAddr>,
        carrier: SocketAddr,
        reply: oneshot::Sender<Result<Vec<PeerMap>, String>>,
    },
    Close,
}

/// A handle on a running allocation and its pump task.
#[derive(Clone)]
pub struct RelayHandle {
    commands: mpsc::Sender<Command>,
    relayed: SocketAddr,
    mapped: Option<SocketAddr>,
    server: SocketAddr,
    lifetime: Duration,
}

impl RelayHandle {
    /// Allocates, then spawns the pump that owns the allocation from here on.
    pub async fn start(
        server: SocketAddr,
        local_ip: IpAddr,
        username: &str,
        password: &str,
    ) -> Result<Self, TurnError> {
        let client = TurnClient::allocate(server, local_ip, username, password).await?;
        let relayed = client.relayed();
        let mapped = client.mapped();
        let lifetime = client.lifetime();
        let (commands, requests) = mpsc::channel(4);
        tokio::spawn(pump(client_into_pump(client), requests));
        Ok(Self {
            commands,
            relayed,
            mapped,
            server,
            lifetime,
        })
    }

    pub fn relayed(&self) -> SocketAddr {
        self.relayed
    }

    pub fn mapped(&self) -> Option<SocketAddr> {
        self.mapped
    }

    pub fn server(&self) -> SocketAddr {
        self.server
    }

    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// Permits and channel-binds every peer in the offer, creates one shim
    /// socket per peer, and points the pump at the carrier.
    ///
    /// Any previous session's shims are torn down first, so a browser refresh
    /// does not leave a stale channel forwarding into a dead socket.
    pub async fn open(
        &self,
        peers: Vec<SocketAddr>,
        carrier: SocketAddr,
    ) -> Result<Vec<PeerMap>, String> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Open {
                peers,
                carrier,
                reply,
            })
            .await
            .map_err(|_| "the turn pump has stopped".to_owned())?;
        answer
            .await
            .map_err(|_| "the turn pump dropped the request".to_owned())?
    }

    /// Tears down the current session's shims, leaving the allocation up.
    pub async fn close(&self) {
        let _ = self.commands.send(Command::Close).await;
    }
}

/// One peer's local stand-in.
struct Link {
    peer: SocketAddr,
    channel: u16,
    shim: Arc<UdpSocket>,
    shim_addr: SocketAddr,
    reader: JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

struct Pump {
    client: TurnClient,
    carrier: Option<SocketAddr>,
    /// This session's live links, by channel.
    links: HashMap<u16, Link>,
    /// Which channel number each peer address owns, for as long as the
    /// *allocation* lives rather than as long as a session does.
    ///
    /// RFC 5766 §11.2 refuses with 400 any ChannelBind that would move a peer
    /// to a different channel number, so handing a refreshed browser a fresh
    /// number is not a clean slate — it is the one thing the server will not
    /// do. Re-binding the same pair is legal and doubles as the refresh.
    assigned: HashMap<SocketAddr, u16>,
    next_channel: u16,
}

fn client_into_pump(client: TurnClient) -> Pump {
    Pump {
        client,
        carrier: None,
        links: HashMap::new(),
        assigned: HashMap::new(),
        next_channel: CHANNEL_MIN,
    }
}

impl Pump {
    /// Creates a shim socket for `peer`, binds it a channel, and starts the
    /// task that reads str0m's packets off it.
    async fn ensure_link(
        &mut self,
        peer: SocketAddr,
        carrier: SocketAddr,
        outbound: &mpsc::Sender<(u16, Vec<u8>)>,
    ) -> Result<u16, String> {
        let channel = match self.assigned.get(&peer) {
            Some(channel) => *channel,
            None => {
                if self.next_channel > CHANNEL_MAX {
                    return Err("ran out of TURN channel numbers".to_owned());
                }
                let channel = self.next_channel;
                self.next_channel += 1;
                self.assigned.insert(peer, channel);
                channel
            }
        };
        if self.links.contains_key(&channel) {
            return Ok(channel);
        }

        self.client
            .create_permission(&[peer])
            .await
            .map_err(|err| err.to_string())?;
        self.client
            .channel_bind(peer, channel)
            .await
            .map_err(|err| err.to_string())?;

        // Bound beside the carrier's own address so the source address str0m
        // sees on inbound packets is the very address its remote candidate
        // names, and no peer-reflexive discovery is needed.
        let shim = UdpSocket::bind(SocketAddr::new(carrier.ip(), 0))
            .await
            .map_err(|err| format!("shim socket for {peer}: {err}"))?;
        let shim_addr = shim
            .local_addr()
            .map_err(|err| format!("shim socket for {peer}: {err}"))?;
        let shim = Arc::new(shim);

        let reader = {
            let shim = Arc::clone(&shim);
            let outbound = outbound.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                loop {
                    match shim.recv_from(&mut buf).await {
                        Ok((len, _from)) => {
                            if outbound.send((channel, buf[..len].to_vec())).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            })
        };

        self.links.insert(
            channel,
            Link {
                peer,
                channel,
                shim,
                shim_addr,
                reader,
            },
        );
        Ok(channel)
    }

    /// Ends the current session: the shim sockets go, their reader tasks are
    /// aborted by `Drop for Link`, and the allocation — permissions, channel
    /// assignments and all — stays exactly as it was.
    fn clear(&mut self) {
        self.links.clear();
        self.carrier = None;
    }

    /// Hands one de-framed payload to the carrier as if it had arrived from
    /// the peer directly.
    async fn to_carrier(&self, channel: u16, payload: &[u8]) {
        let (Some(link), Some(carrier)) = (self.links.get(&channel), self.carrier) else {
            return;
        };
        let _ = link.shim.send_to(payload, carrier).await;
    }
}

/// The one task that owns the allocation for the process's lifetime.
async fn pump(mut state: Pump, mut commands: mpsc::Receiver<Command>) {
    let socket = Arc::clone(&state.client.socket);
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<(u16, Vec<u8>)>(256);
    let mut buf = vec![0u8; 2048];

    // One coarse tick drives every renewal. Allocations last 600 s and
    // permissions 300 s, so a 60 s tick has ample room to be late.
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut since_refresh = Duration::ZERO;
    let mut since_permission = Duration::ZERO;

    loop {
        tokio::select! {
            biased;

            command = commands.recv() => match command {
                None => break,
                Some(Command::Close) => state.clear(),
                Some(Command::Open { peers, carrier, reply }) => {
                    state.clear();
                    state.carrier = Some(carrier);
                    // Permit the server's own address before anything else.
                    // A permission is matched by IP alone (RFC 5766 §9.1
                    // ignores the port), and under a forced-relay policy every
                    // packet the browser sends arrives from *its* allocation on
                    // this same server — including the ones from candidates the
                    // offer never named, which is how a trickling browser gets
                    // through at all. Not fatal if the server declines it: the
                    // per-peer permissions below still carry the named case.
                    let wildcard = SocketAddr::new(state.client.server().ip(), 0);
                    if let Err(err) = state.client.create_permission(&[wildcard]).await {
                        eprintln!("[turn] blanket permission for {} declined: {err}", wildcard.ip());
                    }
                    let mut maps = Vec::new();
                    let mut failure = None;
                    for peer in peers {
                        match state.ensure_link(peer, carrier, &outbound_tx).await {
                            Ok(channel) => maps.push(PeerMap {
                                peer,
                                shim: state.links[&channel].shim_addr,
                                channel,
                            }),
                            Err(err) => {
                                failure = Some(err);
                                break;
                            }
                        }
                    }
                    let answer = match failure {
                        Some(err) => { state.clear(); Err(err) }
                        None => Ok(maps),
                    };
                    let _ = reply.send(answer);
                    since_permission = Duration::ZERO;
                }
            },

            received = socket.recv(&mut buf) => match received {
                Err(err) => {
                    eprintln!("[turn] receive failed: {err}");
                    break;
                }
                Ok(len) => {
                    let datagram = &buf[..len];
                    if let Some((channel, payload)) = parse_channel_data(datagram) {
                        let payload = payload.to_vec();
                        state.to_carrier(channel, &payload).await;
                    } else if let Some(message) = Message::parse(datagram) {
                        let method = message.method();
                        if method == METHOD_DATA && message.class() == CLASS_INDICATION {
                            // A permitted peer the server has no channel for
                            // yet. Take the payload, then learn the peer.
                            let peer = message.xor_address(ATTR_XOR_PEER_ADDRESS);
                            let payload = message.attr(ATTR_DATA).map(<[u8]>::to_vec);
                            if let (Some(peer), Some(payload), Some(carrier)) =
                                (peer, payload, state.carrier)
                            {
                                match state.ensure_link(peer, carrier, &outbound_tx).await {
                                    Ok(channel) => state.to_carrier(channel, &payload).await,
                                    Err(err) => eprintln!(
                                        "[turn] no link for data from {peer}: {err}"
                                    ),
                                }
                            }
                        } else if message.class() == CLASS_ERROR {
                            let (code, reason) = message.error_code().unwrap_or((0, String::new()));
                            eprintln!(
                                "[turn] unsolicited error for method 0x{method:03x}: {code} {reason}"
                            );
                        }
                    }
                }
            },

            Some((channel, payload)) = outbound_rx.recv() => {
                if state.links.contains_key(&channel) {
                    let framed = channel_data(channel, &payload);
                    if let Err(err) = socket.send(&framed).await {
                        eprintln!("[turn] send failed: {err}");
                    }
                }
            },

            _ = ticker.tick() => {
                since_refresh += Duration::from_secs(60);
                since_permission += Duration::from_secs(60);
                let half_life = state.client.lifetime() / 2;
                if since_refresh >= half_life {
                    since_refresh = Duration::ZERO;
                    if let Err(err) = state.client.refresh(600).await {
                        eprintln!("[turn] allocation refresh failed: {err}");
                    }
                }
                // Permissions expire at 300 s and channels at 600 s; renewing
                // both every 240 s keeps a long session alive with one request.
                if since_permission >= Duration::from_secs(240) && state.carrier.is_some() {
                    since_permission = Duration::ZERO;
                    let mut peers: Vec<SocketAddr> =
                        state.links.values().map(|link| link.peer).collect();
                    peers.push(SocketAddr::new(state.client.server().ip(), 0));
                    if let Err(err) = state.client.create_permission(&peers).await {
                        eprintln!("[turn] permission refresh failed: {err}");
                    }
                    let rebind: Vec<(SocketAddr, u16)> = state
                        .links
                        .values()
                        .map(|link| (link.peer, link.channel))
                        .collect();
                    for (peer, channel) in rebind {
                        if let Err(err) = state.client.channel_bind(peer, channel).await {
                            eprintln!("[turn] channel rebind for {peer} failed: {err}");
                        }
                    }
                }
            }
        }
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The class bits are scattered through the message type rather than held
    /// in a field, so the encoder is checked against the constants every TURN
    /// capture shows — including the 0x0113 the live 401 arrives as.
    #[test]
    fn message_types_match_the_published_constants() {
        for (method, class, expected) in [
            (METHOD_BINDING, CLASS_REQUEST, 0x0001),
            (METHOD_BINDING, CLASS_SUCCESS, 0x0101),
            (METHOD_ALLOCATE, CLASS_REQUEST, 0x0003),
            (METHOD_ALLOCATE, CLASS_SUCCESS, 0x0103),
            (METHOD_ALLOCATE, CLASS_ERROR, 0x0113),
            (METHOD_REFRESH, CLASS_REQUEST, 0x0004),
            (METHOD_SEND, CLASS_INDICATION, 0x0016),
            (METHOD_DATA, CLASS_INDICATION, 0x0017),
            (METHOD_CREATE_PERMISSION, CLASS_REQUEST, 0x0008),
            (METHOD_CHANNEL_BIND, CLASS_REQUEST, 0x0009),
        ] {
            let got = message_type(method, class);
            assert_eq!(got, expected, "0x{method:03x}/{class} -> 0x{got:04x}");
            assert_eq!(method_of(got), method);
            assert_eq!(class_of(got), class);
        }
    }

    /// Encode then decode, over the attribute shapes this client actually
    /// builds: padded and unpadded values, repeated attributes, and both
    /// address families.
    #[test]
    fn attributes_round_trip_through_the_builder_and_parser() {
        let txid = *b"0123456789ab";
        let peer_v4: SocketAddr = "192.168.4.28:49185".parse().unwrap();
        let peer_v6: SocketAddr = "[2001:db8::1]:3478".parse().unwrap();

        let mut builder = MessageBuilder::new(
            message_type(METHOD_CREATE_PERMISSION, CLASS_REQUEST),
            &txid,
        );
        builder.push(ATTR_REQUESTED_TRANSPORT, &TRANSPORT_UDP);
        // 5 bytes: forces three bytes of padding, the case a length-unaware
        // parser walks off the end of.
        builder.push(ATTR_USERNAME, b"mere1");
        builder.push(ATTR_REALM, b"mere.probe");
        builder.push(ATTR_XOR_PEER_ADDRESS, &encode_xor_address(peer_v4, &txid));
        builder.push(ATTR_XOR_PEER_ADDRESS, &encode_xor_address(peer_v6, &txid));
        builder.push(ATTR_LIFETIME, &600u32.to_be_bytes());
        let raw = builder.finish();

        assert_eq!(raw.len() % 4, 0, "every message is a whole number of words");
        let parsed = Message::parse(&raw).expect("the builder's output must parse");
        assert_eq!(parsed.msg_type, 0x0008);
        assert_eq!(parsed.method(), METHOD_CREATE_PERMISSION);
        assert_eq!(parsed.class(), CLASS_REQUEST);
        assert_eq!(parsed.txid, txid);
        assert_eq!(parsed.attr(ATTR_USERNAME), Some(&b"mere1"[..]));
        assert_eq!(parsed.attr(ATTR_REALM), Some(&b"mere.probe"[..]));
        assert_eq!(parsed.lifetime(), Some(600));
        let peers: Vec<SocketAddr> = parsed
            .attrs(ATTR_XOR_PEER_ADDRESS)
            .filter_map(|value| decode_xor_address(value, &txid))
            .collect();
        assert_eq!(peers, vec![peer_v4, peer_v6]);

        // The declared length is authoritative: trailing garbage past it is
        // not parsed as an attribute, and a truncated body is refused.
        assert!(Message::parse(&raw[..raw.len() - 4]).is_none());
    }

    /// RFC 5769 §2.4's long-term-credential request, byte for byte.
    ///
    /// This is the acceptance test for the whole authentication path at once:
    /// the `MD5(username:realm:password)` key derivation, the attribute order
    /// and padding, and — the part that is easy to get wrong and impossible to
    /// debug from a bare 401 — the header LENGTH already counting the 24-byte
    /// MESSAGE-INTEGRITY attribute while the HMAC covers only the bytes before
    /// it. If this passes, the sequence in `turn_reference.md` is reproduced
    /// exactly.
    #[test]
    fn rfc5769_long_term_credential_request() {
        // "<U+30DE><U+30C8><U+30EA><U+30C3><U+30AF><U+30B9>", 18 bytes.
        let username = "\u{30DE}\u{30C8}\u{30EA}\u{30C3}\u{30AF}\u{30B9}";
        assert_eq!(username.len(), 18);
        let realm = "example.org";
        let nonce = "f//499k954d6OL34oL9FSTvy64sA";
        // The password *after* SASLprep, which is what the key is derived from.
        let password = "TheMatrIX";

        let key = long_term_key(username, realm, password);
        let txid: [u8; 12] = [
            0x78, 0xad, 0x34, 0x33, 0xc6, 0xad, 0x72, 0xc0, 0x29, 0xda, 0x41, 0x2e,
        ];
        let mut builder =
            MessageBuilder::new(message_type(METHOD_BINDING, CLASS_REQUEST), &txid);
        builder.push(ATTR_USERNAME, username.as_bytes());
        builder.push(ATTR_NONCE, nonce.as_bytes());
        builder.push(ATTR_REALM, realm.as_bytes());
        let message = builder.finish_with_integrity(&key);

        assert_eq!(
            hex(&message),
            concat!(
                // header: Binding request, LENGTH 0x0060, cookie, txid
                "0001", "0060", "2112a442", "78ad3433c6ad72c029da412e",
                // USERNAME: 18 bytes of UTF-8 and two bytes of padding
                "0006", "0012", "e3839ee38388e383aae38383e382afe382b9", "0000",
                // NONCE: 28 bytes, a whole number of words already
                "0015", "001c", "662f2f3439396b39353464364f4c33346f4c39465354767936347341",
                // REALM: 11 bytes and one byte of padding
                "0014", "000b", "6578616d706c652e6f7267", "00",
                // MESSAGE-INTEGRITY: the HMAC the RFC publishes
                "0008", "0014", "f67024656dd64a3e02b8e0712e85c9a28ca89666",
            )
        );

        // And the halves the whole rests on, called out so a failure says
        // which one moved.
        let parsed = Message::parse(&message).expect("the vector must parse");
        assert_eq!(
            hex(parsed.attr(ATTR_MESSAGE_INTEGRITY).unwrap()),
            "f67024656dd64a3e02b8e0712e85c9a28ca89666"
        );
        assert_eq!(
            u16::from_be_bytes([message[2], message[3]]) as usize,
            message.len() - HEADER_LEN,
            "the LENGTH must count MESSAGE-INTEGRITY"
        );
    }

    /// RFC 1321's own vectors, because everything above rests on the key
    /// derivation and a wrong MD5 fails as an opaque 401.
    #[test]
    fn md5_matches_rfc1321() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "1234567890123456789012345678901234567890\
                 1234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(hex(&md5(input.as_bytes())), expected, "MD5({input:?})");
        }
        // 119 bytes: the length that needs a second padding block.
        assert_eq!(
            hex(&md5(&[b'x'; 119])),
            hex(&md5(&"x".repeat(119).into_bytes()))
        );
    }

    /// XOR-RELAYED-ADDRESS decoding, against RFC 5769's published vectors.
    ///
    /// The RFC's vectors are XOR-MAPPED-ADDRESS, which is the identical value
    /// encoding — the attribute type is the only difference, and the decoder
    /// is deliberately type-agnostic — so the bytes are fed in under the
    /// XOR-RELAYED-ADDRESS type this client actually reads.
    #[test]
    fn xor_relayed_address_decodes_the_rfc5769_vectors() {
        let txid: [u8; 12] = [
            0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
        ];

        // §2.2: 192.0.2.1 port 32853.
        let v4 = [0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43];
        let mut builder =
            MessageBuilder::new(message_type(METHOD_ALLOCATE, CLASS_SUCCESS), &txid);
        builder.push(ATTR_XOR_RELAYED_ADDRESS, &v4);
        let raw = builder.finish();
        let parsed = Message::parse(&raw).unwrap();
        assert_eq!(
            parsed.xor_address(ATTR_XOR_RELAYED_ADDRESS),
            Some("192.0.2.1:32853".parse().unwrap())
        );

        // §2.3: 2001:db8:1234:5678:11:2233:4455:6677 port 32853.
        let v6 = [
            0x00, 0x02, 0xa1, 0x47, 0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79, 0xbc,
            0x25, 0xf4, 0xb5, 0xbe, 0xd2, 0xb9, 0xd9,
        ];
        assert_eq!(
            decode_xor_address(&v6, &txid),
            Some("[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                .parse()
                .unwrap())
        );

        // Encoding is the same operation, so the vectors must come back out.
        assert_eq!(
            encode_xor_address("192.0.2.1:32853".parse().unwrap(), &txid),
            v4
        );
        assert_eq!(
            encode_xor_address(
                "[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                    .parse()
                    .unwrap(),
                &txid
            ),
            v6
        );

        // The live allocation this probe took on 2026-08-28: coturn's own
        // XOR-RELAYED-ADDRESS for 192.168.4.28:49185, pinned so a decoder
        // regression is caught without a server.
        let live = encode_xor_address("192.168.4.28:49185".parse().unwrap(), &txid);
        assert_eq!(decode_xor_address(&live, &txid).unwrap().port(), 49185);
    }

    /// ChannelData is four bytes and a payload, and must never be mistaken for
    /// a STUN message in either direction.
    #[test]
    fn channel_data_frames_and_never_parses_as_stun() {
        let payload = b"\x16\xfe\xfd dtls-looking bytes";
        let framed = channel_data(0x4001, payload);
        assert_eq!(&framed[..4], &[0x40, 0x01, 0x00, payload.len() as u8]);
        assert_eq!(parse_channel_data(&framed), Some((0x4001, &payload[..])));
        assert!(
            Message::parse(&framed).is_none(),
            "the top two bits of a ChannelData header are never zero"
        );

        // And the converse: a STUN message is not ChannelData.
        let stun = MessageBuilder::new(0x0001, b"txidtxidtxid").finish();
        assert!(parse_channel_data(&stun).is_none());

        // Channel numbers outside 0x4000..=0x7FFF are not ours.
        assert!(parse_channel_data(&[0x80, 0x01, 0x00, 0x00]).is_none());
        assert!(parse_channel_data(&[0x00, 0x01, 0x00, 0x00]).is_none());
        // A header promising more payload than arrived is refused.
        assert!(parse_channel_data(&[0x40, 0x00, 0x00, 0x08, 1, 2, 3]).is_none());
    }

    // ── the pump, against a TURN server small enough to live in a test ──────

    const MOCK_REALM: &str = "mere.probe";
    const MOCK_NONCE: &str = "f//499k954d6OL34oL9FSTvy64sA";

    #[derive(Debug, PartialEq, Eq)]
    enum ServerEvent {
        /// A request arrived carrying MESSAGE-INTEGRITY, and whether it checked
        /// out against the long-term key.
        Signed { method: u16, valid: bool },
        Permitted(Vec<SocketAddr>),
        Bound { channel: u16, peer: SocketAddr },
        /// A ChannelData frame from the client, de-framed.
        Sent { channel: u16, payload: Vec<u8> },
    }

    /// Verifies MESSAGE-INTEGRITY the way a server does, given that this client
    /// always writes it last: everything before the 24-byte MI attribute is the
    /// hashed span, and the header's LENGTH is already the one the sender
    /// hashed under.
    fn integrity_holds(datagram: &[u8], key: &[u8; 16]) -> bool {
        if datagram.len() < HEADER_LEN + MI_TLV_LEN {
            return false;
        }
        let split = datagram.len() - MI_TLV_LEN;
        if u16::from_be_bytes([datagram[split], datagram[split + 1]]) != ATTR_MESSAGE_INTEGRITY {
            return false;
        }
        let mut mac = HmacSha1::new_from_slice(key).unwrap();
        mac.update(&datagram[..split]);
        mac.verify_slice(&datagram[split + 4..]).is_ok()
    }

    /// A TURN server that does exactly what the handshake needs and nothing
    /// else: challenge once, then sign-check everything, hand out one relayed
    /// address, and report what it saw.
    async fn mock_turn_server(
        socket: UdpSocket,
        relayed: SocketAddr,
        key: [u8; 16],
        events: mpsc::Sender<ServerEvent>,
        mut inject: mpsc::Receiver<Vec<u8>>,
    ) {
        let mut buf = vec![0u8; 2048];
        let mut client: Option<SocketAddr> = None;
        loop {
            tokio::select! {
                datagram = inject.recv() => {
                    let (Some(datagram), Some(client)) = (datagram, client) else { return };
                    let _ = socket.send_to(&datagram, client).await;
                }
                received = socket.recv_from(&mut buf) => {
                    let Ok((len, from)) = received else { return };
                    client = Some(from);
                    if let Some((channel, payload)) = parse_channel_data(&buf[..len]) {
                        let payload = payload.to_vec();
                        let _ = events.send(ServerEvent::Sent { channel, payload }).await;
                        continue;
                    }
                    let Some(message) = Message::parse(&buf[..len]) else { continue };
                    let method = message.method();

                    // The 401 challenge: the one request that may arrive
                    // unsigned, and the one that must be answered with a realm
                    // and a nonce.
                    if message.attr(ATTR_MESSAGE_INTEGRITY).is_none() {
                        let mut reply = MessageBuilder::new(
                            message_type(method, CLASS_ERROR),
                            &message.txid,
                        );
                        let mut code = vec![0, 0, 4, 1];
                        code.extend_from_slice(b"Unauthorized");
                        reply.push(ATTR_ERROR_CODE, &code);
                        reply.push(ATTR_REALM, MOCK_REALM.as_bytes());
                        reply.push(ATTR_NONCE, MOCK_NONCE.as_bytes());
                        let _ = socket.send_to(&reply.finish(), from).await;
                        continue;
                    }

                    let valid = integrity_holds(&buf[..len], &key);
                    let _ = events.send(ServerEvent::Signed { method, valid }).await;
                    if !valid {
                        continue;
                    }
                    let mut reply =
                        MessageBuilder::new(message_type(method, CLASS_SUCCESS), &message.txid);
                    match method {
                        METHOD_ALLOCATE => {
                            reply.push(
                                ATTR_XOR_RELAYED_ADDRESS,
                                &encode_xor_address(relayed, &message.txid),
                            );
                            reply.push(ATTR_LIFETIME, &600u32.to_be_bytes());
                        }
                        METHOD_CREATE_PERMISSION => {
                            let peers = message
                                .attrs(ATTR_XOR_PEER_ADDRESS)
                                .filter_map(|value| decode_xor_address(value, &message.txid))
                                .collect();
                            let _ = events.send(ServerEvent::Permitted(peers)).await;
                        }
                        METHOD_CHANNEL_BIND => {
                            let channel = message
                                .attr(ATTR_CHANNEL_NUMBER)
                                .map(|value| u16::from_be_bytes([value[0], value[1]]))
                                .unwrap();
                            let peer = message.xor_address(ATTR_XOR_PEER_ADDRESS).unwrap();
                            let _ = events.send(ServerEvent::Bound { channel, peer }).await;
                        }
                        _ => {}
                    }
                    let _ = socket.send_to(&reply.finish(), from).await;
                }
            }
        }
    }

    async fn within<F: std::future::Future>(what: &str, future: F) -> F::Output {
        match tokio::time::timeout(Duration::from_secs(5), future).await {
            Ok(value) => value,
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }

    /// The whole relay, end to end, without a browser: allocate, permit, bind,
    /// and carry one packet each way between the "carrier" socket and the
    /// "server" socket.
    ///
    /// This is the half the live run cannot show on its own — a real coturn
    /// proves the handshake, but only a peer proves that a ChannelData frame
    /// arriving from the relay reaches the carrier *as if the peer had sent it
    /// directly*, and that a packet the carrier sends to the shim comes back
    /// out framed for the right channel.
    #[tokio::test]
    async fn the_pump_carries_packets_both_ways_over_a_channel() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();
        // Standing in for `webrtc-carrier`'s socket.
        let carrier = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let carrier_addr = carrier.local_addr().unwrap();

        let relayed: SocketAddr = "203.0.113.7:50000".parse().unwrap();
        let peer: SocketAddr = "198.51.100.9:40000".parse().unwrap();
        let (username, password) = ("1700000000:mere", "s3cret");
        let key = long_term_key(username, MOCK_REALM, password);

        let (events_tx, mut events) = mpsc::channel(32);
        let (inject, inject_rx) = mpsc::channel(8);
        tokio::spawn(mock_turn_server(server, relayed, key, events_tx, inject_rx));

        let relay = within(
            "the allocation",
            RelayHandle::start(server_addr, "127.0.0.1".parse().unwrap(), username, password),
        )
        .await
        .expect("the mock server must grant an allocation");
        assert_eq!(relay.relayed(), relayed);
        assert_eq!(relay.lifetime(), Duration::from_secs(600));

        let maps = within("the session", relay.open(vec![peer], carrier_addr))
            .await
            .expect("opening a session must succeed");
        assert_eq!(maps.len(), 1);
        let map = maps[0];
        assert_eq!(map.peer, peer);
        assert_eq!(map.channel, CHANNEL_MIN);
        assert_eq!(map.shim.ip(), carrier_addr.ip(), "the shim binds beside the carrier");

        let mut seen = Vec::new();
        while !matches!(seen.last(), Some(ServerEvent::Bound { .. })) {
            seen.push(within("a server event", events.recv()).await.unwrap());
        }
        // Every authenticated request checked out, the blanket permission for
        // the server's own address went out ahead of the peer's, and the
        // channel was bound to the peer the offer named.
        assert!(
            seen.iter()
                .all(|event| !matches!(event, ServerEvent::Signed { valid: false, .. })),
            "a request failed MESSAGE-INTEGRITY: {seen:?}"
        );
        assert!(seen.contains(&ServerEvent::Permitted(vec![SocketAddr::new(
            server_addr.ip(),
            0
        )])));
        assert!(seen.contains(&ServerEvent::Permitted(vec![peer])));
        assert!(seen.contains(&ServerEvent::Bound {
            channel: CHANNEL_MIN,
            peer
        }));

        // Inbound: the relay delivers a ChannelData frame, and the carrier
        // must see the bare payload arrive from the address str0m knows the
        // peer by.
        inject
            .send(channel_data(map.channel, b"\x16\xfe\xfd from the peer"))
            .await
            .unwrap();
        let mut buf = vec![0u8; 2048];
        let (len, from) = within("the inbound packet", carrier.recv_from(&mut buf))
            .await
            .unwrap();
        assert_eq!(&buf[..len], b"\x16\xfe\xfd from the peer");
        assert_eq!(from, map.shim, "the carrier must see the peer's shim address");

        // Outbound: what the carrier sends to the shim leaves for the server
        // framed onto the peer's channel.
        carrier
            .send_to(b"\x16\xfe\xfd to the peer", map.shim)
            .await
            .unwrap();
        assert_eq!(
            within("the outbound frame", events.recv()).await.unwrap(),
            ServerEvent::Sent {
                channel: map.channel,
                payload: b"\x16\xfe\xfd to the peer".to_vec(),
            }
        );

        // A refreshed page reopens against the same allocation, and must be
        // given the same channel: RFC 5766 §11.2 refuses (400) any ChannelBind
        // that moves a peer to a different number, which is exactly what a
        // per-session counter would ask for.
        let again = within("the second session", relay.open(vec![peer], carrier_addr))
            .await
            .expect("a reopen must succeed");
        assert_eq!(again[0].channel, map.channel);
        assert_ne!(again[0].shim, map.shim, "each session gets a fresh shim");
    }

    /// ERROR-CODE's class/number split is not a plain integer, and 401/438 are
    /// the two the allocation dance turns on.
    #[test]
    fn error_codes_decode_to_their_stated_numbers() {
        for (class, number, expected) in [(4u8, 1u8, 401u16), (4, 38, 438), (5, 0, 500)] {
            let mut value = vec![0, 0, class, number];
            value.extend_from_slice(b"Unauthorized");
            let mut builder = MessageBuilder::new(0x0113, b"txidtxidtxid");
            builder.push(ATTR_ERROR_CODE, &value);
            let raw = builder.finish();
            let parsed = Message::parse(&raw).unwrap();
            assert_eq!(
                parsed.error_code(),
                Some((expected, "Unauthorized".to_owned()))
            );
        }
    }
}
