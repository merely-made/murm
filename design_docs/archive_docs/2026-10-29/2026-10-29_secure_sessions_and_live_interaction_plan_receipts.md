# N1 secure-channel facts and AT1 integration qualification

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
