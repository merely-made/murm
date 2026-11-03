# Announce timebase: remaining work

**Status (2026-11-02): plan.** The [original timebase plan](archive_docs/2026-11-02/2026-08-25_announce_timebase_plan.md) holds phases A–C, the bounded phase D software evidence, decisions and historical receipts. This plan carries their remaining acceptance and reconciliation; current stock version and receiving owners must be checked at execution.

## Phases and done-conditions

1. **Reservation recovery and package.** Add explicit rekey recovery tests and produce a rebuilt immutable firmware package that declares its preserved reservation ranges and guard-aware persistent state. Done when rekey, corruption with valid identity, first upgrade, exhaustion and downgrade have named outcomes and the actual package enforces its declaration.
2. **Physical cuts and on-air proof.** On both exact board profiles cut power before, during and after reservation and prove recovery of a value above every potentially transmitted pre-cut value. On T114, stock RNS must accept the subsequent announce and complete a link. V4 on-air evidence depends on its separate native-node successor; storage-only probing does not prove emission. Record flash cadence, receive blanking, provenance and actual target memory, including allocator overhead.
3. **Remaining observation boundaries.** Natural elapsed route expiry remains unmeasured: P8 loaded expired state rather than waiting for expiry. If pursued, observe expiry in a persistent stock receiver and retain the exact configuration and lifecycle. Equality carve-outs remain deferred pending measurement. P4 retained-blob capacity and P5 delayed LINKREQUEST behavior remain distinct optional probes; P5 belongs to the separate wall-clock decision. P3 already observed stock acceptance of `2^39`; any further upper-bit probe must identify a distinct question, such as a named peer truncation issue. The live-gate flake has its [own lane](2026-08-23_live_gate_flake_lane.md). Update the [wire reference](2026-07-13_rns_wire_format_reference.md) to distinguish historical Beechat observations from current Retinue freshness and foreign cached path replies, and add the missing local-rebroadcast, observed destination-table and 40-bit ecosystem caveats. Use current tested hop semantics: `crates/retinue/src/packet.rs:50` refuses packets arriving at 128 hops; do not copy the superseded inclusive threshold from the original plan.

## Decisions and constraints

The structured announce input keeps entropy and time caller-owned. Receive freshness gates observable announce effects independently of packet-loop dedup. Firmware reserves an inclusive high-water ceiling before use and abandons unused values after reboot; every minted value advances. Corrupt nonblank state fails closed. Outbound reservations do not make receive replay history durable. A real epoch and unrelated wall-clock uses remain separate decisions.

## Findings (2026-11-02)

`crates/radio-hand/src/announce_reservation.rs:1` defines reservation logic independent of board flash; `crates/retinue/src/announce.rs:76,128` supplies typed blobs and generators. The wire reference at `2026-07-13_rns_wire_format_reference.md:755` closes O-20, and `crates/retinue/oracle/README.md:22` records the source-derived-peer and microReticulum comment boundary. The captured P7 request is `crates/retinue/tests/fixtures/path_request.json`; these resolved items are not repeated as new work. Presence of software does not establish target power-cut behavior, a rebuilt package or real-radio acceptance.

## Deferred and follow-on

Equality carve-outs, durable receive replay state, ratchet expiry and link-request freshness require their own measurement or design; do not infer any from this reservation. V4 native-node work is owned by the wall-node successor.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.