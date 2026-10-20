# Protocol and firmware feature parity

**Date:** 2026-10-16. **Status:** in progress; the named bounded firmware memory
profile passed on 2026-10-17, including boot-painted stack high-water, heap,
queue/failure/reconstruction campaigns and selected reserves. Broader firmware
product acceptance remains open. The owner
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
| Tucket / MeshCore | Identified 1.17.1 adverts, private text/ACKs, learned paths, retry/flood fallback and one-repeater routes at all three prefix widths; native Djinn-backed two-connection text/history pass; scoped host contact reload; V4 standalone contact storage/reboot exchange and T114 storage backend | T114 live service, multiple repeaters, scoped routing, full companion commands, repeater/room-server/management/sensor roles and current-release differences | Easiest overall; compact role structure and MIT upstream reuse within existing provenance rules |
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

The [on-device continuation](../testing/receipts/protocol-parity-memory/physical/README.md)
now measures the named combined diagnostic profile on T114 and the COM6 V4.
Both complete sixteen fragmented rounds with an occupied peak of 18452 bytes,
zero occupancy after each round, sixteen deliberate oversize refusals and no
unexpected refusals. T114's heap is 49152 bytes; V4's is 65536. The diagnostic
runs from a separate executor task after a nested-stack version overflowed
T114's stack. These are synthetic retained states inside real firmware, not
simultaneous three-network traffic or the live full radio-hand Runtime.
V4's main-handler stack sample is 97344/107960 bytes, leaving only 10616;
T114's lowest worker checkpoint samples 61344/84932. Neither is true high-water.
Heap fit therefore does not close the whole-image memory or reserve gate.

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
3. **Finish Tucket endpoint, then repeater and server roles.** Scoped host and
   bounded standalone contact restoration are now qualified below; the V4 reboot
   fixture needs no fresh peer advert to rebuild known contact state. The T114
   live endpoint is now qualified below. Continue with multiple-hop and
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

### Memory closure and persistent-contact continuation, 2026-10-16

Owner-authorized sequence: finish the bounded firmware memory gate, then
Tucket persistent contacts. Total retained receipt/input budget for this
continuation is 100 MB across repositories. Required evidence is a compact
source/image/tool manifest, target memory layout, boot-painted stack high-water,
heap/queue/driver-pressure and restart/refusal outcomes, protected-state
restoration, and bounded contact-save/reload/failure tests. Prior cited evidence
is immutable. Full logs are retained only for these acceptance cases.

Selected acceptance reserves are 16 KiB untouched stack and 25% free heap,
supplied explicitly to the runner rather than imposed as firmware defaults.
The owner authorized justified limits. Stack reserve is an engineering cushion
greater than three of the roughly 5.2 KiB crypto-leaf frames seen in the target
disassembly; full protocol/runtime construction frames are exercised, rather
than charged to that reserve. It covers additional interrupt/callee variation
within this bounded profile, not arbitrary unexecuted paths. Heap reserve is
12 KiB on T114 and 16 KiB on V4, each larger than the measured 9932-byte retained
protocol allocation above fragmentation guards. Construction/reallocation
overlap and full queues are exercised before applying this separate cushion.
These are conservative workload acceptance limits, not formal worst-case
proofs or permission to expand capacities without a fresh qualification.

The measured profile is the existing 8-peer/4-action/1-link/4-route Retinue,
8-contact/32-dedup/2-pending Tucket, and 8-directory/64-private-seen Sennet
diagnostic, plus four real Tucket-only runtime reconstructions per round.
Each reconstruction fills eight work slots, refuses the ninth and oversized
ingress, rejects stale completions, accounts for eight failed transmissions
and drops all state. Synthetic identities never reach RF. All executor tasks
and IRQ activity share the measured MSP on T114 or CPU0 stack on V4; this
V4 configuration starts neither CPU1 nor an independent radio-stack thread.
The RTOS idle path reuses the main stack. This qualifies the bounded memory
profile, not simultaneous three-network reception, scheduling deadlines,
physical radio-failure recovery or unattended/power-cut product acceptance.

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
- 2026-10-16: the separate physical continuation passes the sixteen-round
  allocation/fragmentation/refusal campaign on T114 and COM6 V4. Their common
  occupied peak is 18452 bytes; allocations return to zero every round.
  A direct allocator call avoids LLVM eliding the refusal test, and a separate
  executor task avoids nesting the fixture on the large normal poll stack.
  Failed attempts, target crash/disassembly evidence, the attended T114 DFU
  watchdog bridge, exact V4 application/state comparisons and T114 live
  identity/profile restoration checks remain individually recorded. COM7's
  post-flash USB problem is tracked separately and supplies no successful
  memory result. Phase 2 still requires real task/interrupt stack high-water,
  pressure/restart/fault coverage and selected reserve margins. See the
  [physical receipt](../testing/receipts/protocol-parity-memory/physical/README.md).

- 2026-10-17: the [bounded firmware memory gate](../testing/receipts/protocol-parity-memory/closure/README.md)
  closes for source `a7107a7266f` and the named capacities above. Both boards
  pass sixteen rounds with a heap peak of 18452 bytes, no unexpected allocator
  refusals or leaks, and sixty-four runtime reconstructions each. Minimum
  untouched stack is 62140 bytes on V4 and 18916 on T114, above the explicitly
  selected 16384-byte reserve; both retain at least 25% free heap. V4 state
  borrowing removes its large duplicate runtime frame; in-place fixture
  construction and sequential ARM pressure stages preserve capacities while
  removing artificial temporary nesting. Production images also link:
  V4 data+BSS 204476 / stack region 128040; T114 allocated SRAM 152668 / linker
  gap 84900. Linker gaps remain distinct from measured high-water.
  Both tested boards return to their original named firmware and profiles;
  SB2 protected state is byte-identical and its prefix ROM MD5 matches the
  fresh backup. T114 identity/sequence/profile and application-only DFU pass
  without claiming protected-flash readback. SB1 is untouched. The separate
  final hardened clean-source campaigns preserve all earlier failed attempts.
  This closes phase 2's named memory profile, not physical radio-driver,
  power-cut, deadline, unattended or whole-product acceptance. Continue with
  phase 3: persist Tucket's verified public contacts through the existing
  identity/realm-scoped Signalman atomic journal, then prove cold restoration
  and acknowledged traffic without another peer advert. Private identity
  custody and pending-operation restart semantics retain their existing owners.

- 2026-10-17: phase 3's first [persistent-contact slice](../testing/receipts/tucket-contacts/README.md)
  passes cold file-journal reload into an empty runtime, acknowledged text
  without any injected new peer advert, exact history, atomic-save failure,
  malformed/scope/collision/capacity refusals and paused-personality restoration.
  Signalman's existing identity/realm-scoped journal owns public peer keys;
  new contacts have no saved route, dedup or pending work. Contact writes use
  version 2 so older writers refuse before dropping the cache; empty-cache
  version-1 books still load. Node restoration uses already reserved capacity.
  The native Djinn-backed run exchanges four texts and four protocol ACKs
  across two connections with the reconnect re-advert suppressed. Seven
  captures are reviewed. Full suites pass 83 Tucket, 75 Signalman and 168
  radio-hand tests; formatting, strict core/desktop Clippy and both bare-target
  checks pass. The three unrelated existing LOC failures remain explicit.
  This closes scoped host contact restoration, not MCU flash persistence,
  stock-peer reboot persistence, power-cut or whole-role parity. Next endpoint
  work is bounded multiple-hop/scoped routing and companion/management cases,
  with standalone firmware persistence separately owned and qualified.

### Standalone contact storage continuation, 2026-10-17

The owner requested standalone firmware contact storage next. This lane retains
at most 100 MB of inputs/receipts and preserves all earlier evidence. Required
receipts are bounded codec/journal refusal and every-word interruption tests,
target links, real flash cold reload, a V4 resident exchange after restart
without a fresh peer advert, and explicit restoration/protected-state checks.
At this 2026-10-17 checkpoint, T114 had no live Tucket loop: its adapter and pre-radio storage/reset campaign
qualify storage only, not Tucket firmware service or RF interoperability.

The opt-in V4 `resident-contact-store` profile uses its own 4 KiB A/B sectors
at `0x3fa000..0x3fc000`, following the packet-ID lease pair. T114 reserves its own pair
at `0xe4000..0xe6000`, shortening application FLASH while preserving the
existing control, reservation and identity addresses. The portable profile
contains eight full public peer keys, local Tucket public identity and a
domain-separated receive-profile digest. Transmit power does not select a new
scope. Labels, trust, routes, pending operations and private identity material
are excluded. The selected volatile identity/setup must still be supplied
again after boot; this lane does not persist credentials or commissioning.

Records use the existing CRC-protected outer format and a versioned `TKC0`
body. The commit magic is programmed last. Torn inactive candidates leave the
previous committed slot authoritative; an interrupted first write can retry
when neither commit word was written. Committed unknown formats/versions,
overflow, malformed keys, mismatched scope, conflicting equal sequences and
sequence exhaustion fail before erasure. A readback error after a complete
program is an uncertain commit, not a promise of rollback. Duplicate adverts
do not write. V4 live writes pass through the exclusive cancel-safe radio
quiet guard before protocol transmission resumes; its listening blackout is
explicit rather than described as continuous coverage.

This requested option narrowly extends the field-relay seizure inventory:
public keys reveal peer associations and are classified pseudonymous. The
default relay keeps host-only contacts. Both cache schemas are checked by the
flash classification audit. The board storage fixture is an explicit separate
feature, uses synthetic identities only in flash, and never emits them on RF.

The [completed receipt](../testing/receipts/tucket-firmware-contacts/README.md)
records V4 full-capacity reload across two resets and T114 reload across an
application-only reinstall, with unchanged snapshots producing zero commits.
Both pass sixteen full pressure rounds at the selected 16384-byte untouched
stack / 25% free-heap limits. T114 retains 18836 stack bytes and peaks at
18452/49152 heap bytes. The V4 pressure image explicitly charges the new
20600-byte boxed resident state: peak 39052/65536, zero final occupancy, and
101812 untouched stack bytes. This is a V4 storage charge, not qualification
of a boxed three-core T114 service that does not yet exist.

The first real V4 constructor exposed a path the earlier fixture did not
exercise: roughly 48 KiB USB poll + 73 KiB constructor + 20 KiB runtime frames
exceeded its roughly 127 KiB stack region. A separate executor turn and boxed
result remove that nesting and duplicate inline copies. The flash/radio owner
retains its quiet guard; the worker receives only volatile setup, verified
leases and validated public keys. Scope validation still precedes reservations.
The live image's minimum untouched stack is 51792 bytes, and heap peaks at
21980/65536 after actual traffic. Production V4/T114 storage profiles also link.

The final live source `fb7e0e6eacc` repeats the passing `21ac0176a22` exchange:
four texts/four protocol ACKs across
two V4 boots. Before new peer work on the second boot, one contact is installed
with RX/TX counters both zero; no reconnect peer advert is issued. A fixed
one-second retry interval had overlapped responses. Firmware now derives that
interval from the caller's TX budget plus turnaround (4200 ms in this profile),
with a delayed-ACK regression. Eighty-four Tucket and 176 radio-hand tests,
strict core/host-probe Clippy, formatting and schema/unsafe checks pass. Two
existing application LOC failures remain separate.

Original named firmware and radio settings are restored. V4 prefix comparisons
are exact, immutable tail ranges are exact, and packet/announce ceilings advance
without rollback. Generated test contacts occupy only this lane's new pairs.
Independent bench identities were cleared only after exact scope/preimage
checks, retaining their raw before-images and comparing every other tail byte;
neither clear occurs between the passing reboot exchanges. T114 restoration
checks live identity/profile, without claiming a protected-flash dump. The
initial allocation guard, USB fragmentation failure, constructor stall, retry
failures and a checksum-format parser refusal remain individually retained.
Physical power-cut, stock filesystem persistence, unattended/driver-deadline,
broader routing/roles and the T114 Tucket loop remained open at that checkpoint. The later T114 continuation below qualifies its bounded live endpoint.

### T114 live Tucket continuation, 2026-10-20

**Status: physically qualified for the bounded endpoint profile.** The opt-in
Tucket-only service uses the existing
T114 radio/USB owner. Keep eight contacts, 32 dedup entries, two pending text
operations and eight queued frames, with radio-budget-derived retry timing.
Retain the explicit 16384-byte untouched-stack and 25% free-heap limits.
The pressure image charges the exact live state's boxed storage alongside the
existing fragmented protocol workload. Construction and actual RF traffic
also need boot-painted stack/allocator measurements.

Required receipts: endpoint queue/expiry/ACK/refusal tests; production and
instrumented target links; sixteen bounded pressure rounds; scoped Djinn
commissioning, bidirectional exact text/ACK and cache restoration across reboot
without a fresh peer advert; original firmware/identity/profile restoration.
Keep new inputs under `validation/results/tucket-t114`, with public evidence
under `testing/receipts/tucket-t114`. Total retained input/receipt cap is
100 MB across this lane and its retained prior radio/memory inputs. Preserve
failed trials. No private setup or flash dump is published.

The prior T114 cache contains the exact eight-key synthetic non-RF fixture.
An explicit bench-only pre-radio handoff image may clear only that exact
snapshot at sequence 1, touching only `0xe4000..0xe6000`. Other scopes, keys,
sequences and corruption refuse. This is separate from product commissioning
and is never run between the accepted first/reboot exchanges. Private identity
setup remains volatile, released through scoped Djinn custody after each boot.

The `30733812192` instrumented image passes sixteen physical pressure rounds
on USB `TULLE-T114-01`: minimum boot-painted untouched stack 75820/139420
bytes, maximum occupied heap 21860/49152 bytes (55.5% free), zero final heap
occupancy, and exactly one deliberate oversize refusal per round. Every round
also exercises the new endpoint's eight-frame overflow/stale-completion/driver-
failure/reconstruction path, alongside the prior full protocol workload and an
exact 3408-byte live-state storage charge. Production, fixture-handoff and
instrumented application-only images link below `0xe4000`. The original named
firmware, identity and profile are restored and verified after this campaign.
The [final receipt](../testing/receipts/tucket-t114/README.md) repeats all sixteen
rounds on final source `9de106432ad`, with identical figures after suppressing
empty idle USB reports. Four exact texts and four protocol ACKs pass against
stock MeshCore 1.17.1 across two boots. Before new peer work after reset, the
saved contact is already installed; the full second transcript contains no
fresh advert. Live construction/RF retain at least 120308 stack bytes, with a
6404/49152 maximum heap peak and zero allocation failures. The production role
also links, without constructing Retinue/Sennet instances or their nonce leases.

Scoped real-Keeper custody controls repeat using the exact previously qualified
Tucket-only supplier; the current host probe honors the public DTR setting.
Both original named firmware/profile states restore, SB1's prefix and full tail
compare exactly, and owned helpers stop. T114 restoration checks live identity,
settings sequence, profile and application only. The generated public bench
contact remains in its dedicated pair. USB identity setup remains volatile.
Battery-connected software reset, one-hop endpoint traffic and selected pressure
are the acceptance scope. Power cuts, installed personal-daemon/UI integration,
physical deadline/fault trials and broader routing/repeater/server roles stay open.
