# mere-webrtc-carrier

The WebRTC carrier for the [mere](https://crates.io/crates/mere) workspace: a
Wasm-clean default core, plus two off-by-default runtime adapters. Package
name is `mere-webrtc-carrier`; the lib is `webrtc_carrier`, so consumers write
`use webrtc_carrier::...`.

The default core is the part a browser and a native host must agree on
byte-for-byte before either has opened a socket: bounded data-channel frames,
invitation identifiers, role-tagged DTLS fingerprints, the link-challenge
transcript that derives Notochord's 16-byte `shared_link`, and the two-mark
`Backpressure` policy both adapters obey. On top of that core sit two
off-by-default features: `native`, a str0m 0.23.1 answerer (Sans-IO — this
crate drives the poll loop, not str0m — over tokio); and `browser`, a
`web-sys` `RtcPeerConnection` initiator (the browser *is* the WebRTC
implementation here — no Rust WebRTC stack ships in Wasm). See
[Features](#features) below.

This crate carries C0 of the
browser WebRTC carrier plan (`mere/design_docs/mere_docs/implementation_strategy/2026-08-25_browser_webrtc_carrier_plan.md`)
— the shared core — plus the two C1 runtime adapters built on it; the
feasibility work behind it is the
browser-to-native WebRTC probe (`mere/design_docs/mere_docs/research/2026-08-25_browser_native_webrtc_carrier_probe.md`).

## The core computes; it does not generate

Every nonce and every fingerprint arrives as an argument. There is no RNG here,
no clock, no socket, and no `wasm-bindgen`: freshness is the runtime adapter's
obligation, stated in its signature rather than hidden behind a core that
quietly reaches for `getrandom`. That is also what keeps the default build
compiling for `wasm32-unknown-unknown` with no feature coaxing, and it is why
this crate depends on neither `mere-transport` nor `notochord` — the admission
layers above it are reached by handing them a `[u8; 16]`, not by linking them
in.

## Public surface

Every module is private and its contents are re-exported at the crate root, so
a caller writes `webrtc_carrier::LinkChallenge`.

| Source | Contents |
| --- | --- |
| `frame` | `FrameHeader`, `encode_frame`, `encode_frame_into`, `decode_frame`, `FRAME_HEADER_BYTES`, `MAX_FRAME_PAYLOAD_BYTES`, `MAX_FRAME_BYTES` |
| `invite` | `InviteId`, `INVITE_ID_BYTES` |
| `fingerprint` | `DtlsFingerprint`, `FingerprintRole`, `DTLS_FINGERPRINT_BYTES`, `CANONICAL_FINGERPRINT_BYTES`, `FINGERPRINT_ALGORITHM` |
| `challenge` | `LinkChallenge`, `NONCE_BYTES`, `SHARED_LINK_BYTES`, `MAX_TRANSCRIPT_FIELD_BYTES`, `LINK_CHALLENGE_VERSION`, `SHARED_LINK_DOMAIN` |
| `backpressure` | `Backpressure`, `MAX_QUEUED_BYTES`, `DEFAULT_HIGH_WATER_BYTES`, `DEFAULT_LOW_WATER_BYTES` |
| `error` | `FrameError`, `InviteIdError`, `FingerprintError`, `ChallengeError`, `BackpressureError` |

Root also carries the `VERSION` / `STAGE` consts. The `native` and `browser`
features add further gated exports — see [Features](#features) below.

## Bounded frames

Four-byte big-endian payload length, then that many bytes. The ceiling is
`MAX_FRAME_PAYLOAD_BYTES` = 65,532 (64 KiB minus the four-byte header), so
`MAX_FRAME_BYTES` — the whole frame, header included — is exactly 64 KiB
(65,536 bytes). That is deliberate: 64 KiB is SCTP's own default
`max-message-size` and str0m's `DEFAULT_REMOTE_MAX_MESSAGE_SIZE`
(`sctp/mod.rs:36`). A payload ceiling of a full 65,536 bytes would make the
*whole frame* 65,540 bytes — four bytes over that default, rejected by any
peer that never negotiated a larger message size. Browsers are not the
constraint here; every one tested advertises 262,144 (256 KiB) in SDP, which
is why the old ceiling survived as long as it did. This one is sized for a
conforming peer, not a browser.

The ordering is the point. `FrameHeader::decode` and `decode_frame` reject an
oversize frame from the **length prefix alone**: no buffer is reserved, no
payload is copied, and nothing is deserialized before the declared length is
compared against the ceiling. Passing four bytes that declare four gigabytes
returns `FrameError::Oversize`, not `Incomplete`, and `decode_frame` borrows
its payload rather than allocating one. Backpressure — high- and low-water
marks over `bufferedAmount` — is a separate policy layered on top; see
[Backpressure](#backpressure) below.

## Backpressure

`Backpressure` is two marks, not one: a high-water mark a sender stops at and
a strictly lower low-water mark it must drain back to before resuming. A
single threshold oscillates — the queue sits at the mark and every completed
write re-crosses it — which is why this is a type with a constructor that
refuses equal or inverted marks, not a pair of loose constants. The defaults
derive from `MAX_FRAME_BYTES`: high water at eight frames, low water at two,
under a `MAX_QUEUED_BYTES` ceiling of sixteen.

The policy lives in the shared core because both adapters must agree on the
numbers, but each reads a different gauge:

- the browser reads `RTCDataChannel.bufferedAmount` directly;
- the native side reads `queued_outbound + sctp_buffered` — the hand-off
  queue this crate holds for the driver, **plus** the bytes str0m's own SCTP
  association has accepted and not yet flushed. Counting only one of the two
  is wrong, not just imprecise: str0m stops accepting writes at 128 KiB
  across all streams, below this crate's default high-water mark, so a gauge
  that ignored the hand-off queue could never reach the configured mark at
  all.

On the native side, `CarrierConfig::sctp_window_bytes` (default 16 KiB) adds
a second, narrower ceiling: how much this crate hands to SCTP at once, not
how much a sender may queue. It exists because str0m 0.23.1's `do_poll_output`
calls itself once per SCTP packet drained, so one `poll_output` call recurses
as deep as the outstanding window — and with a peer that stops acknowledging,
nothing unwinds until the burst is drained. `sctp_window_bytes` bounds that
recursion depth, and therefore the driver task's stack; the measured
stack-depth-per-window table that set the 16 KiB default lives on the field's
doc comment in `src/native/session.rs`. Raise it only with the driver's
thread stack in hand.

## Role-tagged fingerprints

A fingerprint is canonicalized as a one-byte role tag followed by the 32 raw
digest bytes. `Client` is `0x01` (the browser: it offers, initiates, and holds
the invitation); `Server` is `0x02` (the native host: it answers and, from C2,
signs the transcript). Neither tag is zero, so a zeroed buffer is not a valid
canonical fingerprint of either role.

The two halves are therefore not interchangeable. Swapping them yields a
different transcript and a different link, which is what stops a signaling
intermediary from terminating two DTLS sessions and relaying one end's binding
at the other. `LinkChallenge::new` refuses a fingerprint presented in the wrong
role slot rather than retagging it.

The text form is parsed strictly or not at all. `parse_sdp_hex` implements
RFC 8122's `2UHEX *(":" 2UHEX)` exactly: 32 groups, two hex digits each,
uppercase only, single colons, no whitespace. Lowercase is a reject.
`parse_sdp_attribute` additionally requires the `sha-256` token — an SHA-1
fingerprint is not a weaker binding this carrier accepts with a warning, it is
one it does not accept. Nothing truncates, pads, or coerces; a fingerprint that
nearly parses is worth less than one that does not.

## The link challenge

`LinkChallenge` binds exactly what plan §5 names:

1. the protocol,
2. the data-channel label,
3. the invite id,
4. a fresh client nonce,
5. a fresh server nonce,
6. the role-tagged SHA-256 DTLS fingerprints of both ends.

`encode()` writes them as eight length-prefixed fields, opened by
`LINK_CHALLENGE_VERSION`. Fixed-width fields are prefixed too, which is
redundant on its own terms and is exactly what makes the encoding injective: no
regrouping of the same bytes across two fields can produce the same transcript.

`shared_link()` is

```text
blake3( SHARED_LINK_DOMAIN || len(transcript) || transcript )[..16]
```

with `SHARED_LINK_DOMAIN = "mere.webrtc-carrier/shared-link/v1"` and all
lengths eight-byte little-endian. Two names, both frozen: the version tag lives
*inside* the transcript because those bytes get used twice — the host signs
them in C2, both ends hash them here — so a signature over them cannot be
reinterpreted as a signature over a later transcript shape; the domain string
separates the derivation so no other use of the same bytes can collide with a
link id.

That `[u8; 16]` is what `IngressContext::webrtc(shared_link)` carries into
Notochord's `SessionFacts`, and what `initiator_link_binding` proves possession
of — the same grammar Reticulum already uses for a carrier that cannot
authenticate peers.

## Vectors

Browser and native share wire types and test vectors, not one runtime.
`tests/vectors.rs` holds the canonical vector as a fixed input and a fixed
*literal* expected output, recomputed by nothing:

```text
protocol       "mere/graphshell/v1"
channel label  "mere-graphshell"
invite id      000102030405060708090a0b0c0d0e0f
client nonce   11 * 32
server nonce   22 * 32
client dtls    AA * 32, role tag 0x01
server dtls    BB * 32, role tag 0x02
-> transcript  280 bytes
-> shared_link 4692c88e70470f7e4b7ba46b7fce78b2
```

Changing the version tag, the domain string, the field order, the prefix width,
or the truncation moves that value. It is a tripwire on wire behaviour: edit it
deliberately and in step with every other implementation, never "to match" a
build that started failing.

The rest of the file is the differential half — fingerprint swap, single-field
change, splitting ambiguity, oversize frame, malformed fingerprint — each
asserting that a substitution an attacker or a bug would attempt lands on a
different link or an error.

Verified on both targets, default core and each feature:

```bash
cargo test -p mere-webrtc-carrier
cargo test -p mere-webrtc-carrier --features native
cargo check -p mere-webrtc-carrier --target wasm32-unknown-unknown
cargo check -p mere-webrtc-carrier --features browser --target wasm32-unknown-unknown
```

## Features

`default = []`. The Wasm-clean core described above is what every consumer
gets without asking, and it alone keeps `cargo check --target
wasm32-unknown-unknown` with no features passing untouched by either feature
below.

- **`native`** — a str0m 0.23.1 answerer, Sans-IO: this crate drives the
  poll/output loop over a tokio UDP socket, str0m owns no socket or thread of
  its own. Built with `default-features = false, features = ["rust-crypto"]`
  — the pure-Rust DTLS backend, so no `aws-lc-sys`, no C toolchain, and it
  cross-compiles wherever `rustc` does. Its dependencies are declared only
  under `cfg(not(target_arch = "wasm32"))`, `dep:`-gated, so turning `native`
  on for a wasm32 target resolves nothing.
- **`browser`** — a `web_sys::RtcPeerConnection` initiator: the browser's own
  WebRTC stack, driven through its JS bindings, not a Rust WebRTC stack
  compiled into Wasm (there isn't one here to compile in). Its dependencies
  are declared only under `cfg(target_arch = "wasm32")`, `dep:`-gated, so
  turning `browser` on for a native target resolves nothing.

Both are off by default and stay that way; each pulls in a socket, a runtime
or a JS bridge that the Wasm-clean default build exists specifically to not
require.

### For consumers shipping `browser` through `wasm-bindgen`

Pin `wasm-bindgen` to the installed `wasm-bindgen-cli` version **exactly**
(currently `=0.2.126`), not a version range. The JS glue `wasm-bindgen`
generates and the schema the CLI expects have to match byte-for-byte; a range
resolves whatever is newest on the next `cargo update`, compiles cleanly, and
then fails at binding generation the moment the resolved crate and the
installed CLI disagree — a failure mode that shows up after the build looked
fine, not during it.

## Dependencies

| Crate | Why |
| --- | --- |
| `blake3` | The one hash, and it hashes exactly one thing: the transcript. (DTLS fingerprints are SHA-256 by RFC 8122, but they arrive as 32 bytes and are never computed here.) Workspace-pinned (`1`). Matches `graphshell::browser_carrier`'s own transcript-link derivation — one link-derivation discipline in the repo, not two. Pure Rust; builds for `wasm32-unknown-unknown` unmodified. |
| `thiserror` | The five error enums. |

No `subtle`: constant-time identifier comparison here is a short fold over two
byte slices, and the workspace does not otherwise carry that crate.

## Status

Pre-1.0 (`STAGE = "pre-alpha"`). C0 (the core and its vectors) and C1 (the
`native` and `browser` runtime adapters) are here. Invitation redemption and
Notochord admission (C2), forced-relay reconnect (C3), and the real browser
Graphshell consumer (C4) are implemented above this crate and recorded in the
carrier plan (`mere/design_docs/mere_docs/implementation_strategy/2026-08-25_browser_webrtc_carrier_plan.md`).
Public rendezvous (C5) remains next. Those Graphshell receipts do not qualify a
browser Murm conversation consumer.

## License

MPL-2.0 (see LICENSE).
