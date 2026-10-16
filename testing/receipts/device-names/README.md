# Persistent public device names, 2026-10-16

Firmware source: `cb042329232571cd3c50b2ef9b5e386fbe4f434a`.
The owner requested visible board names because two unmarked V4s cannot be
identified physically by Windows COM port or USB serial.

The local label appears as the boot and default status title and in USB
`status` / `name`. `name set SB1` saves through the existing verified A/B
settings journal and reboots; `name clear` clears it. Names are at most sixteen
printable ASCII characters, with no leading/trailing spaces. Invalid commands
and unchanged names do not write. The label is independent of host identity,
relay identity, RF profile names and radio addresses.

## Installation checkpoint

| Label | Port at qualification | Saved | Post-reboot USB readback |
| --- | --- | --- | --- |
| SB1 | COM6 | yes, sequence 11 | pass |
| SB2 | COM7 | yes, sequence 6 | pass after physical reconnect |
| T114 | COM10 | yes, sequence 94 | pass |

SB2's first physical reconnect restored USB and allowed its name save.
The subsequent software reboot stalled the endpoint again. A second physical
reconnect used the now-visible SB2 label. The follow-up passes exclusively
read-only queries across two USB sessions, without another reboot. All three
names now reload from the journal, with original region/channel settings.

## Evidence boundaries

`qualification.json` records the installation outcome, image/source hashes and
the separate physical screen-observation boundary. Per-device provisioning
records contain the actual commands, replies, reconnect sessions and errors.
Software rendering checks prove the full sixteen-character title fits both
128x64 and 240x135 surfaces. USB `ui` reports an active status screen; this
does not photograph either physical display.

The shared gates pass: twenty radio-face tests, 149 radio-hand tests, formatting,
strict shared Clippy, both normal release links, target Clippy with existing
allowances, flash classification, unsafe audit and validation inventory.
The same six pre-existing LOC violations remain, with no new ones. Target lint
allowances are listed in `commands.json`; these checks do not claim an
unqualified warning-free firmware graph.

The first SB1 install exposed an 85-byte settings body that made the board's
derived NorFlash read buffers unaligned. Reads failed before identity creation
or a provisioning write. A raw comparison of the entire protected 64 KiB tail
matched the original exactly. The corrected body is padded to 88 bytes, while
the original 68-byte prefix and the name offsets remain unchanged. The failed
record remains a failure in `sb1-provision.json`; the corrected campaign is
`sb1-aligned-provision.json`.

SB2's first corrected-image probe hit the existing Windows USB data-endpoint
timeout after flashing. Its failed record remains `sb2-provision.json`.
The user physically reconnected the V4 other than the one displaying SB1;
the subsequent provisioning campaign has its own record.

V4 updates wrote only the application at `0x10000`, below the protected state
at `0x3f0000`. T114 used the qualified small watchdog handoff before its
application-only serial DFU; the new erase span ends at `0x7f000`, below the
control/reservation/settings areas beginning at `0xe6000`. The DFU package
payload matches the raw binary plus word padding. Bootloaders and partitions
were not flashed. Original recovery artifacts were verified before updates.
The named firmware is deliberately retained on the boards.

Journal sequences and the T114 public node prefix are observed in USB records.
There is no raw protected-state readback after the intentional label writes,
and no new RF interoperability, stack high-water or memory-reserve claim.

`files.json` seals the public receipt bytes. Images, the private protected-state
comparison and original recovery material remain ignored under
`validation/results/device-names` and the prior physical-memory artifact home,
owned by these receipts. Existing shared Cargo target/home were reused; no
isolated target, Cargo home or worktree was created. Owned flash and probe
processes are stopped at the checkpoint.


Before a later stock-firmware excursion, take fresh state backups and update
guarded fixture image hashes to this named baseline. Older frozen restoration
inputs intentionally describe the earlier unnamed firmware/settings and would
undo the labels if used as a new baseline without refresh.
