# Protocol and firmware feature parity

**Date:** 2026-10-16. **Status:** in progress; current host/target memory build
checkpoint passed, with physical peak/stack/reserve acceptance open. The owner
approved proceeding after the proposal. The owner asked about parity for Sennet/Meshtastic,
Tucket/MeshCore and Retinue/Reticulum, relative difficulty, firmware and board
memory. This plan proposes ordering and measurable completion conditions.

Related: [family integration](2026-10-13_murm_family_move_plan.md),
[wire compatibility](2026-09-27_wire_compatibility_plan.md),
[resident controller and memory](2026-09-10_murmuration_controller_plan.md),
[RNS core review](2026-10-07_rns_157_comparative_review.md),
[RNS interface review](2026-10-09_rns_157_interface_review.md).

## Target and ownership

Pursue all three, with separate completion records for protocol behavior,
device role and firmware/product features. A passing endpoint text exchange
does not qualify a router, repeater, server, peripheral module or unattended
installation. Full parity is the union of named profiles; each profile names
the reference version, host/board, capacities and applicable features.

Retinue owns Reticulum protocol state, Sennet owns its independently authored
Meshtastic-compatible implementation, and Tucket owns MeshCore-compatible
state. Tulle/radio-hand execute physical radio work and restoration. Dramatis
and Djinn keep identity/custody authority. Signalman supplies the application
controls and observations. Existing provenance rules remain in effect; this
plan does not authorize new source donors or change licenses.

Reticulum is a network stack, not a directly comparable MCU firmware product.
Keep its host stack/interface parity distinct from its bounded embedded-node
profile and from the RNode-compatible modem role. Outrider/LXMF and applications
are separately named higher layers. A full host feature list need not be linked
into every constrained radio image.

## Findings, 2026-10-16

| Family | Verified starting point | Remaining parity work to inventory/complete | Relative judgment |
| --- | --- | --- | --- |
| Tucket / MeshCore | Identified 1.17.1 adverts, private text/ACKs, learned paths, retry/flood fallback and one-repeater routes at all three prefix widths; native Djinn-backed two-connection text/history pass | Persistent contacts/restart, multiple repeaters, scoped routing, full companion commands, repeater/room-server/management/sensor roles and current-release differences | Easiest overall; compact role structure and MIT upstream reuse within existing provenance rules |
| Retinue / Reticulum | Extensive RNS 1.5.7 black-box qualification and subsequent core comparative repairs; embedded Retinue and bounded routing receipts | Reconcile current-main interface coverage with the integration review; finish remaining host services/interfaces and platform acceptance; embedded capacity, persistence and hostile-traffic qualification | Deepest transport/reliability and host-interface problem; much of the core already exists |
| Sennet / Meshtastic | 2.7.26.54e0d8d channel and PKI directed text, correlated ACKs, duplicate handling, retained native history and automatic board restoration | Broader roles/routing and channels, discovery/configuration, current security modes, stock-client-facing device behavior, position/telemetry and module/product coverage | Hardest whole-product catch-up; broad firmware surface and independently authored wire implementation |

This ranking is engineering judgment, not a measured effort estimate. Full
Reticulum host/interface breadth could dominate if every reference interface
and daemon service is required. The first implementation slices should follow
current missing behavior rather than reset already passing core work.

Reference drift is explicit:

- Tucket's identified physical baseline is 1.17.1. Official
  [MeshCore 1.18.0 notes](https://blog.meshcore.io/2026/10/15/release-1-18-0)
  dated October 15 add companion CLI/remote administration, telemetry push and
  broader combined BLE/Wi-Fi builds. These are upgrade targets, not qualified
  Tucket features. The notes explicitly report greater RAM/flash use when
  connection stacks coexist.
- Sennet's passing stock target is 2.7.26.54e0d8d. Current
  [Meshtastic security documentation](https://meshtastic.org/docs/development/reference/encryption-technical/)
  describes additional 2.8.1 authenticated channels and ACK proofs. Its
  [module catalog](https://meshtastic.org/docs/configuration/module/)
  includes telemetry, store/forward, MQTT, traceroute and peripheral functions.
  Keep those separate from R8's unauthenticated channel-ACK correlation.
- The latest recorded local RNS live oracle is 1.5.7; historical 1.5.4 receipts
  and older README assertions do not establish the current gap list. In
  particular UDP and Unix AutoInterface exist in current source. The October 14
  interface review names work on `iv/integration`; that ref is unavailable in
  this local checkout, so its status must be reconciled with receiving main
  before marking individual features missing or complete.

### Hardware and memory evidence

The two board families are Heltec WiFi LoRa 32 V4 (ESP32-S3) and Mesh Node T114
(nRF52840). The current stock/native pair actually uses two V4 units on COM6/7;
the T114 is a separate previously qualified board. This audit opens no ports.

| Board | Hardware and application budget | Measured evidence | Present conclusion |
| --- | --- | --- | --- |
| V4 | 512 KiB internal SRAM, 16 MB physical flash; both existing V4 captures identify 2 MB PSRAM, rather than the newer R8's 8 MB | September resident image reserves 64 KiB heap; data+BSS 202676 bytes and linker stack region about 129824 bytes. Physical ordinary three-protocol workload: allocator peak 1444 bytes, zero allocation failures; sampled stack use 61328 bytes | Existing bounded resident workload fits. Full tables, latest R8 PKI, BLE/Wi-Fi, peripherals and worst-case stack are not qualified together |
| T114 | 256 KiB physical SRAM / 1 MiB internal flash. Current linker permits 232 KiB application RAM and 768 KiB code flash after reserved boot/durable regions; fixed heap is 48 KiB | August AIR3 native Retinue flood: 32 peers/16 routes, peak heap 18168/49152 bytes with saturation/refusal behavior observed | Measured Retinue profile fits, with 30984 heap bytes left at that observed peak. Three current protocol instances together remain unmeasured on this board |

Sources: manufacturer [V4](https://heltec.org/project/wifi-lora-32-v4/) and
[T114](https://heltec.org/project/mesh-node-t114/) specifications;
existing V4 identification logs
[COM6](../testing/receipts/sennet-tucket-current/physical/com6-uncompressed-recovery.log)
and [COM7](../testing/receipts/sennet-tucket-current/physical/reconnected/com7-uncompressed-recovery.log);
[MC4c physical receipt](2026-09-13_murmuration_resident_physical_receipt.json);
[AIR3 T114 receipt](2026-08-13_air3_t114_on_air_receipt.md);
[T114 linker](../firmware/t114-phy/memory.x) and
[fixed heap](../firmware/t114-phy/src/heap.rs).

These figures are historical measurements, not fresh current-image guarantees.
The V4 stack figure is a current-pointer sample, not high-water. Its tiny heap
peak is an ordinary workload, not full-capacity stress. The current capacity
fixture's October 2 receipt records a different host workload peak of 18380
requested bytes and an Xtensa inline resident layout of 3744 bytes; neither is
a target allocator/stack measurement. Preserve those distinct scopes.

PSRAM is not credited to the current firmware heap budget: its allocator is an
internal static array and no external-memory allocation path is qualified.
Physical flash is likewise not all freely usable application space. Keep
bootloaders, partitions, identity/settings and monotonic reservation ranges
outside code and data growth. Retaining all three protocol states costs memory
even while only one protocol is using the SX1262.

## Proposed phases and done-conditions

1. **Freeze the parity matrix and target profiles.** Inventory each reference
   release's endpoint, relay/server, management and optional application
   features against actual receiving code. Reconcile the RNS interface review
   with main. Every row gets implemented/tested/physical/firmware status and a
   versioned acceptance case, or a hardware-specific applicability explanation.
   Done when the matrix distinguishes pinned baselines from upgrade targets
   and all gaps have an implementation owner.
2. **Refresh board memory before expanding firmware.** Build current combined
   protocol images for V4 and T114 using existing stable target locations.
   Record flash sections, static RAM, per-task/interrupt stack bounds, allocator
   occupied peaks/failures, and queue/table capacities. Include the new Sennet
   PKI path and simultaneous retained state. Stress full tables, maximum frames,
   outstanding ACKs/resources, churn, fragmentation and restart/refusal paths.
   Done when both profiles fit with an explicitly selected reserve margin and
   preserve state under pressure. No numerical reserve margin is silently
   adopted by this proposal; it belongs in each reviewed board profile.
3. **Finish Tucket endpoint, then repeater and server roles.** Start with durable
   contact restoration so the existing reconnect fixture needs no fresh peer
   advert solely to rebuild known contact state. Follow with multiple-hop and
   scoped routing, companion/management coverage and 1.18 upgrade cases. Track
   room-server persistence and sensors as distinct feature rows. Done per role
   when stock peers complete normal/retry/restart/failure paths with bounded
   state, and the chosen firmware independently passes the same behavior.
4. **Finish Retinue's pinned host and embedded profiles.** Reuse the landed
   comparative repairs; close remaining interface/shared-instance/management
   and platform cases from the reconciled review. Give resource sizes, links,
   routes and queues explicit configurable bounds in the embedded profile.
   Done when reference-side state agrees across the selected interfaces and
   repeated embedded transport/resource/restart cases fit the board budgets.
5. **Expand Sennet toward role and product parity.** Carry current PKI/counter
   behavior into standalone firmware, then finish chosen routing/channel and
   client configuration/discovery surfaces. Add current security modes,
   telemetry/position and remaining module families with independent public
   documentation/stock-capture evidence. Done per feature/role when identified
   stock peers/clients interoperate and restart, malformed-input and bounded
   memory behavior pass on the named board.
6. **Complete firmware/product acceptance.** Qualify USB, applicable BLE/Wi-Fi,
   display/GNSS/sensors, persistent configuration, upgrades/recovery, sleep/power,
   and unattended operation. Shared-radio schedules additionally measure missed
   receive windows and deadline behavior. Done when each advertised board/role
   profile has a reproducible whole-image receipt including maximum workload,
   power-cut recovery and fault handling. A single SX1262 still receives only
   its active compatible PHY profile; feature parity does not create simultaneous
   coverage of three incompatible networks.

## Progress

- 2026-10-16: inventory and proposal recorded. Existing receipts and source were
  read, and official feature/device documentation checked. No new target build,
  firmware flash, physical test, selected capacity or acceptance claim follows.
  Recommended sequence: memory refresh, Tucket, Retinue, Sennet, with common
  firmware infrastructure advanced as each concrete role requires it.
- 2026-10-16: current memory fixture now retains actual Sennet/Tucket instances
  with maximum PKI texts, a full private seen window and ACK/resource work.
  Host peak is 15528 requested bytes and zero remain after drop. The actual
  LLFF allocator passes sixteen fragmented rounds at both 48/64 KiB, with an
  occupied peak of 20624 bytes, no unexpected refusals and one deliberate
  oversize refusal per campaign. These are host measurements, not target peaks.
- 2026-10-16: both ARM/Xtensa combined capacity images link. Target inline
  retained objects total 10064 bytes; radio-hand Runtime separately totals
  20256 bytes. Actual V4 resident data+BSS is 224380 bytes and its linker stack
  region is 108136, materially tighter than the old image. The T114 firmware
  link and a separate combined capacity example fit their protected memory
  windows. GNU readelf corroborates section reports. See the
  [current memory receipt](../testing/receipts/protocol-parity-memory/README.md)
  for exact artifacts, source hashes, compiler exception, workload and limits.
  No image is installed and no serial port opened. This closes the fresh
  host/build slice, not phase 2's on-device peak, stack, faults or reserve gate.
