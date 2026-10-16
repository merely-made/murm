# Tucket over Tulle desktop checkpoint

The [Djinn-backed native functional trial](custody/native-turnaround/README.md)
passes two tagged texts and both protocol ACKs against stock MeshCore, then
disconnect/reconnect with exact history retained. All six captures are reviewed;
both boards are restored and owned helpers stopped. This is one delivery
connection and one history-only reconnect, with no new sends after reconnect.
The [custody continuation](custody/README.md) records the published scoped
authority path, 300 core test executions and 41 desktop tests. UI polish and
the Sennet stack leg remain open.

The initial isolated seed-file checkpoint below is historical. The retained protocol, carriage
bridge, explicit isolated bench authority and desktop controls pass software
checks. The retained Tucket instance now emits the real wire ACK for direct
incoming text. A negative control failed before that repair.

Earlier native and standalone Tulle-to-stock attempts failed. The separate stock
MeshCore pair passed again, including reconnect. COM6 restoration is verified;
COM7's restoration was pending in that receipt. The owner's USB reconnect has
since recovered it; [fresh readback](COM7-restored-after-reconnect.json) verifies
the saved identity, status, region and channel. Historical receipts are unchanged.

[qualification.json](qualification.json) records the source hashes, public
evidence hashes and exact boundaries. Raw local evidence is retained under
`validation/results/tucket-desktop-20261015/`; private identity material is
excluded from this receipt. See [R6](../../../design_docs/2026-10-13_murm_family_move_plan.md#r6--tucket-over-tulle-through-signalman-2026-10-15-in-progress)
for the acceptance conditions and current checkpoint. This original ledger
remains historical; the successful trial has its own qualification and hashes.

The guarded physical runner is `testing/run_tucket_desktop.ps1`. It requires
one explicit authority configuration, a fresh output directory, full public key
and both USB hardware identities. Further trials use `-DjinnCustody` with the
public device UUID/realm configuration. The historical `bench_only` mode stays
explicit. `tucket_bench_identity` creates private material
only through an explicit create-new bench action; desktop startup never
generates it. Each repeated trial must use a same-boot profile baseline, or
capture a fresh one after a firmware reset.
