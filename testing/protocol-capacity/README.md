# Protocol capacity probe

**Current continuation, 2026-10-16:** the legacy workload and `PROTOCOL_LAYOUT_V1`
remain available. `--current` additionally retains actual Sennet/Tucket instances
with PKI, a full 64-entry private seen window and outstanding ACK work. The
adjacent `receipt.json` is the historical October 2 receipt; the new
[memory refresh](../receipts/protocol-parity-memory/README.md) records current
sources, host allocator campaigns and both target images separately.

This unpublished validation crate compiles Retinue, Sennet and Tucket together
with `no_std + alloc`. It is not a protocol switching runtime.

The fixture retains one Retinue Node (8 peers, 4 actions, 1 link, 4 routes,
2 freshness blobs per peer, 255-byte ingress, 32-byte local announce data,
160-byte link payload, 1,024-byte outbound resource and 8 inbound parts), one
Sennet channel/packet allocator (8 directory rows, 32 dedup identities), and one
Tucket Node (8 contacts, 32 dedup hashes, 2 pending texts of at most 171 bytes).
Capacities are explicit fixture selections, not imposed firmware settings.

It fills directories, churns duplicate history, encodes/decodes Sennet texts,
retains Tucket routes and attempted texts, establishes a Retinue link, receives
a 1 KiB resource and leaves an outbound resource pending. Synthetic peer objects
are included in the allocation peak. Fixture identities and repeated keys must
never be used in deployed firmware.

```powershell
cargo run -p protocol-capacity-probe --features host --locked --offline
cargo +esp rustc -p protocol-capacity-probe --lib --release --locked --offline --target xtensa-esp32s3-none-elf '-Zbuild-std=core,alloc' -- --emit=llvm-ir
cargo +esp build -p tulle-heltec-v4-phy --example protocol_capacity --features resident-protocols --release --locked --offline --target xtensa-esp32s3-none-elf '-Zbuild-std=core,alloc'
```

The host executable reports **requested allocation bytes** through a System
allocator wrapper. It excludes allocator metadata, size-class rounding and
stack. Its peak is one exercised workload, not a worst-case theorem or an
Xtensa measurement. All workload allocations must be released on drop.

The target LLVM symbol `PROTOCOL_LAYOUT_V1` lists pointer size, Retinue Node,
Actions and InterruptionReport, Sennet Channel, PacketIdState, ManagedFlood,
NodeDirectory, Tucket Node, PendingText, the complete Residents structure, and
PendingTexts, in that order. These sizes include inline state, not heap storage.

The V4 example is **build-only**, with a 64 KiB LLFF heap, 64 KiB radio/runtime
BSS reservation and 16 KiB queue/state BSS reservation. Its main links the
workload but validation does not flash or execute it. Reservations demonstrate
linker space for a candidate budget; actual runtime, allocator fragmentation,
target heap/stack high-water and adverse traffic remain for board integration.
The shipping direct-PHY firmware remains a separate image.

Use the protocol-owned checked input paths and retain queues within their
declared bounds. General low-level allocation helpers and caller-owned vectors
do not acquire a global budget merely by linking this crate. Compression is off
in the target graph; incoming decompression needs a separate expansion limit.

The adjacent `receipt.json` records the exercised source hashes, host results,
target layouts, linked sections and scope. It is working-tree evidence, not a
clean-revision release receipt.

## Current retained workload

`current::workload` retains the existing full Retinue tables and link/resource
state, then moves the full Tucket contact table into its retained instance with
two outstanding 171-byte texts. It fills eight Sennet directory rows, churns
256 authenticated maximum-size private texts through a 64-entry private seen
window, verifies fresh ACKs for duplicates, and leaves an outgoing ACK window
alongside a queued control ACK. Tampered ciphertext and a busy send are refused.
There is no RF, secret release, commissioning or durable counter write. All
fixed seeds and nonce values are synthetic fixtures and cannot be deployed.

`PROTOCOL_LAYOUT_V2` reports pointer size, Retinue Node, Sennet instance, Tucket
instance, the combined retained structure, and radio-hand's actual Runtime.
The last layout includes its work/report queues; those queues are not executed
by this fixture. Heap storage is excluded from every layout size.

```powershell
cargo run -p protocol-capacity-probe --features host --bin protocol-capacity-probe --locked --offline -- --current
cargo run -p protocol-capacity-probe --features fixed-heap --bin protocol-capacity-fixed-48 --locked --offline
cargo run -p protocol-capacity-probe --features fixed-heap --bin protocol-capacity-fixed-64 --locked --offline
cargo +stable build -p tulle-t114-phy --example protocol_capacity --release --locked --offline --target thumbv7em-none-eabihf
```

The two fixed-heap host executables use the firmware's LLFF allocator at 48/64
KiB. They leave 16 of 32 alternating 257-byte guard allocations alive, execute
the current workload sixteen times, require every round to return to its
fragmented baseline, and check one deliberate oversized allocation refusal.
Bootstrap allocations use System before activation and are routed back to
System by pointer range. These are host ABI/allocator measurements, not target
runtime measurements; no target stack or firmware power-cut result follows.

The V4 and T114 `protocol_capacity` examples both use the current workload. They
reserve 64 KiB for radio/runtime and 16 KiB for queues in BSS alongside the real
retained objects and fixed heap. These placeholders deliberately do not claim
to represent either complete board runtime. Neither capacity image runs USB,
radio traffic or a schedule. The ordinary T114 firmware and V4 resident image
have separate whole-image linker reports.

The installed Xtensa compiler is 1.97.0-nightly. Family-plan ruling 8 explicitly
permits `--ignore-rust-version` for bench images while this compiler remains
below the owned 1.97.1 floor. That exception is recorded; the T114 measurements
use the installed stable compiler 1.97.1 without an override. Reproduction uses
explicit existing `CARGO_HOME`, `CARGO_TARGET_DIR` and `CARGO_BUILD_BUILD_DIR` as
recorded in the receipt; no new target directory or isolated Cargo home is needed.

`measure_elf.py ELF --output NEW.json` reads target layouts, allocated sections,
load segments and linker stack bounds without executing or installing the ELF.
It refuses an existing output path. ARM load segments must remain inside the
current code region, clear of durable storage. Linker gaps are not measured
stack high-water. Current output is cross-checked against GNU readelf sections.

The [physical follow-up](../receipts/protocol-parity-memory/physical/README.md)
uses `target_round::run` from a separate firmware task. The fixed-heap host bins
accept `--target-rounds` to repeat that same fragmentation/refusal function sixteen
times. Refusal calls the allocator directly because a global-shim allocation/free
pair can be removed by LLVM. Named stack/supervisor checkpoints allocate nothing.

`board_probe.py BOARD --port PORT --output NEW.json --rounds 16` requires an
already installed, explicitly selected bench image and an exact known USB
identity. For the second V4 use `--usb-serial 44:1B:F6:6A:FB:28`; the default is
the FA:64 V4. Zero rounds performs read-only firmware/profile checks. Flashing,
private backups and restoration are separate attended actions recorded in the
physical receipt. This command does not transmit synthetic identities or alter
persisted radio settings.
