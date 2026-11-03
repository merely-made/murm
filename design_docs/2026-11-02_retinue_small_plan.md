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

## Deferred and follow-on

The four local TFT pages still need physical screen judgement by the owner; `host=fresh` is a locally fed label candidate when that surface is touched. Verify national region-table values before shipping into each region; a lawful EU bench must separately exercise duty refusal. A region requiring listen-before-talk needs an explicit fail-closed CAD-fault policy; dwell limits require implementation before a profile that needs them is admitted. The old plan records unsolicited RNode frames, unused SX127x, carrier-sense tradeoffs and watchdog persistence as distinct matters. Reconcile these against current source and owners before dropping any of them; do not adopt historic interim statements as present status without verification.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.