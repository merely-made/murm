# Sennet directed text across native reconnect, 2026-10-16

At Murm `826752791da`, Signalman's native Messages controls pass four exact
directed PKI texts and four correlated protocol ACKs over two connections.
Sennet runs on this computer through Tulle on COM7; the identified stock peer
on COM6 runs Meshtastic 2.7.26.54e0d8d. Exact history is checked before fresh
work after reconnect, and all four records survive final disconnect.

The same scoped device/realm identity is released through an isolated real
Djinn Keeper wallet. Its public node is `7dc97cd4`; the stock peer is `f66afb28`.
The stock contact import has independent SDK and QR public-key readback, and
the native activation pins the peer's complete key from identified USB readback.
Both incoming texts authenticate under that pin. Both native outgoing records
have physical completion and matched code-zero routing responses; the stock
peer independently matches their exact texts and reports ACKs for both returns.
This qualifies the pinned 2.7 wire profile. Its channel ACKs are unauthenticated
correlation facts, not read receipts or 2.8 authenticated ACK proofs. Installed
personal-daemon integration, private-channel custody, broader mesh routing and
platform D13 remain separate.

Text packet IDs 1 and 257 and control IDs 2 and 258 use disjoint durable
intervals 1–256 and 257–512. Next state advances from 1 to 513; unused IDs are
abandoned on reconnect. This is local reservation/reopen evidence, not a
hardware power-cut or global rollback claim. Peer helper totals of 16843 ms
and 4258 ms cover service start through return ACK, not per-message latencies.

All seven captures are visually reviewed: 226 presented frames, zero blank
captures, seven distinct digests and all four final rows visible. The connecting
notice remains stale after connection and the long journal path is clipped in
its input. Functional behavior is qualified; UI polish remains open.

The native scenario and all three message processes exit 0. The wrapper's first
DUT restoration readback fails because asynchronous radio bytes are mixed into
its textual status/sync response, despite a successful config ACK and unchanged
boot ID. Its raw `bench.json` and `custodian.json` therefore remain **failed**.
COM6 then restores to the pinned Tulle image with exact status/identity/region/
channel. A separate invocation of the unchanged strict restoration probe, after
that peer restoration, passes on COM7 with the same boot, status and sync and
config ACK zero. Both boards are restored and all owned helpers/custodian stop.
The follow-up wrapper change restores the stock peer first; this run qualifies
the original native source plus the recorded separate restoration recovery,
not an end-to-end automated pass of that later wrapper revision.

[qualification.json](qualification.json) records the reviewed result and hashes
of all 53 frozen public evidence files, including both restoration attempts.
The raw reports are unchanged. Private wallets, opaque application stores and
lock files are excluded. `source/` freezes the exact run scripts; local attributes
preserve receipt bytes. The [software ledger](../README.md) retains supplier,
clean-room provenance and earlier failed wire experiments. Cleanup approval is
recorded, but automatic approval review blocked fixture/empty-worktree deletion;
stopped ignored wallets, the empty Mere directory and its published local branch
remain. Existing targets and the co-owned upstream Cargo cache are retained.
