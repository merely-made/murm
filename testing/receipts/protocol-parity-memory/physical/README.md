# On-device memory continuation, 2026-10-16

This continues the immutable parent host/build checkpoint at `280e40dd34c`.
The explicit `memory-bench` feature installs a synthetic three-protocol fixture
inside each real board firmware. It retains bounded Retinue, Sennet and Tucket
state, exercises 256 maximum private frames and duplicate ACKs per round, keeps
alternating fragmentation guards alive, then releases the state and deliberately
requests twice the allocator capacity. The direct allocator must refuse exactly
once per round without changing occupancy. The next round verifies continued use.
Fixture identities never reach the radio or durable storage.

The ordinary USB/radio firmware remains linked, but this is a diagnostic profile,
not a simultaneous on-air resident-runtime or product-parity acceptance. Retinue
uses eight peers/four routes/one link and a 1024-byte Resource; Sennet has eight
directory entries, 64 private seen entries and 220-byte texts; Tucket has eight
contacts and two outstanding 171-byte texts. These choices remain distinct from
the T114 native-node 32-peer/16-route production profile. The separate full
radio-hand Runtime layout from the parent checkpoint is not added again as a
live object by this diagnostic.

## Target measurements

| Measurement | T114, COM10 | V4, COM6 |
| --- | --- | --- |
| Fixed heap | 49152 | 65536 |
| Occupied peak | 18452 | 18452 |
| Fragmented guard baseline | 4544 | 4544 |
| Guards plus retained protocol allocations | 14476 | 14476 |
| Protocol allocations above guards | 9932 | 9932 |
| Occupancy after each complete round | 0 | 0 |
| Lowest worker stack checkpoint, expressed as bytes below stack top | 61344 | 51584 |
| Main-handler stack sample | 59704 | 97344 |
| Linker stack region/gap | 84932 | 107960 |

The campaign and restoration results are asserted separately in
`qualification.json`; raw measurements remain in each board's campaign JSON.
Each accepted campaign requires sixteen completed rounds and exactly sixteen
expected allocation failures. Peak occupancy includes temporary synthetic peers,
fragmentation guards and actual allocator metadata. It is not requested-byte
accounting or a post-flood estimate. Both host LLFF campaigns also exercise this
same target-round function and return to zero.

T114's final wrapper passes directly. V4's raw campaign wrapper reports failure:
a buffered, truncated ESP ROM segment-log fragment prefixes its first status.
All sixteen memory replies complete, with exact refusal counts and zero release
residue. The separate qualification removes only that specifically recognized
ROM prefix and requires the remaining complete status/identity to match exactly.
It never edits the raw failure. V4's restoration wrapper then loses USB during
its final `air` read; its completed prefix/tail comparisons and a separate fresh
status/profile probe establish restoration. The diagnostic image itself has
helper-transfer and live build-marker evidence, not an independent full readback.

Stack figures are samples at named checkpoints, not painted-stack high-water.
They omit deeper callees, startup maxima and interrupt nesting. In particular,
V4's main sample leaves only 10616 bytes above its linker floor; that is not an
accepted reserve. T114's sampled worker minimum leaves 23588 bytes. Phase 2's
real stack bounds, queue/driver pressure, power-cut/fault matrix and explicit
reserve selection remain open. No PSRAM is credited to the fixed heap.

## Findings and failed attempts

The first oversize check used the global allocation shim. LLVM removed its
unused allocation/free pair, leaving an unconditional assertion failure in the
ARM disassembly. The corrected check calls each board's allocator implementation
directly; the counter and refusal are now observable. Failed image hashes,
disassembly and crash reports remain separate evidence.

Calling the complete fixture from the T114 main poll stack then caused a hard
fault. Recorded PC `0002b686` is in the USB reply path after the workload.
Disassembly shows about 59640 bytes for the main poll frame, about 30600 for the
round and another roughly 30600 for the legacy fixture: their nesting exceeds
the 85020-byte linker gap before deeper calls. A separate executor task runs the
fixture after the main handler yields, so those stacks are reused instead of
nesting. It adds bounded static signal/task storage and does not move protocol
objects into the measured heap. Explicit fixture checkpoints sample SP; T114
also services its watchdog there. This synchronous bench does not prove USB or
radio scheduling deadlines under heavy work.

T114's inherited eight-second watchdog also interrupted the large serial DFU
handoff. The verified 1368-byte `dfu_watchdog_handoff` example waits for that
watchdog to reset, then requests serial DFU with WDT stopped. Application-only
DFU can then transfer the large diagnostic. This is an attended bench bridge,
not a changed production bootloader or a qualified general updater. Nordic's
[reset behavior](https://docs.nordicsemi.com/r/bundle/ps_nrf52840/page/power.html)
distinguishes soft reset from watchdog reset; the tagged Adafruit
[single-bank erase path](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/0.9.0/lib/sdk11/components/libraries/bootloader_dfu/dfu_single_bank.c)
erases the rounded application image span. Both inspected package spans end
below the protected `0xe6000` boundary.

COM7's USB data endpoint repeatedly stopped responding after flashing, including
after restoring its original bytes and both software reset methods. An owner
power cycle restored its original status once; a later attempt is retained as
another failed campaign, not counted as V4 execution. The alternate V4 on COM6
supplies the target memory run. Every serial write checks its exact USB identity.
Final independent probes find both V4s and T114 on their original firmware and
profiles after the owner's cable reconnection. Hardware identity here is a
software binding; an unmarked physical board cannot be identified by asking its
owner to recognize that serial string. Persistent visible device names are the
next owner-requested firmware slice.

## Restoration and provenance

V4 writes begin at `0x10000`, preserve the existing bootloader/partition table,
and stay below the fresh 1 MiB backup boundary. Restored prefix and protected
`0x3f0000..0x400000` tail comparisons are byte-for-byte checks. COM6's original
state is US915/rnode, slot A/sequence 10; COM7's is US915/modem, slot B/sequence 5.
Private flash backups are excluded from Git.

T114 returns to the saved `711816a8124bd58cd8cc314ec4de5009be7df166`
application, US915/modem, slot B/sequence 93, node `599997c8`. Its application-only
DFU completion and live identity/profile checks are recorded. This is not a
byte-identical protected-flash readback. Only the intentional watchdog bridge's
boot-cause/crash fields are normalized in restored status comparison; the memory
campaign itself requires unchanged before/after status and monotonically counted
refusals. Crash residue is preserved, not cleared to manufacture a clean result.

`qualification.json` binds final source hashes, images, runners and measurements;
`files.json` seals public evidence. Earlier failures remain failures. The final
T114 source only cfg-gates an otherwise unused bench getter; rebuilding proves
its complete application binary exactly matches the physically exercised bytes.
Selected Clippy gates permit only recorded pre-existing board warnings. The LOC
gate still has six unrelated baseline violations; this change extracts the V4
channel unit tests and removes that file's obsolete ceiling exemption.

Existing repository target/build paths and the co-owned `mere-upstreams` Cargo
home are reused. No isolated target/home/worktree is created. Ignored flash
backups and generated images remain owned by this physical receipt for recovery
and reproduction.
