# Retinue small node: remaining work

**Status (2026-11-02): plan.** The [original small-node plan](archive_docs/2026-11-02/2026-07-31_retinue_small_plan.md) retains the N0–N6 history, rulings and bounded receipts. This plan carries remaining work without treating historic interim progress as current failure. The later [murmuration direction](2026-08-09_channel_murmuration.md) supersedes boot-selected citizenship as product direction.

## Phases and done-conditions

1. **Power and persistence.** For the exact T114 profile, verify identity and settings survival through actual power removal and restoration, including atomic recovery and deterministic corruption handling. Measure idle, receive and transmit current with named instrumentation. Reset, reflash and host models do not establish these claims.
2. **Radio scheduling and coexistence.** Evaluate the open cheaper carrier-sense or owned-turn alternative against the measured behavior and retained contention controls. A chosen change is done when its profile, fairness, bounded deferral, failures and counted RF A/B are evidenced. Carry foreign-protocol adapters to their listener and [hardware-family](2026-09-07_radio_hand_hardware_family_plan.md) owners; Retinue-over-Meshtastic remains a separate V4 measurement.
3. **Recovery and RNode follow-ons.** Investigate the observed intermittent stock RNS startup configuration mismatch if reproducible, with a minimally intrusive raw serial witness; do not carry forward the old unhandled `0x0a` claim: current baseline names it `LEAVE` and parses it; check actual current client behavior separately. V4 channel and CRC support were subsequently receipted in the original. For unsolicited battery/channel/PHY telemetry and the unused SX127x CRC path, check current source and actual consumer needs before deciding any change. The old unexplained T114 byte-path observation is not fully explained merely by the parser now naming `LEAVE`; trace it only if it recurs. Done when each is resolved with source and scoped evidence or retained with an explicit owner.
4. **Client interoperability.** Recheck the historical MeshChatX unknown-sender path with a current stock client; its missing delivery announce left that exact path unresolved. Assess whether our sender should identify on delivery links and retain a retry-storm test on the constrained bearer as a separate candidate. Do not relax sender verification or assume the client now announces. `outrider` owns the message-authentication behavior.
5. **Supervision.** Assess per-turn deadlines for a stuck future that still yields, V4 supervision parity and whether crash residue needs durable storage across full power loss. Done when the chosen behavior and fault/recovery boundary are tested on its actual target; do not claim that a breathing executor proves task progress.

## Decisions and constraints

Use shared bounded core rather than a protocol fork; device identity is separate from application identities. Board/executive owns radio interruption and durable writes; the current coordination and quiet-window contracts supersede the old boot-channel story. Count RF runs, record failures and use a same-session control when a change can affect radio behavior. BLE is a later interface, not node ownership.

## Findings (2026-11-02)

`crates/radio-hand/src/rnode.rs:55,192` names and parses `0x0a` as `LEAVE`; `crates/tulle/src/pacing.rs:69` implements a link retry floor and `crates/radio-hand/src/channel/node.rs:62` defines a bounded node channel. Their existence neither proves the meter or power-cut gates nor settles current hardware parity.

## Findings (2026-11-18): transport routing is written twice

Read at `704a04f`, paths under `crates/retinue/src/`. Structural decision 1 of
the original plan ("both exercise one algorithm") holds for links, channels,
resources, the address book, `announce_freshness`, `iface_mode` and the
top-level `rebroadcast` engine. It does not hold for transport routing. `Node`
and `Endpoint` each carry their own route table, reverse table, link bridges,
type-2 forwarding, announce relay gate and packet filter.

- **Size.** About 830 to 920 node lines correspond to about 970 endpoint lines.
  Roughly 900 more endpoint lines have no node counterpart: path discovery
  (`endpoint/discovery.rs`), held announces, gravity, routing selectors, the
  link-MTU clamp and tunnels. Node-only code is about 40 lines.
- **Same struct twice.** `LinkBridge` in `node/tables.rs:34` and
  `endpoint/transit.rs:39` has the same fields, comments and `lapsed` rule. They
  differ in the time type (`u64` against `Instant`) and the container (a bounded
  array of 16 against a `HashMap` of 4096). `ReverseEntry` follows suit.
- **Tests.** About 25 behaviours are tested once per driver, in the mirrored
  `node/tests/` and `endpoint/tests/` files; several test names are identical.
  The whole Rust suite is 774 tests in 34 seconds, so the cost is upkeep.

Behavioural differences for the same input. "Read" means both sides were read
at the lines given; "reported" means a reviewer pass found it and it was not
rechecked. Which side matches stock RNS is not established for any of them.

| # | Difference | Status |
|---|---|---|
| 1 | A relayed announce leaves `Node` only on the interface it arrived on (`node/transit.rs:338`, `node/rebroadcast.rs:107`). `Endpoint` sends it on every permitted interface (`endpoint/rebroadcast.rs:263`). | read |
| 2 | With no live route at release, `Node` falls back to the ingress interface; `Endpoint` drops the relay. | reported |
| 3 | `Node` runs bridged link traffic through the transit filter (`node/transit.rs:152`). `Endpoint`'s bridged path has no such check (`endpoint/router.rs:133`). | read, `forward_on` not read |
| 4 | `Endpoint` refreshes a bridge's `seen` at admission, before policy, hop and egress checks (`endpoint/router.rs:115`). `Node` refreshes after a successful send. | reported |
| 5 | `Endpoint` records a packet's hash before the route lookup (`endpoint/transit.rs:189`), so an unroutable copy is remembered and not counted. `Node` looks the route up first and counts it (`node/transit.rs:197`). | read |
| 6 | `Endpoint` applies the hop ceiling to reverse proofs and rewrites the header to type 1. `Node` only increments hops. | reported |
| 7 | `Node` checks the reverse table before bridge and type-2 handling; `Endpoint` checks it after. | reported |
| 8 | `Endpoint` records the reverse entry and touches the path before the send; `Node` after a successful one. | reported |
| 9 | Packet filter: `Node` keeps 32 hashes per generation, turning over every 60 s, shared by transit and relay. `Endpoint` keeps 32,768 per generation with no ageing, and a separate 4096 for announces. | reported |
| 10 | On `Endpoint` a gravity-repointed announce copy bypasses the filter, and held announces near the hop ceiling are refused. `Node` has neither feature. | reported |
| 11 | **Corrected 2026-11-18:** both can forward a request with fewer than 64 key bytes without a bridge. Node's clamp accepts it unchanged, and failed id derivation only skips insertion (`node/transit.rs:221`, `node/transit.rs:236`, `link/wire.rs:175`; `endpoint/transit.rs:208`). | read; first-pass suspicion wrong |
| 12 | **Corrected 2026-11-18:** both refuse oversized output. Node checks its logical MTU in transit; Endpoint checks the final encoded frame plus access-code overhead in `endpoint/interface.rs:225`, reached through `endpoint/shared.rs:459`. The limits differ by carrier, not by a missing Endpoint check. | read; first-pass implication wrong |
| 13 | **Scope corrected 2026-11-18:** ordinary forget/detach leaves reverse entries and pending rebroadcasts (`node/routes.rs:120`, `endpoint/attach.rs:117`); Endpoint does purge them on shared-instance loss (`endpoint/local.rs:162`). | read; ordinary-detach report confirmed, shared-instance exception added |

The reviewer pass did not read `endpoint/announces/held.rs`,
`discovery/rates.rs`, `iface_policy.rs`, `mtu.rs`, `tunnel.rs`,
`inbound_queue.rs` or most of `shared.rs`.

What a shared implementation would take as parameters: table capacity, `now`
passed in, an output sink (`Actions` against the endpoint's queues), and the
per-interface policy. `crate::rebroadcast` already has this shape: a clock-free
struct that takes `now` and returns a decision, with the driver owning locks,
containers and sends. rns-rs is organised the same way, with one `no_std`
transport engine under two drivers (its `docs/engines.md`; docs only, no source
read). The work is planned in the
[shared transport core plan](2026-11-18_shared_transport_core_plan.md).

## Deferred and follow-on

The four local TFT pages still need physical screen judgement by the owner; `host=fresh` is a locally fed label candidate when that surface is touched. Verify national region-table values before shipping into each region; a lawful EU bench must separately exercise duty refusal. A region requiring listen-before-talk needs an explicit fail-closed CAD-fault policy; dwell limits require implementation before a profile that needs them is admitted. The old plan records unsolicited RNode frames, unused SX127x, carrier-sense tradeoffs and watchdog persistence as distinct matters. Reconcile these against current source and owners before dropping any of them; do not adopt historic interim statements as present status without verification.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
