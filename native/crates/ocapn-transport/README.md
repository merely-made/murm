# Admitted OCapN channels

`murm-ocapn-transport` composes the optional `murm-ocapn` runtime with Murm's
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
