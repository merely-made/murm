# Djinn-backed Tucket reconnect-send pass, 2026-10-16

Source `e00ccd32ce2d26e633ea00f716be481c403bc76c` passes the Windows native
Messages scenario with Tucket over COM7 Tulle and stock MeshCore 1.17.1 on
COM6. Four tagged texts and all four protocol ACKs match across two delivery
connections. The first exchange's exact history is checked after reconnect,
before either new message is sent. All four records remain after final
disconnect. Separate tags and stock-peer receipts prevent the earlier ACKs
from satisfying the new exchange.

The real isolated Keeper wallet supplies the same device/realm identity on
both connections through `DjinnStationAuthority`. No seed-file fallback or
personal vault is used. This qualifies the scoped consumer route, not an
installed personal daemon or broader platform D13 revocation work. Public
configuration, contact card and journals are included; private wallet/runtime
files are excluded.

The fixture explicitly requests a stock advertisement after each Tucket
commissioning. After reconnect, the runner waits for the desktop's readiness
marker, which requires the first matched exchange and exact reopened history,
then advertises and starts the second peer helper. This qualifies rediscovery
with that explicit fixture, not automatic contact restoration or an arbitrary
advertisement schedule. Reply turnaround remains 200 ms. Stock helper exchange
times are 1937 ms and 1474 ms; these are helper totals, not per-message latency.

All seven captures were visually reviewed. The 230-frame native run has zero
blank captures and seven distinct digests. Connection state, both exchanges,
ACK labels and retained history agree with the journals. The stale lower
connecting notice remains; the four-record view also clips the oldest row
at the viewport bottom. Functional acceptance does not qualify UI polish.

COM7's saved same-boot profile/sync is acknowledged. COM6 is restored to the
pinned Tulle ELF with exact saved status, identity, region and channel. The
desktop and both peer helpers exit zero; the custodian is stopped.

[qualification.json](qualification.json) records the bounded pass, visual
review and exact evidence hashes. Raw runner outputs retain their original
pending-visual-review values. Frozen files preserve their original bytes in
Git. The [prior history-only pass](../native-turnaround/README.md) and two
earlier failed Djinn trials retain their original scope/results.

The [guarded recipe](../run-djinn-reconnect.ps1) uses the same pinned hardware
and performs restoration in its finalizer. Its supporting 41 desktop tests
(one existing ignored), strict all-target Clippy and binary/exporter build pass;
see `reconnect-desktop-*.log` in the [parent directory](../README.md). The core
implementation is unchanged from the prior 300-execution software gate.
