# Transport consolidation through Murm

**Status:** 2026-11-03: T7 in progress under Mark's instruction to continue
with Stickleback after the Distillery extraction. T1 landed; T2 to T6 open. T6 (admitted
services onto the secure setup gate), added from the 2026-10-31 survey, has
its Murm piece landed: the session lane sets up through the gate.

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
that service reuses the endpoint authority defined here, and T6 moves the
family's other admitted services onto its setup gate.

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
- **Network sessions are Murm's.** T7 moves `JoinedSpace`, `SyncedSpace` and
  the transport address book from Stickleback into `murm-sync`. A host injects
  `(Endpoint, Gossip)` from `P2pandaTransport::sync_parts()`. Eidetic retains
  its store and policy-before-insert processor; domain closures still decide
  admission. This revises the source home of the earlier join ceremony while
  preserving its single implementation and shutdown ordering.

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
`isocosm/session/v25` (`isometry/session/v22` when surveyed). It admits no
peer through Notochord; a session keys peers by a hash of the iroh remote id. That duplicates `Transport::connect`/`accept` on an ALPN
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

### T6. Admitted services set up through the shared gate

The secure-session plan's N2 put setup and admission under one gate,
`transport::security`: a service-owned capacity lease and deadline that run
through admission, an owner-selected mode with no plaintext retry, and native
iroh channel binding. Three services use it: Knot publishing,
`murm-ocapn-transport` and, since 2026-11-01, Murm's session lane. The others
that admit peers on Murm endpoints each build their own setup (Findings,
2026-10-31), so the gate's bounds do not reach them.

- Done when: each service that admits peers on a Murm endpoint sets up through
  `transport::security` with secure admission v2, or this plan records the
  carrier profile it waits on. WebRTC has no `SecurityMode` profile, so
  Graphshell's WebRTC door waits on one.
- Done when: Murm's own `session_lane.rs` is on the gate, since a Murm service
  outside it leaves no example of the path for the rest. Met 2026-11-01:
  [`SessionLane`](../native/crates/murm/src/session_lane.rs) serves and pushes
  through one shared gate, native on p2panda and Noise elsewhere, with secure
  v2 admission under the owner's `SecureIdentityPolicy`; its v1 entry points
  are gone.
- Needs: each service's owner: Graphshell (host and the resident's browser
  door), Mesh, Distillery, the resident inference mesh and Gemot (commons call,
  when its A1 lands). Isonetry follows T2.

### T7. Extract Stickleback's network sessions

Mark, 2026-11-03: "continue with stickleback, then continue scoping mere
extractions/directory structure cleanup". The family boundary map puts
transport/discovery/sync sessions in Murm and validated storage in Eidetic.

Move the address book and joined/synced-space implementation, including
their tests, into `native/crates/sync` (`murm-sync`). Group-key admission and
encryption stay in Stickleback; a receiver capability exposes the existing
admission path without moving signing material into the network task.
Murm's group-key extension owns joining/publishing over that receiver.
Update actual domain/host callers directly, without an old network re-export.
Stickleback's breaking public-path change advances it to 0.2.0.

Done when: Eidetic's normal graph has no p2panda-net/sync dependency; moved
sources have recorded donor hashes; core and receiving tests pass; a real
two-peer caller preserves rejection-before-insert, convergence and shutdown;
actual consumer roots check locked with one storage/network source; commits
and source pins are pushed. Preserve existing protocol ids and stored formats.

Required receipts: before/after module and Cargo-edge inventory, core tests,
address-book tests, live session controls, real Gemot/mesh/Mere receiving
checks, and exact pushed cohorts. Retain at most 100 MB under
`Code/testing/murm/stickleback-extraction-20261103`; use stable repository
targets and no isolated Cargo home. Existing unrelated physical and headed
receipts keep their original scope. The separately authorized extraction-cache
deletion was again rejected by automatic approval review as "blocked by policy".

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
  - 2026-11-03: Mark chose the second, as `mere2panda-*` with library names
    unchanged; the migration is mere's release log plan, R1
    (`mere/design_docs/mere_docs/implementation_strategy/2026-11-03_release_log_plan.md`).
- 2026-10-28: consolidation does not remove lockfiles. Every application root
  keeps its own `Cargo.lock`. The rustls advisory touched 20 locks in 15 repos
  on 2026-10-28. Fewer direct transport rows mean fewer manifest edits on a
  bump, not fewer locks.
- 2026-10-31, survey of origin/main across `Code/repos` for peer admission on
  Murm endpoints. On the shared gate (`transport::security`, secure admission
  v2): Knot publishing (`knot-editor/crates/knot-editor/src/publish_host.rs`,
  `publish_carrier.rs`), `murm-ocapn-transport` and the N3 fixture. Outside it:
  - **Notochord v1 `admit_session`:** Murm's
    [`session_lane.rs`](../native/crates/murm/src/session_lane.rs), Mere's
    `graphshell-host` (`admission.rs`, its WebRTC door included) and
    `mere-resident`'s `door/browser_carrier.rs`.
  - **Murm's bounded accept with Notochord's `admit`:** Mesh's `ingress`.
  - **Own admission:** Distillery's `inference_stage.rs` frames with Notochord
    and admits against its mesh lease; the resident inference mesh
    (`resident_inference/mesh_discovery.rs`) verifies its own signed claims.
  - **Planned:** Gemot's `crates/commons/src/call.rs` names Notochord admission
    (its A1) and calls none yet.
  - **None:** Isonetry (T2).
- 2026-11-01: the session lane is on the gate (murm `4fd77e5d78b`). The
  initiator now issues its hello once the channel exists, since a v2 proof
  signs that channel, and it names the responder key from its invitation:
  over Reticulum that is the responder's Noise key, not the destination
  identity it dials. The lane's tests cover both owner identity policies on
  native p2panda, a scoped endpoint key admitted by bound proof and refused by
  the strict policy. Mere's direct-PHY probe (`crates/probes/murm-direct-phy`,
  mere `c6d1d19f9`) moved with it and checks against Murm main, but nobody has
  rerun it on the radios. Its LoRa setup limits (a 240-second deadline, 4,096
  I/O polls) are estimates until it is. It had stopped resolving before this
  change, because its patch table still sent `muniment` and `mere-proofs` to
  Mere after they moved to eidetic.
- 2026-11-01: over Reticulum the lane runs Noise inside a link Reticulum
  already encrypts. On LoRa that costs airtime: three handshake messages and
  an encrypted protocol frame before the hello. The native Reticulum profile
  deferred in the secure-session plan would remove that layer.

## Progress

- 2026-10-28: plan opened from the family survey. T1 landed: Mere `29e11b3e4` (mesh) and Gemot `2fafd5d` (commons-spine, gemot, mien).
- 2026-10-31: refreshed T2 (Isonetry's ALPN and peer keying); T6 opened from
  the admitted-services survey.
- 2026-11-01: T6's Murm piece landed: the session lane on the gate (murm
  `4fd77e5d78b`), Mere's direct-PHY probe moved with it (mere `c6d1d19f9`).
