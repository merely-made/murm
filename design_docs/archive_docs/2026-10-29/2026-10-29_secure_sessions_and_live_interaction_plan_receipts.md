# Secure-channel facts and optional live interaction qualification

**Date:** 2026-10-29 (America/New_York).

**Scope:** source integration, focused Rust tests and local-network acceptance
for [N1](../../2026-10-29_secure_sessions_and_live_interaction_plan.md#n1-separate-secure-channel-facts-and-complete-at1).
N2/N3, OCapN, physical radios, firmware and headed-browser acceptance remain
open. No release or crates.io publication was performed.

## Source revisions

| Repository | Published implementation |
| --- | --- |
| Dramatis | `051e95a68011072f172b9ec4c0f04d4e5259df5b` |
| Murm | `1f601488f851c4e54672cd439b7c45728255ea65` |
| Knot | `d400d0cff1de44dfb7033a9c6a076958075915ec` |

Murm's committed native lock resolves Personae, Insigne and Notochord to the
Dramatis revision above. Knot's committed lock resolves that same Dramatis
revision and Murm's implementation revision. Final consumer checks used these
Git sources with `--locked`, without local supplier path patches. The registry
package/version sets did not change during either lock intake; Cargo normalized
some existing dependency edges. Temporary CLI path patches were used only for
development and are absent from the committed manifests and final locks.

Knot was implemented in the isolated
`/Users/markik/Code/worktrees/knot-secure-channel-n1-20261029` worktree based on
its then-current `origin/main`. The primary Knot checkout's unrelated local
commit and ongoing desktop/workspace edits were not staged or pushed by this
work.

## Dramatis / Notochord

Run from the Dramatis root:

| Command | Result |
| --- | --- |
| `cargo test --locked -p notochord --features tokio --lib --tests` | 63 passed: 21 unit, 16 legacy handshake, 13 policy matrix, 3 transport and 10 secure-channel tests |
| `cargo test --locked -p notochord --lib --tests` | 55 passed in the default sans-I/O configuration |
| `cargo clippy --locked -p notochord --features tokio --all-targets --no-deps -- -D warnings` | Passed |
| `cargo +1.97.1 check --locked -p notochord --features tokio` | Passed |

The primary agent independently reran the 63-test Tokio suite after review.
The new compatibility test uses a fixed v1 transcript digest and signature
obtained by compiling the pre-N1 source in a temporary worktree, rather than
computing both expectations with the changed implementation. That worktree was
removed after capturing the fixture.

Secure tests cover hash/profile/version/role/outer-protocol/inner-protocol/link
and carrier-key mutation, unchanged local-interface exclusion, distinct
carrier/application identity policy, checked derived signers, unrelated-key
refusal, literal carrier-authentication requirements, malformed local facts,
no v1 fallback and retention of original carrier facts.

Strict Clippy including local dependencies encountered five untouched baseline
Personae diagnostics: redundant slicing in `seal.rs:60` and missing field docs
in `carry/mod.rs:61–64`. The focused Notochord `--no-deps` check passed. Those
unrelated files were not changed.

## Murm / mere-transport

Final checks ran from the Murm root against the published Dramatis source:

| Command | Result |
| --- | --- |
| `cargo test --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum -j 2 --lib noise::tests` | 9 passed |
| `cargo test --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum -j 2 --lib outbound` | 2 passed |
| `cargo test --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum -j 2 --test notochord` | 3 passed |
| `cargo clippy --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum -j 2 --lib --tests --no-deps -- -D warnings` | Passed |
| `cargo fmt --manifest-path native/Cargo.toml --all --check` | Passed |

The Noise tests include old/new API wire compatibility in both roles, fresh
handshake hashes, canonical secure facts, independently bound outer and inner
protocols, tamper refusal, framing and the existing real iroh composition.
Default outbound metadata and the memory backend's acceptance/binding agreement
are separate controls. Noise-only feature testing also passed during development
without enabling Notochord.

`reticulum_tcp_noise_admits_a_secure_subject_on_the_real_shared_link` uses real
Reticulum nodes over local TCP with reliable links. It verifies the actual
stream/shared-link identity, the original absent carrier peer, both roles'
identical secure facts, the expected server Noise identity, owner-granted
subject admission and delivery of application bytes after admission. Its outer
fixture deadline is 45 seconds. This establishes a native local-network path,
not physical radio behavior or embedded resource acceptance.

## Knot publishing

The final unpatched command, run from the isolated Knot worktree, was:

```sh
CARGO_TARGET_DIR=/Users/markik/Code/testing/knot-secure-channel-n1-20261029/target cargo test --locked -j 2 -p knot-editor --features peers --lib publish_
```

**Result:** 13 passed, zero failed/ignored; 154 unrelated tests filtered. This
includes the five new tests for absent carrier authentication with a distinct
routing identity, shared-link mismatch, authenticated carrier/Noise mismatch,
an unexpected publisher receiving no admission hello, and replay of a prior
Noise session's hello. Existing source-disclosure, revocation, capacity, wire
refusal and real P2panda loopback tests also passed.

Knot's link-shaped memory fixture is explicitly not a Reticulum qualification;
the actual local-TCP Reticulum case is the Murm test above. The current
deployment and endpoint-ticket fallback remain iroh-specific. Alternate
publishing carriers still need appropriate routing/ticket adapters.

Touched-file Rustfmt and whitespace checks passed. The whole-workspace format
check reports pre-existing drift outside the changed publishing files. The
final build also reports existing unused-patch warnings and an unused `path`
variable in `directory.rs:53`; those were not changed.

## Review, compatibility and evidence limits

The primary agent reviewed the shared contract and all three implementations.
Each repository lane also received an independent read-only review by another
lane; no actionable findings remained. Added relative documentation paths and
anchors and staged whitespace were checked before publication.

Legacy Notochord carrier-only v1 APIs and the existing Noise wire remain
compatible. Knot publishing now requires secure admission hello v2 and has no
v1 fallback. Its application payload and `mere/knot-publish/v1` ALPN are
unchanged; old admission peers must upgrade together. Earlier physical or UI
receipts retain their original scope and do not qualify the v2 path.

These tests do not establish broader handshake/rate/deadline policy (N2), full
carrier lifecycle acceptance (N3), group encryption, stored-object protection,
browser behavior, physical-device execution or OCapN compatibility.

## N2/N3 checkpoint — 2026-10-30

**Published Murm implementation:** `e57b92273c6de4c2832b90e06565001fcad2f352`.
The fixture is [`testing/secure-sessions-n3`](../../../testing/secure-sessions-n3/README.md).

The shared gate holds capacity and one deadline across carrier establishment,
selected security and Notochord admission. The private native backend evidence
binds the actual stream, identities, service and negotiated TLS ALPN. A review
found cancellation of only a `SetupGuard::run` future could retain an active
lease. The fix arms a cancellation token and moves the permit into shared
terminal state, releasing capacity and poisoning buffered I/O immediately.
An independent reviewer approved the fix and found no further actionable issues.

Final transport checks used committed suppliers and `--locked`:

| Check | Result |
| --- | --- |
| `cargo test --locked --manifest-path native/Cargo.toml -p mere-transport --all-features --lib security::` | 14 passed after the review fix |
| `cargo test --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum --lib noise::` | 14 passed before the isolated lease-cancellation fix; no subsequent wire/parser changes |
| `cargo clippy --locked --manifest-path native/Cargo.toml -p mere-transport --features noise,notochord,reticulum --all-targets -- -D warnings` | Passed after the review fix |
| Default and Noise-only library feature checks | Passed |
| Rustfmt and owned-path whitespace checks | Passed |

The final two-process matrix receipt is
`/Users/markik/Code/testing/murm-secure-sessions-n3-20261030-final/summary.json`.
It reports `overall_passed: true`, 31 expected/passing cases, and the exact
published implementation above. Retained fixture/transport snapshots, source
hashes, binary hash, per-case process IDs, contacts, results and logs accompany it.
The binary SHA-256 is
`8396cdb3f22a6bf4a58ee047e75686039333af7e7ef5c9dae17439f447cf8064`;
the transport source aggregate is
`7f5eb564bc191a1b635541cea2946d2b601fbe629012670c43663d5003076cae`.

The committed Python harness runs separate native server/client processes:

```sh
cargo build --locked --manifest-path testing/secure-sessions-n3/Cargo.toml
python3 testing/secure-sessions-n3/run.py --binary <actual-built-binary> --output <new-output-directory>
```

There are eight cases each for Noise over raw TCP, iroh and Reticulum reliable
links over local TCP, plus seven for native iroh TLS without inner Noise. Cases
cover fresh reconnect, old-hello replay, wrong keys, signed-proof mutation,
below-Noise ciphertext mutation, cancellation, stalled admission/setup and a
consumer that never acknowledges its response. Negative admission controls
reach no handler, positive handler/completion counts are exact, mutators prove
they changed input, and every setup lease returns to zero. Native iroh's
ciphertext-mutation case is explicitly excluded: this stream API is above TLS
and cannot mutate encrypted QUIC packets. A backend unit test separately checks
different streams on one TLS connection have different bindings.

An earlier fixture teardown trial failed because the client dropped its endpoint
before the server completed shutdown/result writing. That failure is retained;
the final harness keeps connection/stream handles alive through server completion.
It does not count a teardown race as successful application completion.

This is native local process/network evidence, including actual Reticulum
links over TCP. It does not qualify native Reticulum without Noise, physical
radio, firmware, browser, remote-host, relay failover, group-key or durable
mutation behavior. Reticulum links already encrypt; their absent authenticated
initiator field is not evidence of plaintext.

Knot's bounded publishing consumer is published at
`b161d368a456a05a7c5bbd237a2f57019763fa48`, with Murm's implementation above in
the committed lock. Another lane advanced Knot main during this work; the
consumer was rebased onto that received supplier cohort before publication.
The final unpatched `cargo test --locked -j 2 -p knot-editor --features peers
--lib publish_` passed 15 tests, zero failed or ignored, from the isolated Knot
worktree with the N1 target directory. The retained log is
`/Users/markik/Code/testing/knot-secure-channel-n1-20261029/n2-rebased-publish-tests.log`.
The two additional tests prove a stalled setup releases the host lease before
source disclosure, and the shared lease remains held through admission then
releases on cancellation. Existing proof, refusal and actual P2panda loopback
controls remain included. Touched-file formatting and whitespace passed;
Cargo's earlier lock intake preserved the registry package/version/source set.
Unrelated primary-checkout work was not included.

## Bounded OCapN runtime and native admission — 2026-10-30

O1 intake selected an owned MPL Rust runtime rather than a supplier import.
Rexa's inspected revision `00b3c029` was unfinished, LGPL-3.0-or-later and
contained machine-local paths; the Tarq demo had no available maintained Rust
runtime source; Endo's `679d9afaa4acd089f578055118b6711a22537f05` implementation
is JavaScript, with Rust crypto/WASM rather than a native Rust actor runtime.
None of those implementations was imported or presented as a qualified supplier.

The immutable independent suite is `ocapn/ocapn-test-suite` revision
`31f0b806108a55d76a6ede5af7e413818dfb9221`. Current draft
`ebf68742a40a89fff4730886a8788c5fe1bc9b96` has incompatible session-crypto,
delivery and listen forms. Historical draft
`8a2677c7b5eb58086f26e1254029d335e45bcb33` also differs in its listen text.
The runtime therefore names the actual supported legacy profile explicitly;
it does not claim current-draft or full OCapN conformance.

The final independent receipt is
`/Users/markik/Code/testing/murm-ocapn-o1-20261030/independent-final/summary.json`.
It records **15/15 original supplier tests passed**, zero skips/errors/failures,
clean supplier source before and after, and unchanged runtime source, native
manifest/lock and binary during execution. It retains exact test IDs, full logs,
source snapshots and dependency hashes. The binary SHA-256 is
`73dff073de89f116c1c685c4c7651b09a1ce37a2ebac811879caebf78bc6146b`.
The reusable runner is [`testing/ocapn-interop`](../../../testing/ocapn-interop/README.md).
The original suite's own setup retry decorator remains unchanged and is
recorded; this runner adds no retries or test substitutions.

The subset covers three session-start cases, one abort, four delivery/pipeline,
three listen, three export-GC and one answer-GC case over the suite's testing-only
plaintext TCP netlayer. Crossed hellos (two tests), third-party handoffs (seven),
later-draft conformance, arbitrary Syrup passables and Tor/Noise/native-iroh
netlayer interoperability are explicitly excluded. The production Murm netlayer
has separate native secure admission below CapTP; the TCP test netlayer is not
a production security profile.

The runtime's own **13 tests** passed and were independently rerun. They cover
codec bounds, callbacks, immutable per-direction authority, promise pipelines
and listeners, cross-session references, exact table/queue limits, unsupported
operations, separate write/response/idle deadlines, cancellation and early
export collection. Review found a concurrent promise-graph check/store race;
a session mutex now serializes that bounded critical section, with a two-thread
regression. An iterative completion drain bounds chained failure processing.
No open findings remained after independent final review.

The native admission helper uses published Dramatis
`6aa0daa4ba957e2197d649e1e87f381261f9087e`. Its additive
`BoundChannelApplicationProof` policy was reviewed independently by two lanes;
**64 Notochord tests** and strict focused Clippy passed. The new mode verifies
the subject's signature over the actual channel/action binding without claiming
the endpoint key derives from that root. Existing strict policies and v1/v2
wire encodings retain their behavior. Expected server endpoint authentication
is not a separate responder application-root proof.

The native helper's **two endpoint tests** pass. Production scoped-key
constructors supply endpoint keys distinct from both application roots. Actual
native admission and CapTP echo run while another ALPN remains usable on those
same endpoints. A separate silent peer never sends its first stream: bounded
ingress reaches one pending reservation, releases it on deadline, and never
enters secure/application setup. Review required and approved typed bounded
registration before acceptance; actual queue/first-stream age reduces the
remaining setup deadline. Serving counts remain the host owner's responsibility.

Root's final commands include:

```sh
cargo test --locked --manifest-path native/Cargo.toml -j 2 -p murm-ocapn -p murm-ocapn-transport --all-features
cargo test --locked --manifest-path native/Cargo.toml -j 2 -p murm-ocapn-transport --test native_endpoint
cargo clippy --locked --manifest-path native/Cargo.toml -j 2 -p murm-ocapn -p murm-ocapn-transport --all-targets --all-features -- -D warnings
cargo +1.97.1 check --locked --manifest-path native/Cargo.toml -j 2 -p murm-ocapn -p murm-ocapn-transport --all-targets --all-features
```

All passed. Logs are in
`/Users/markik/Code/testing/murm-ocapn-o1-20261030/`. The committed native lock
uses Git suppliers without development path patches; its registry
package/version/source set is unchanged. The optional packages are separate
from the ordinary carrier/conversation dependency graph. No crate release or
crates.io publication was performed.
