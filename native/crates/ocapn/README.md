# murm-ocapn

An optional, bounded native Rust implementation of a two-party CapTP subset on
already admitted ordered streams. It owns no endpoint, router, discovery,
Notochord admission policy, persistent capability catalog, or domain authority.

The explicit profile `OcapnSuite31f0b806` targets the independent OCapN Python
test suite at `31f0b806108a55d76a6ede5af7e413818dfb9221`. It uses the legacy
four-field `op:start-session` and `op:deliver`, settling-only boolean `op:listen`,
Ed25519 gcrypt-style key/signature records, and ASCII length/colon frames without
a trailing comma. This is not a claim of conformance to the latest OCapN draft.
Current draft `ebf68742a40a89fff4730886a8788c5fe1bc9b96` changes session crypto
records, delivery/listen modes and operations. Historical draft
`8a2677c7b5eb58086f26e1254029d335e45bcb33` is a useful baseline but its listen
text also differs from the independent implementation's boolean field.

Passables are deliberately bounded: i64 integers, booleans, UTF-8 strings and
symbols, bytes, lists, canonical dictionaries, ordinary records, and opaque
same-session capabilities. Floating point, arbitrary precision integers, sets,
third-party handoffs, sturdyref dialing, crossed hellos, and passing an answer
reference as a normal argument are explicitly unsupported. An answer reference
can be used as a pipeline target before its result arrives.

`attach` accepts separate `AsyncRead` and `AsyncWrite` halves, immutable
host-supplied `SessionAuthority`, limits and an explicit bootstrap registry.
The host must bind the stream's admitted peer to `expected_remote` when that
mapping is available. A valid ephemeral location signature proves control of
that session key; it does not establish a Notochord principal or grant domain
rights. Applications recover their trusted context only through
`CallContext::authority::<T>()` and apply their existing domain checks.

The caller runs the returned `Driver`. The handle queues bounded calls and
notifications, provides promise pipelining and callback capabilities, and
exposes `is_closed`, `closed().await` and an opaque `session_id`. Host futures run
concurrently with transport progress under explicit count and deadline limits.
Dropping imports/answer leases emits cooperative reference collection; table,
frame, nesting, value size, listener, pipeline, job and queue limits fail closed.
EOF, cancellation, explicit close and deadline expiry close the session and
break pending work. No remote reference can be serialized in another session.

The `suite-fixture` feature enables only the loopback test example:

```sh
cargo run -p murm-ocapn --features suite-fixture --example suite_server -- LOCATOR_FILE
```

It writes an ephemeral loopback locator and serves the independent suite's
factory, echo, greeter and promise fixtures. Their fixed swiss values exist only
in this example, not in the production library. The supported acceptance subset
is 15 tests: three ordinary start-session cases, abort, delivery/pipeline,
listen, export GC and answer GC. The two crossed-hello and seven third-party
handoff tests are excluded explicitly rather than counted as qualified.

## Native admission (`native` feature)

The non-default `native` feature adds `murm_ocapn::native` (formerly the separate
`murm-ocapn-transport` package, folded by engine-split D64). It composes the runtime with Murm's
existing endpoint and Notochord secure v2 admission. A host calls
`NativeService::register` on its existing `P2pandaTransport` and accepts through
the returned `NativeListener`. This requires the bounded first-stream/queue
route; do not add the service to the builder's ordinary `alpns` list. Other
registered services keep their routes. The adapter creates no endpoint, router, key store or
discovery loop.

`NativeService` requires native iroh TLS, verifies its stream-specific exporter
binding, and retains one setup lease through application admission. It does
not add an inner Noise session. A failed native profile or admission is a
refusal. Native Reticulum and alternate carrier adapters need their own
qualified binding; Murm's separate Noise profile remains available to those
services.

Ingress has its own configured capacity (at most 64) and first-stream timeout;
the setup deadline includes the actual first-stream/queue wait. Once dequeued,
the shared setup gate limits security and admission work separately. These are
bounded layers on the host endpoint, not an assumption that a later handshake
gate protects an unbounded earlier queue. Route shutdown remains an action of
the transport owner.

The listener reads the host's trusted clock after dequeue, before application
admission. The host still owns current policy/ledger updates and must account
for active admitted sessions atomically when passing Notochord's serving count.
The setup gate is a setup bound, not a replacement for the host's serving
budget or the runtime's per-session table/queue limits.

The explicit `BoundChannelApplicationProof` policy permits a stable Personae
root to use an independently scoped native endpoint key. The checked root
signer signs the complete v2 channel/action binding. This proves consent to
this exact channel, without claiming that the endpoint key derives from the
root. The client checks its trusted expected server endpoint before sending
the proof. Server application-root authentication is a separate concern; this
adapter proves the initiating application root and authenticates the expected
responder endpoint.

The returned channel is ready for the declared CapTP handshake. The host must
retain `RetainedAuthority::from_admitted` in its immutable runtime context and
recheck its current ledger plus the domain owner's current authority before
every privileged invocation or subscription emission. An OCapN ephemeral key
or peer-supplied argument is never that application principal. The optional
Gemot adapter lives in `mere/ports/moot/src/ocapn_coop.rs`.

The service's `connect` grant admits the adapter only. Collection reads and
proposals still require Gemot's current collection capability and membership
checks. Session objects have no automatic durable resurrection or mutation
retry. The canonical design and evidence boundaries are in
`murm/design_docs/2026-10-29_secure_sessions_and_live_interaction_plan.md`.

Sources: [official suite](https://github.com/ocapn/ocapn-test-suite/tree/31f0b806108a55d76a6ede5af7e413818dfb9221),
[current draft](https://github.com/ocapn/ocapn/tree/ebf68742a40a89fff4730886a8788c5fe1bc9b96).
