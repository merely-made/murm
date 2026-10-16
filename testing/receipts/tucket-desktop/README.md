# Tucket over Tulle desktop checkpoint

This is an incomplete physical qualification. The retained protocol, carriage
bridge, explicit isolated bench authority and desktop controls pass software
checks. The retained Tucket instance now emits the real wire ACK for direct
incoming text. A negative control failed before that repair.

Native and standalone Tulle-to-stock attempts failed. The separate stock
MeshCore pair passed again, including reconnect. COM6 restoration is verified;
COM7's restoration needs a physical USB reconnect and fresh status/profile
readback. A completed firmware transfer is not sufficient evidence of recovery.

[qualification.json](qualification.json) records the source hashes, public
evidence hashes and exact boundaries. Raw local evidence is retained under
`validation/results/tucket-desktop-20261015/`; private identity material is
excluded from this receipt. See [R6](../../../design_docs/2026-10-13_murm_family_move_plan.md#r6--tucket-over-tulle-through-signalman-2026-10-15-in-progress)
for the acceptance conditions and remaining production custody gate.

The guarded physical runner is `testing/run_tucket_desktop.ps1`. It requires
an explicit `bench_only` config, fresh output directory, full public key and
both USB hardware identities. `tucket_bench_identity` creates private material
only through an explicit create-new bench action; desktop startup never
generates it. Each repeated trial must use a same-boot profile baseline, or
capture a fresh one after a firmware reset.
