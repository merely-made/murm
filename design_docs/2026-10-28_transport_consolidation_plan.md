# Transport consolidation through Murm

**Status:** 2026-10-28: in progress. T1 landed; T2 to T5 open.

**For:** Mark, 2026-10-28: "we should consolidate transport concerns through
murm and not sidestep around it." This plan says what Murm owns at the family
level, lists where family code reaches iroh or p2panda-net without Murm, and
records what consolidation does and does not cut.

Not here: the replication runtime, retention and the Murm family move, which are
mere `design_docs/mere_docs/implementation_strategy/2026-10-28_murm_runtime_plan.md`;
wire compatibility, Reticulum and I2P, which keep their own plans here; which Mere
packages belong in other repositories, which is mere
`design_docs/mere_docs/technical_architecture/2026-10-28_source_homes_map.md`.
Generic Noise/AT1 admission and optional OCapN live interaction are in the
[secure-session plan](2026-10-29_secure_sessions_and_live_interaction_plan.md);
that service reuses the endpoint authority defined here.

## Boundary

- **Transport is Murm's:** binding endpoints (the iroh `Endpoint`, p2panda-net's
  `Endpoint`, `Gossip` and `LogSync`), ALPN registration, tickets and peer routes,
  relays, blob carriage on the wire, WebRTC, Reticulum and I2P carriers, and
  transport diagnostics. `mere-transport`'s `P2pandaTransport` is the endpoint
  authority; `Transport` gives ALPN-scoped byte streams.
- **p2panda-core is not transport.** `Operation`, `Header`, `Hash`, `SigningKey`,
  `Topic`, the CBOR codec, prune flags and the p2panda-store traits are the
  replication vocabulary that Stickleback and the domain crates speak. Gemot,
  mesh, Knot, Djinn, Distillery and Moot use them in dozens of files and none of
  those files opens a connection. Routing them through Murm would put a data
  model behind a network crate.
- **The join ceremony is Stickleback's.** `JoinedSpace` owns it (Mere's transport
  ownership ruling (b), 2026-07-25). A host takes `(Endpoint, Gossip)` from
  `P2pandaTransport::sync_parts()` and hands them to the domain's join helper,
  which passes them to `JoinedSpace`. Stickleback re-exports both types "so a
  domain crate can offer a join helper without importing a p2panda crate itself"
  (eidetic `crates/stickleback/src/lib.rs`).

## Phases

### T1. Domain crates name the injected handles through Stickleback

Mere's `mere-mesh` and Gemot's `commons-spine`, `gemot` and `mien` declared
`mere-p2panda-net =0.7.5` with its six features only to name `Endpoint` and
`Gossip`. Mesh's manifest said why that hurts: "feature unification means EVERY
consumer has to opt out, not just one."

- Done when: no domain crate declares p2panda-net; the handle types come from
  `stickleback::{Endpoint, Gossip}`; lockfiles lose only those edges and the
  enabled p2panda-net features are unchanged (Stickleback's `net` declares the same
  six).
- Status: landed 2026-10-28.

### T2. Isonetry's VTT session over Murm's `Transport`

isocosm `crates/isonetry/src/iroh_link.rs` (517 lines, the `iroh` feature, which
`isometry-genet` enables) builds its own iroh `Endpoint` and parses its own
`EndpointTicket`s, then runs one bidirectional stream per peer under ALPN
`isometry/session/v22`. That duplicates `Transport::connect`/`accept` on an ALPN
and Murm's `encode_peer_ticket`/`decode_peer_ticket`, which use the same
`EndpointTicket` text. The session keeps a second endpoint implementation with
its own identity, relay and discovery defaults, and Murm's transport diagnostics
(mere murm runtime plan M8) cannot see it. Every iroh bump also has to edit
isonetry's manifest.

- Done when: the `iroh` feature runs on `P2pandaTransport` and `Transport`
  streams; tickets stay `EndpointTicket` strings; any wire change moves the ALPN
  with `PROTOCOL_VERSION`; isonetry declares neither `iroh` nor `iroh-tickets`.
- Needs: Isocosm's owner, since the session protocol is theirs.

### T3. Isonetry campaign sync through `JoinedSpace`

isocosm `crates/isonetry/src/campaign_sync.rs` (`campaign-p2p-net`) drives
p2panda-net's `LogSync` and `SyncHandle` directly instead of joining through
Stickleback. That bypasses the shared policy-before-insert path that every
other domain's live intake goes through.

- Done when: campaign sync joins through `JoinedSpace` (Isocosm calls Stickleback
  `murm-replication`), and isonetry declares neither `p2panda-net` nor
  `p2panda-sync`.
- Needs: Isocosm's owner.

### T4. Distillery's remote backend over a Murm endpoint

Mere's Distillery `remote` feature enables `burn-remote`'s `iroh` transport,
which binds its own endpoint. Mere already carries a patched `burn-remote`
(`support/patches/burn-remote`, iroh in 20 files).

- Done when: the remote backend takes a connection or endpoint from the host, and
  Distillery's manifest names no `iroh`.
- Needs: a seam in `burn-remote`. A seam that upstream Burn would accept is worth
  more than a larger local patch.

### T5. One iroh relay fixture

Two fixtures pin iroh exactly, each with its own lockfile: Murm's
`testing/iroh-control` and Mere's `support/graphshell-coop/relay_server`, which
serves the Graphshell co-op harness. On 2026-10-28 the relay's lock needed a hand
fix for noq-udp that no other lock needed. The co-op harness hashes the relay's
files into its frozen inputs (`freeze.py`), so moving the relay changes those
inputs.

- Done when: one relay fixture lives beside Murm's iroh pin and the co-op harness
  runs it by a stable path.
- Needs: the Graphshell co-op harness owner.

## Findings

- 2026-10-28, survey of origin/main in Mere, Eidetic, Gemot, Isocosm, Knot,
  Turnstone, Woodshed and Cleromancy. These sites reach transport:
  - **Murm itself:** `native/crates/transport` binds the endpoints.
  - **Stickleback's `net` feature:** `joined_space.rs`, `synced_space.rs`,
    `group_lane.rs` and `address_book.rs` run LogSync and gossip over injected
    handles. This is by ruling (b).
  - **Domain crates naming the handles:** mesh, commons-spine, gemot and mien
    named the injected handles from p2panda-net directly. T1 moved them to
    Stickleback's re-export.
  - **Already through Murm:** `graphshell-host` and Woodshed use `murm-webrtc`.
    Mere's `fetch` (with its `iroh-blobs` feature) and `knot-editor` go through
    `transport`.
  - **Bypassing Murm:** Isonetry's `iroh_link.rs` and `campaign_sync.rs` (T2,
    T3), Distillery's `burn-remote` (T4) and the co-op relay fixture (T5).
  - **Direct on purpose:** Djinn's `tests/knot_residue.rs` uses iroh and
    p2panda-net as the baselines it measures Knot's residue against.
- 2026-10-28: after T1, the normal dependencies that name p2panda-net are Murm's
  `transport`, Stickleback (`net`) and Isonetry. Djinn names it only as a
  dev-dependency.
  - Moving Stickleback's network half into Murm would leave one, at the cost of
    changing `JoinedSpace`'s public path for every domain and host.
  - Nothing in this survey argues against ruling (b).
- 2026-10-28: consolidation does not remove the `[patch]` tables.
  - **Why every root repeats them:** 22 roots repeat the 6–8-row `mark-ik/p2panda`
    table. Cargo applies `[patch]` only from the root being built, never through a
    git dependency. So every root that resolves a p2panda crate repeats the
    table, whether it names p2panda or only reaches it through Murm.
  - **Why the table exists:** the fork tag
    `mere-p2panda-net-0.7.5-upstream-20261013` is built on upstream main 311
    commits past the v0.7.1 release. It moves p2panda-core and p2panda-encryption to dalek 3 and changes
    p2panda-net: a store-agnostic address book, a shutdown drain, admitted
    external ALPN handlers and gossip watcher refresh. Consumers declare
    crates.io `0.7.x` so they can still be published.
  - **What would remove the rows:**
    - an upstream release that carries dalek 3 and those p2panda-net changes; or
    - the fork crates published under fork names, the posture Bole, Vano and
      Didgeridoo already have, which would change p2panda from a personal
      patch-carrying fork into a family fork.
- 2026-10-28: consolidation does not remove lockfiles. Every application root
  keeps its own `Cargo.lock`. The rustls advisory touched 20 locks in 15 repos
  on 2026-10-28. Fewer direct transport rows mean fewer manifest edits on a
  bump, not fewer locks.

## Progress

- 2026-10-28: plan opened from the family survey. T1 landed: Mere `29e11b3e4` (mesh) and Gemot `2fafd5d` (commons-spine, gemot, mien).
