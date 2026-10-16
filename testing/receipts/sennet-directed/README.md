# Directed Sennet continuation, 2026-10-16

R8 in the [family plan](../../../design_docs/2026-10-13_murm_family_move_plan.md#r8--sennet-directed-text-and-delivery-acks-2026-10-16-in-progress)
is in progress. The [R7 channel pass](../sennet-desktop/native-reconnect/README.md)
remains frozen. No directed/native/ACK success is claimed by these software logs.

Dramatis `2f27f1a` supplies transient zeroizing material from the existing
device/realm child, with 74 tests and selected Clippy passing. Mere `594c5b38`
adds that material to the same sticky release without changing broker policy.
Its six actual Keeper cases pass, including same material on reopen and
wrong-app controls. Evidence is in Mere's
`testing/receipts/sennet-directed-custody/`. The unchanged fixture host's
binary hash remains `25f41c90...`; private runtime roots are excluded.

The receiving cohort uses canonical Mere `594c5b38`, Dramatis `2f27f1a`, Genet
`e37762d7` and Netrender `f4f9c0d8`. All 57 Sennet library tests pass, including
an independently generated Python cryptography 50.0.2 X25519/AES-CCM vector and
authentication/refusal checks. All 66 Signalman and 44 desktop library tests
pass against this cohort; one existing desktop test is ignored. Sennet strict
Clippy and selected capture/exporter strict Clippy pass. The capture probe and
station exporter build. Full directed wire, instance/ACK and native gates
remain pending.

The build inherited an unrelated target environment and the first capture
recipe picked the older exporter from `target/debug`; its card lacked the new
public key. The trial refused before contact import or RF transmission, both
boards restored and packet IDs stayed unused. Rebuilding with explicit
repository target/build paths records the actual compiler-reported executable
paths and hashes in `software/capture-executables.jsonl`. PKI mode now refuses
an old public card before touching hardware.

The [identified receive](receive-observed/README.md) now authenticates and
matches the exact stock PKI text. It fixes the wire nonce placement and four-byte
tail independently of the current 2.8 documentation. A later
[reply/control capture](routing-observed/README.md) records a physically completed
Sennet PKI reply and the stock channel response with port 5, request field 6 and
code zero. The peer-bound packet codec and strict routing profile pass 60 tests;
protocol instance state and native directed/ACK delivery remain pending.

**Retained runtime checkpoint.** The [two-way ACK wire experiment](ack-wire/README.md)
matches stock CLI ACK completion and a separately correlated Sennet reply ACK.
The directed instance now binds one explicit public peer, shares the durable
packet-ID lease with channel/control traffic, starts its response window only
after physical completion, re-ACKs duplicate authenticated texts without duplicate
delivery, and refuses directed channel-text downgrade. Comms journals the pinned
public identities and refuses reopening that history with a replacement key.
Timeout/disconnect preserves observed physical completion without replay.

The receiving cohort passes 64 Sennet, 146 radio-hand, 69 Signalman and 19 runtime
integration tests, the bare runtime check, and strict selected core Clippy.
Desktop passes 45 library tests (one existing ignored), strict library/bin/example
Clippy and current desktop/station-exporter builds. The opt-in directed native
fixture requires actual protocol ACKs and exact reopened history before fresh
messages. [Runtime software receipts](runtime-software/) freeze these gates;
the full native directed/reconnect gate remains pending at this checkpoint.

The qualified Mere worktree's checkout and registration have been removed.
Windows first retained the empty directory because removal ran from inside it;
automatic approval review then blocked empty-directory/ref cleanup with
"blocked by policy". That empty directory and published branch remain. The
primary Mere checkout and other agents' worktrees were preserved.
