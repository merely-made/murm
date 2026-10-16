# Sennet channel text across native reconnect, 2026-10-16

At Murm `bf3301caa58`, Signalman's native Messages controls pass bidirectional
public LongFast text through Sennet on this computer and Tulle on COM7,
against the identified stock Meshtastic 2.7.26.54e0d8d peer on COM6. Four exact
tagged texts match over two connections. Exact history is checked before new
work after reconnect, and all four records remain after final disconnect.

The first qualifying run uses scoped Sennet custody through an isolated real
Djinn Keeper wallet. The public node is `04b0ec39`, with stock peer `f66afb28`.
This does not qualify the installed personal daemon, broader platform D13
revocation, sender authentication, directed messages, protocol ACKs or private
channel key custody. The configured LongFast key is deliberately public.

Outgoing Sennet records have physical completion and `HandedToRadio`; stock
receipts independently prove their exact reception. Stock return records have
device acceptance, and the native Sennet end independently matches each return.
No record claims a protocol ACK. Helper totals are 11468 ms and 888 ms, measured
from peer service start through device acceptance, not per-message latency or
an RF completion measurement at the stock transmitter.

The identity-bound allocator reserves intervals 1–256 and 257–512 before use.
The two Sennet outgoing packet IDs are 1 and 257; unused IDs from the first
connection are abandoned. Durable next state is 513. The commissioning fence
is retained. This demonstrates local reservation/reopen behavior, not hardware
power-cut durability or resistance to deliberate rollback of all state files.

All seven captures were visually reviewed: 530 presented frames, zero blank
captures and seven distinct digests. Connection, both round trips, reopened
history and final disconnect are visible. All four final history rows fit this
viewport. The connecting notice remains stale after connection, and the long
journal path is clipped within its text input. UI polish is not qualified.

COM7's saved same-boot profile/sync and COM6's pinned Tulle image with exact
status, identity, region and channel were restored and read back. Both peers
and desktop exited 0; the isolated custodian stopped. Private wallet/runtime
files and the opaque application store are excluded. Original raw reports keep
their pending-visual-review values; [qualification.json](qualification.json)
records this completed review and SHA-256 hashes of all 43 frozen evidence files.
Local attributes preserve their exact bytes across checkout platforms.

The [software and supplier ledger](../README.md) records the qualified source
cohort and tests. The [initial setup](../setup-initial/README.md),
[readback discriminator](../setup-readback/README.md) and
[USB transition refusal](../native-usb-boundary/README.md) preserve the failed
steps preceding this pass. No erase or factory reset was used.
