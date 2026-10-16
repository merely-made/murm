# Directed reconnect with automated board restoration, 2026-10-16

At Murm `feab8c3f29b`, the full native fixture passes four exact directed PKI
texts and four correlated protocol ACKs over two connections against identified
stock Meshtastic 2.7.26.54e0d8d. Sennet runs on this computer through Tulle on
COM7; COM6 is the stock peer during the exchange. Exact history is checked
before fresh work after reconnect; all four records survive final disconnect.

The same scoped device/realm identity is released through an isolated real
Djinn Keeper wallet on both connections. Its public node is `9c562939`, with
stock peer `f66afb28`. Both incoming texts authenticate with the explicitly
pinned public peer key. Each native outgoing record has physical completion
and a matching code-zero routing response; the stock peer independently matches
both exact texts and observes ACKs for both returns. The 2.7 channel ACKs remain
unauthenticated correlation facts. Installed personal-daemon integration,
private-channel custody, broader mesh routing and 2.8 ACK proofs remain open.

Text IDs 1/257 and control IDs 2/258 use durable intervals 1–256 and 257–512;
next state is 513. Unused IDs are abandoned on reconnect. This is local
reservation/reopen evidence, not hardware power-cut or global rollback proof.
Helper service totals 15010/3217 ms are not per-message latencies.

All seven captures are visually reviewed: 403 presented frames, zero blank
captures, seven distinct digests and all four final rows visible. The stale
connecting notice and clipped journal input remain cosmetic findings.

The wrapper restores COM6 first to the pinned Tulle image and verifies exact
status/identity/region/channel. It then restores COM7's original profile and
verifies config ACK zero, the same boot, exact status and exact sync. Both
restorations pass in this single automated invocation; manual recovery is not
required. Desktop and both peer processes exit 0 and the custodian stops.
Raw results retain their original pending-visual-review values.
[qualification.json](qualification.json) records the completed visual review
and SHA-256 hashes of all 51 frozen evidence files. The
[earlier failed wrapper/readback and separate recovery](../native-reconnect/README.md)
remain unchanged. This clean run qualifies the revised restoration order.

Executable hashes match the [software checkpoint](../runtime-software/executables.json).
Rust sources and supplier pins are unchanged; no new software-suite or build
claim follows. `source/` freezes the exact run scripts. Local attributes preserve
receipt bytes. Private wallets, opaque application stores and lock files are
excluded. This run's synthetic custodian root remains ignored and stopped after
automatic approval review rejects its deletion with "blocked by policy"; the
lane now retains 25 such roots. The existing upstream Cargo cache remains
co-owned. No new Cargo target, Cargo home or worktree was created.
