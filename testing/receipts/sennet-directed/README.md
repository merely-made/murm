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
an old public card before touching hardware. A fresh receive-only trial is the
next wire discriminator.
