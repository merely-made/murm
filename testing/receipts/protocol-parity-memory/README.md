# Current protocol memory checkpoint, 2026-10-16

**Result:** host execution and ARM/Xtensa build checkpoint passed. No firmware
was installed, no serial port opened and no radio command sent. Phase 2's
physical peak/stack/fault/reserve acceptance remains open.

The sources are base `bb919d665e6` plus the working bytes sealed in
[receipt.json](receipt.json). All 452 source-file hashes are recorded. This is
not a clean-revision release receipt. The unchanged legacy fixture remains
available; the new workload retains actual Sennet/Tucket instances alongside a
full Retinue Node/link/resource fixture.

## Exercised profile

Retinue: 8 peers, 4 actions, 1 link, 4 routes, two freshness blobs per peer,
255-byte ingress and a 1 KiB resource. Tucket: 8 full contacts/routes, 32 dedup
hashes and two pending 171-byte texts. Sennet: eight full directory rows, a
64-entry private seen window, maximum 220-byte PKI texts, duplicate re-ACKs,
malformed ciphertext and busy-send refusal, and an outgoing logical response
window coexisting with queued physical control work. Synthetic peers contribute
to construction peaks. All keys/nonces are test fixtures, never deployed keys.

## Host execution

| Measurement | Bytes / result |
| --- | ---: |
| Requested bytes retained by current workload | 10925 |
| Requested-byte peak, including temporary peers | 15528 |
| Host inline retained structure, separately from heap | 13312 |
| Remaining requested bytes after drop | 0 |
| Fragmented LLFF baseline, including surviving guards | 4992 |
| LLFF occupied peak, same result at 48/64 KiB | 20624 |
| Additional retained LLFF occupied workload bytes | 10968 |
| Repeated workload rounds per heap size | 16 |
| Unexpected allocation refusals | 0 |
| Deliberately oversized allocation refusals per campaign | 1 |
| LLFF occupied bytes after all state/guards drop | 0 |

The fixed-heap campaigns use the same LLFF allocator as firmware and retain
alternating guard allocations throughout. Reallocation overlap is included by
the allocator wrapper. They are **64-bit host ABI simulations**; target heap
peak, fragmentation, stack and timing are not inferred from them. Heap headroom
at this observed host peak is 28528 bytes for 48 KiB and 44912 for 64 KiB.

## Target builds

Both target compilers emit the same inline sizes, excluding heap storage:

| Target layout | Bytes |
| --- | ---: |
| Retinue Node at the selected 8/4/1/4 capacities | 9272 |
| Sennet retained instance | 400 |
| Tucket retained instance | 392 |
| Combined retained objects | 10064 |
| radio-hand Runtime, including inline work/report queues | 20256 |

The Runtime size is reported separately; this capacity fixture does not execute
its scheduler or work/report queues. Both capacity images reserve 64 KiB
radio/runtime and 16 KiB queue BSS placeholders, with 64 KiB heap on V4 and
48 KiB on T114. They have no USB/radio service loop. A combined T114 protocol
capacity image is not a three-protocol T114 radio-runtime implementation.

| Image | Static / linker result | Flash result |
| --- | --- | --- |
| V4 actual resident firmware | data+BSS 224380; CPU0 stack region 108136 | merged image 783776 bytes from origin 0, below protected state at 0x3F0000 |
| V4 combined capacity example | data+BSS 158872; CPU0 stack region 177164 | merged image 474928 bytes, same protected-state bound |
| T114 firmware built alongside the example | allocated SRAM sections 152540; gap above sections 85028 | load span 363490 of 786432 code bytes |
| T114 default firmware built separately | same SRAM sections/gap | load span 363106 of 786432 code bytes |
| T114 combined capacity example | allocated SRAM sections 141652; gap above sections 95916 | load span 263458 of 786432 code bytes |

V4 `.data` is 5268 and `.bss` is 219112 in the resident image. Compared with
September's resident receipt, their total grew by 21704 bytes; its available
CPU0 linker stack region is 21688 bytes smaller. The selected target Retinue
Node is now 9272 inline bytes, versus 3256 in the October 2 capacity receipt.
These are measured layout changes, not an attribution of all growth to one
feature. The 10064 combined layout includes retained instances, so it is not
directly interchangeable with the old primitive-only Residents structure.

Stack regions/gaps are linker space, **not measured unused stack at runtime**.
The older 61328-byte sampled V4 stack use is not imported as a current peak.
No PSRAM is credited. T114's linker-protected boot/durable regions remain
outside every measured load segment. V4 image generation uses `--skip-padding`
so whole-flash padding is not confused with application growth. No image here
was flashed and no installed board firmware or counter state changed.

GNU readelf independently agrees with every reported allocated section in all
four ELFs. `files.json` seals 21 initial public evidence files; later explicitly
named [standalone T114 follow-up](standalone-t114-followup.json) seals three
additional evidence files and corroborates its sections against GNU readelf.
That separate build avoids dev-dependency feature unification when measuring
the default production firmware; the additional capacity example is unchanged.
No ELF, flash image, cache or
private runtime tree is embedded in the committed receipt.

## Validation and remaining gate

Current host workload, both fixed-heap campaigns, both capacity target links,
actual board-image links, selected strict host Clippy, T114 example strict
cross-target Clippy, formatting, unsafe audit and validation inventory pass.
The inventory pass also registers five previously orphaned attended helper or
historical-source assets, without executing any of those physical helpers.
Existing T114 unused-assignment and V4 dead-code/linker warnings are preserved;
the whole board builds are not called warning-clean.

Host compiler: 1.99.0. ARM compiler: installed stable alias 1.97.1. Xtensa:
1.97.0-nightly, using the owner's existing family-plan ruling 8 bench override
`--ignore-rust-version`; the declared 1.97.1 floor is unchanged. Existing
repository target/build paths and the co-owned `mere-upstreams` Cargo cache are
reused. No isolated target/home/worktree was created. Generated small flash
images remain under ignored `validation/results/protocol-parity-memory` for the
physical continuation; ordinary reusable build outputs remain in `target`.

Next: select reviewed board reserve margins and run current-image on-device
heap/fragmentation and real task/interrupt stack measurements, then queue/driver
pressure and restart/fault cases. Full parity, unattended firmware acceptance
and phase 2 completion are not claimed by this checkpoint.
