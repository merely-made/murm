# mere-transport

The peer transport layer for the [mere](https://crates.io/crates/mere) browser.
Package name is `mere-transport`; the lib is `transport`, so consumers write
`use transport::...`.

It wraps [iroh](https://www.iroh.computer) for authenticated, encrypted QUIC
streams between known peers, exposes content-addressed blob storage (BLAKE3 via
`iroh-blobs`), and presents a `Transport` trait the rest of the workspace takes
as a generic parameter rather than a `Box<dyn Transport>`.

Consumers layer their own framing on the byte stream. Each protocol registers
its own ALPN string (`mere/cable/v1`, `mere/coop/v1`, and so on) so several
protocols share one peer connection.

## Public surface

`blobs`, `memory`, `p2panda_transport`, and the three feature modules are
public. `transport`, `accepted`, `peer_id`, `alpn`, and `error` are private and
their contents are re-exported at the crate root, so a caller writes
`transport::PeerID`.

| Source | Contents |
| --- | --- |
| `transport` | The `Transport` trait |
| `accepted` | `AcceptedSession<S>`, `IngressContext`, `IngressInterfaceId`, `TransportKind` |
| `OutboundSession<S>` | Outbound stream with honest initiator authentication and shared-link facts |
| `p2panda_transport` | `P2pandaTransport`, `P2pandaTransportBuilder`, `P2pandaStream`, `KnownPeer`, `sync_overlay_topic`, re-exports `MdnsDiscoveryMode` and `RelayUrl` |
| `memory` | `MemoryTransport::pair(a, b)`, the in-process test fixture |
| `peer_id` | `PeerID` (newtype over `Ed25519PublicKey`; `from_public_key`, `public_key`, `to_bytes`, `from_bytes`) |
| `alpn` | `Alpn`, a hash-friendly protocol-name newtype |
| `blobs` | `BlobStore`, `BlobHash`, `BlobError` |
| `error` | `TransportError` |
| `noise` (feature) | `NoiseStream`, `NoiseListener`, `handshake`, `secure_initiator`, `secure_responder`, their `*_with_facts` variants, `NoiseHandshakeFacts`, `NoiseSessionFacts`, `connect_to`, `accept_from`, `ProofError`, `NOISE_PARAMS`, `NOISE_PROFILE`, `NOISE_PROFILE_VERSION` |
| `notochord` (feature) | `AcceptedSession::session_facts` / `into_session`, `initiator_binding`, `initiator_link_binding` |
| `reticulum_transport` (feature) | `ReticulumTransport`, `ReticulumTransportBuilder`, `ReticulumInterface`, `ReticulumStream` |

Root also re-exports `p2panda_net::gossip::GossipHandle`, plus
`Ed25519PublicKey` and `IdentityProvider` from identity, and the `VERSION` /
`STAGE` consts.

## The `Transport` trait

```rust,ignore
pub trait Transport: Send + Sync {
    type Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static;
    fn local_peer_id(&self) -> PeerID;
    fn connect(&self, peer: PeerID, alpn: Alpn)
        -> impl Future<Output = Result<Self::Stream, TransportError>> + Send;
    fn accept(&self, alpn: Alpn)
        -> impl Future<Output = Result<AcceptedSession<Self::Stream>, TransportError>> + Send;
}
```

`accept` returns an `AcceptedSession`, not a bare stream: the stream plus the
protocol, the ingress context, and `peer: Option<PeerID>`. `peer` is `Some`
only where the carrier itself authenticated it (p2panda, and `MemoryTransport`
by construction); Reticulum reports `None`. A subject named by application
bytes is never placed there. `is_transport_authenticated()` reads that
distinction; `into_stream()` discards the context.

`connect_session` retains outbound binding facts alongside its stream. The
default for an unknown adapter reports no authenticated initiator or shared
link. P2panda and the paired memory fixture report the local identity they
prove to the responder; Reticulum reports no initiator identity and its actual
shared link. `OutboundSession::proof_binding()` (`notochord` feature) produces
the same common carrier context as the responder's acceptance. A local routing
key alone is never evidence that the far end authenticated it.

The Noise `*_with_facts` APIs preserve the existing wire while exposing the
verified role-ordered Ed25519 participants, exact profile/version and fresh
handshake hash. The secure variants also retain the authenticated inner
protocol. With both `noise` and `notochord`,
`NoiseSessionFacts::secure_channel_facts()` supplies the shared admission
vocabulary; the original carrier facts stay separate and unchanged. Noise
alone does not enable the Notochord dependency. Expected-peer checks and
admission remain the service owner's responsibility.

## Bounded service security

The `security` module (`noise` feature) adds exact owner-selected profiles:
`ServiceSecurityPolicy::noise`, `native`, or explicit `public_plaintext`.
There is no opportunistic negotiation or fallback. Noise requires the existing
XX/Ed25519 wire, checks the expected peer when configured, and verifies the
independent inner protocol. Plaintext produces no secure-channel facts or
authenticated peer. Services disclosing bearer invitations or privileged
object references require a confidential profile.

Store one `SetupGate` with the service and call `reserve` before connecting.
`SetupGuard::run` applies the same absolute deadline to connection establishment
and subsequent admission. Its `noise_initiator` / `noise_responder` return a
`PreparedStream`, security facts and the same guard. Keep that guard through
Notochord admission; only `admitted()` lifts setup I/O budgets and returns the
pending permit. `refused()` records admission refusal. Drop/cancellation
returns the permit and poisons both raw I/O and buffered decrypted bytes.
Cancelling a polled `guard.run(...)` also poisons partial framing and returns
capacity immediately, even if the guard and stream remain retained. A retained
guard cannot admit or restart that cancelled session. Timeouts and I/O-budget
refusals likewise release the shared permit at their terminal transition.
Convenience methods on `SetupGate` serve already-established streams; their
deadline begins at that call and therefore does not cover an earlier connect.
Application lifetime, payload, subscriber and slow-consumer limits begin after
admission and remain the application owner's responsibility.

`SetupGuard::native(P2pandaStream)` obtains evidence from that exact backend
stream. Its TLS exporter context binds the bidirectional stream ID, both
role-ordered backend identities, profile/version, the registered service name
and p2panda's actual network-salted TLS ALPN. The returned public binding is a
digest, not raw exporter keying material. A different stream on the same QUIC
connection has a different binding. This mode authenticates the carrier's
identity; an independent application identity requires an appropriate admission
proof or the selected inner Noise profile.

Iroh already encrypts through relays. Reticulum links also encrypt their link
payload; their absent carrier-proven initiator is an authentication distinction,
not a plaintext label. This native evidence adapter currently covers iroh.
A Reticulum-native admission profile needs its own verified link evidence and
qualification; it must not invent an authenticated initiator. Noise remains
optional under each service's declared requirements.

Defaults are a 10-second total deadline, 16 KiB read/write each, a 4 KiB encoded
frame, 24 frames and 512 I/O polls through security plus admission. Limits are
explicit and configurable up to 300 seconds, 1 MiB each direction, 65,535 bytes
per frame, 4,096 frames and 16,384 polls. The shared pending gate clamps to
`MAX_PENDING_SETUPS` (256), including absurd caller values without a semaphore
panic; zero disables admission. Frame prefixes are checked before body
allocation, and setup byte/poll counters are cumulative. Encoded buffer storage
scales with pending setups and bounded frames. Incoming setup scratch follows
the configured frame cap; the shared Noise writer can still allocate one
65,535-byte output frame before its setup adapter refuses an oversized write.
At the hard ceilings this cost is multiplied by at most 256 pending setups;
crypto state and caller-owned
admission allocations remain additional costs. These are software bounds, not
a measured whole-process heap or embedded-memory receipt.

`SetupStats` reports establishment, secure-policy success, application admission,
stage-specific refusal, timeout, cancellation and capacity counts. Public
plaintext does not increment the secured count. Diagnostics contain no bearer
capabilities, plaintext buffers or protocol/identity dumps.

## Key entry points

| Item | Role |
| --- | --- |
| `P2pandaTransport::builder_for(&DerivedKeypair)` | Production builder over a scoped Personae-derived key. `builder(&Ed25519Keypair)` requires `test-identity` outside this crate's tests; `builder_from_seed_ref(&[u8; 32])` accepts borrowed external seed material. |
| `P2pandaTransportBuilder` | `alpns`, `blobs`, `relay_url`, `mdns`, `discovery`, `discovery_config`, `gossip`, then `bind()`. |
| `P2pandaTransport::bind` / `bind_seed` / `bind_with_blobs` | Shorthands for the common builds. `bind_with_blobs` makes the router serve iroh-blobs. |
| `endpoint_addr`, `ticket`, `add_peer`, `add_peer_ticket`, `peer_ticket` | Explicit bootstrap: serialize this node's `EndpointAddr` to a pasteable ticket and dial back from one. |
| `subscribe(topic: [u8; 32]) -> GossipHandle` | Joins a gossip topic; `publish(bytes)` broadcasts, `subscribe()` yields received bytes. |
| `set_topics`, `remove_topic`, `peers_for_topic` | Topic bookkeeping for peers reached without discovery. |
| `sync_parts() -> Option<(Endpoint, Gossip)>` | The parts a LogSync session is built from above this crate. |
| `sync_overlay_topic(sync_topic) -> [u8; 32]` | Derives the overlay topic LogSync joins gossip on. Explicit bootstrap must tag peers with it. |
| `BlobStore::new()` / `BlobStore::open(root)` | In-memory (`iroh_blobs` `MemStore`) or on-disk (`FsStore`) behind the same API; `is_persistent()` reports which. |
| `put_bytes`, `get_bytes`, `read_range`, `has`, `flush`, `shutdown` | Local blob operations. `fetch_from` downloads a blob from a peer over a `P2pandaTransport`. |

`PeerID` is named for peer identity to keep it distinct from `kernel`'s graph
`NodeId`.

## Features

| Feature | Effect |
| --- | --- |
| `reticulum` (off) | Builds `ReticulumTransport` over `retinue`, Mark's Rust Reticulum stack, for the bilateral stream lane on LoRa / packet-radio / serial links. Sync and blob transfer stay iroh-only. Adds `retinue`, `hkdf`, `sha2`. |
| `noise` (off) | An application-layer Noise_XX handshake that composes inside a stream a carrier already opened, and stands alone over TCP where there is no carrier. Adds `snow`, `hkdf`, `sha2`. |
| `notochord` (off) | Converts an `AcceptedSession` into `notochord::SessionFacts`, plus the initiator-side proof bindings. |

The `notochord` integration test declares `required-features = ["notochord"]`,
so a bare `cargo test -p mere-transport` skips it.

## Dependencies

| Crate | Why |
| --- | --- |
| `identity` (`personae`) | Scoped `DerivedKeypair` for production transport construction; `Ed25519PublicKey` behind `PeerID`. External providers can lend raw seed material. |
| `iroh` 1.0.3 (manifest floor), 1.3.0 (current lock) | QUIC. The shared p2panda supplier requires Iroh 1.3.0. |
| `iroh-blobs`, `iroh-tickets` | Content-addressed storage; pasteable `EndpointAddr` tickets. |
| `p2panda-net` | The endpoint authority behind `P2pandaTransport`: discovery, relay and hole-punching, gossip, LogSync. The `mere-p2panda-net` fork preserves Mere's protocol routing and address-book seams. |
| `p2panda-core` | Operation types on the sync path. |
| `tokio` | Stream I/O, the accept plumbing, `endpoint_addr`'s discovery wait. |
| `retinue`, `snow`, `hkdf`, `sha2`, `notochord` | Optional, per the features above. |

## Consumers

- `murm` opens a stream per cabal, carries signed p2panda-core operations over
  it, and reconciles the log over gossip plus LogSync.
- `gemot` uses transport streams and gossip topics for moot-scoped event sync.
- `mesh` rides `P2pandaTransport` and `sync_overlay_topic` for its sync lane.
- `commons-spine` depends on it with `reticulum` enabled.
- Fetch's `iroh-blobs` feature fetches artifacts through `BlobStore` / `BlobHash`
  over a `P2pandaTransport`.

## Status

Pre-1.0 (`STAGE = "pre-alpha"`). The `Transport` trait, `MemoryTransport`,
`P2pandaTransport` (with gossip and LogSync served off the same endpoint), and
both memory- and disk-backed `BlobStore` are in place. The `reticulum`,
`noise`, and `notochord` lanes are present behind their features, default-off.
Production discovery wiring continues to land.

Protocol semantics (the cabal log, folds, tiers) live above this crate.

## License

MPL-2.0 (see LICENSE).
