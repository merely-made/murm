# Bounded firmware memory gate, 2026-10-17

**PASS for the named profile, source `a7107a7266fa42aa36cd33d1325c9a449dc39dcc`.**
Both boards complete sixteen fragmented allocation rounds and runtime
queue/failure/reconstruction stages. Original named firmware is restored.

| Final on-device measurement | V4 SB2 | T114 |
| --- | ---: | ---: |
| Heap capacity | 65536 | 49152 |
| Occupied heap peak | 18452 | 18452 |
| Minimum untouched boot-painted stack | 62140 | 18916 |
| Measured stack region | 127752 | 84820 |
| Expected oversize allocation refusals | 16 | 16 |
| Unexpected allocation refusals / leaked allocations | 0 / 0 | 0 / 0 |

Each profile requires at least 16384 untouched stack bytes and 25% free heap.
These are explicit runner arguments, not firmware defaults. The stack cushion
exceeds three of the roughly 5.2 KiB crypto-leaf frames seen in target
disassembly; the large protocol/runtime construction frames are exercised
directly. The heap cushion is 12/16 KiB, larger than the measured 9932-byte
retained protocol allocation above fragmentation guards. These are engineering
acceptance margins for this workload, not formal bounds on unexecuted paths.
Capacity or firmware-role expansion requires new qualification.

## What runs

The retained diagnostic has Retinue's 8 peers, 4 actions, 1 link, 4 routes and
1 KiB resource; Tucket's 8 contacts, 32 dedup entries and 2 pending maximum
texts; and Sennet's 8 directory entries, 64 private-seen entries, maximum PKI
texts and coexisting ACK/control work. It includes churn and fragmentation.
Synthetic identities never reach a radio or durable credential store.

Each round also constructs four real Tucket-only runtimes. Each fills eight
work slots, refuses the ninth and oversized ingress, rejects stale completions,
accounts for eight failed transmissions, then drops/reconstructs its state.
Complete runtime/frame values cross optimizer barriers. All allocations return
to the initial occupancy. ARM runs this stage after the protocol fixture's
temporary frame returns; V4 keeps the nested campaign. This is software
driver-failure accounting, not physical SX1262 failure injection.

ARM uses cortex-m-rt's reset-time `paint-stack`. The V4 bench has a synchronous
entry hook before executor construction, preserving HAL startup and painting
only below live SP with a spill margin. Its 64-byte bottom guard is excluded.
The bounded scan masks interrupts and never repaints a live stack. Executor
tasks and interrupts share the measured MSP/CPU0 stack; V4 idle reuses the main
stack, and neither CPU1 nor an independent radio thread starts in this profile.
Boot-painted write evidence is distinct from the retained SP samples.

## Findings and correction

The first painted campaigns exposed only 2432 bytes free on V4 and 2660 on
T114. SP samples had missed deeper use. Borrowing the resident runtime into its
service future instead of moving/copying it reduced V4's main poll frame from
about 97 KiB to 48 KiB and removed about 20 KiB of duplicated static future
storage. Its constructor now has a separate non-inlined boundary.

The T114 fixture used consuming builders that copied complete Retinue nodes.
In-place configuration preserves the exact capacities and workload. Sequential
pressure stages then avoid nesting a 42 KiB runtime frame under the 30 KiB
protocol fixture after that fixture's objects have already been dropped.
The intermediate 8716-byte result remained a failure against the selected
reserve. Final hardened, clean-source images were independently rerun.

Earlier build-hook failures, pilot measurements, the V4 harness's mistaken
zero-based refusal expectation after an earlier pilot, and failed bulk flash
readbacks remain sealed as retained local inputs. They are not counted as
successful acceptance runs. Prior cited receipts are unchanged.

## Restoration and limits

SB2 returns to `c8e5eb9309e`, SB2/modem/US915, slot B sequence 13. Its fresh
1 MiB prefix matches the ROM-computed MD5; the final protected 64 KiB tail is
compared byte for byte. Failed bulk prefix transfers are not promoted to
successful byte comparisons. Two final read-only sessions match the original
status and preferences. T114 returns to named `cb042329232`, modem/US915,
slot A sequence 94, public node prefix `599997c8`. Its application-only DFU,
status and identity checks are recorded; protected flash was not read back.
The intentional watchdog DFU bridge changes boot/crash residue. SB1 is untouched.
No manual replug was required during this continuation.

Production images also link separately and their layouts are recorded, with
independent LLVM/GNU section reports. Nineteen runtime tests, fragmented host
48/64 KiB campaigns, host strict Clippy, board Clippy with recorded existing
allowances, unsafe audit and inventory pass. Five pre-existing LOC violations
remain; the resident constructor extraction removes its old ceiling exemption.

This closes the bounded memory gate. Physical driver failure, power cuts,
real-time service deadlines, unattended operation and simultaneous network
coverage remain product acceptance work. PSRAM is not credited. T114 does not
gain a production three-protocol radio runtime from this diagnostic.

`qualification.json` binds source, images, limits and measurements; `files.json`
seals public evidence. Ignored images/private backups and exploratory inputs
are receipt-owned under `validation/results/firmware-memory`; their byte ledger
is `retained-inputs.json`, within the continuation's shared 100 MB cap. Existing
shared Cargo caches are reused. No isolated target, Cargo home or worktree was
created, and private state/image bytes are not published.
