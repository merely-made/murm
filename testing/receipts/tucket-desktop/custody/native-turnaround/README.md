# Djinn-backed Tucket native functional pass, 2026-10-16

Source `cf06a1a9569bff0b73a138b84caf72f523d85358` passes the Windows native
Messages scenario with Tucket over COM7 Tulle and a genuine stock MeshCore
1.17.1 peer on COM6. Both tagged texts match and both outgoing messages have
matched protocol ACKs. The app disconnects, reconnects with exact history,
then disconnects again. Coverage is two messages on one delivery connection
and one history-only reconnect; there are no new sends after reconnect.

The run uses `DjinnStationAuthority` with a real isolated Keeper wallet and
explicit device UUID/realm, plus public configuration/contact card. Private
wallet/runtime files are excluded. This qualifies the scoped consumer route,
not an installed personal daemon or broader platform D13 revocation work.

The configurable reply delay is explicitly 200 ms. This follows the pinned
stock implementation's [ACK turnaround](https://github.com/meshcore-dev/MeshCore/blob/d929643/src/helpers/BaseChatMesh.cpp#L3-L49).
The first two Djinn trials missed the stock return ACK, including one which
kept the app connected awaiting peer confirmation. Their frozen evidence is
in [native-initial](../native-initial/README.md) and
[native-peer-gate](../native-peer-gate/README.md). This delayed-reply build is
the first full functional pass; it does not establish results for other delays.

The native run completes 273 frames with six distinct, nonblank captures.
All six PNGs were visually reviewed: connection state, both text bodies,
protocol acknowledgement, retained history and final disconnection agree
with the journals. A lower connecting notice remains stale after connection;
visual polish is still open. The stock helper's 4935 ms is total exchange
timing, not per-message latency.

COM7's saved profile/sync was acknowledged on the same boot. COM6 was restored
to the pinned Tulle ELF and its exact saved status, identity, region and
channel verified. Both native processes exit zero; the custodian is stopped.

[qualification.json](qualification.json) records the completed visual review,
bounded functional pass and evidence hashes. Raw `bench.json`, `run.json` and
`custodian.json` retain the runner's original pending-visual-review result.
The source/binary/scenario ledger, journals, peer receipt, six captures,
public configuration and restoration readbacks are adjacent.

Software gates at this source pass 300 core executions and 41 desktop tests
(one existing ignored), strict core/desktop Clippy, bare radio-hand/Tucket
checks, binary/exporter builds and formatting. Their logs are retained in
the [parent custody directory](../README.md) as `turnaround-*.log`.
