# T114 live Tucket endpoint, 2026-10-20

**Passed on the instrumented opt-in image `9de106432ad`.** Four exact texts and
four protocol ACKs pass against stock MeshCore 1.17.1 `d929643` on SB1, across
two T114 boots. The second connection restores one saved contact before new
peer work; its entire transcript contains no decoded advert. Both original
named firmware images and radio settings are restored, and owned helpers stop.
See [qualification](qualification.json), [radio result](radio/radio.json),
[first exchange](radio/first/receipt.json) and [reboot exchange](radio/cold/receipt.json).

The `tucket-endpoint` feature starts in the recovery modem until volatile USB
commissioning, then serves Tucket alone. Its eight contacts, 32 dedup entries,
two pending messages and eight queued frames retain their original bounds.
It accepts status, text and advert commands; other protocol/role commands refuse.
The small endpoint avoids retaining unused Retinue/Sennet storage. Construction
runs on a separate executor turn, with no radio/flash capability. The sole radio
owner drains completed RX, enters standby and settles IRQs for a cache write,
then restores RX before protocol TX. This is an explicit listening blackout.
Idle polling emits no empty USB diagnostics.

## Memory

The selected limits remain **16384 untouched stack bytes and 25% free heap**.
The same image passes sixteen fragmented pressure rounds, including the prior
full protocol workload, its exact 3408-byte live-state allocator charge, and
four endpoint overflow/stale-completion/driver-failure/reconstruction campaigns
per round. Stack paint includes the shared MSP's main, worker and IRQ activity.
It is a bounded workload receipt, rather than a formal worst-case stack proof.

| Workload | Minimum untouched stack | Peak occupied heap |
|---|---:|---:|
| Sixteen pressure rounds | 75820 / 139420 bytes | 21860 / 49152 bytes |
| First live exchange | 120308 / 139420 bytes | 6388 / 49152 bytes |
| Exchange after reboot | 124620 / 139420 bytes | 6404 / 49152 bytes |

Pressure leaves **55.5% heap free** at peak, releases all allocations each round,
and records exactly sixteen deliberate oversize refusals. Actual construction,
contact writes and RF traffic produce zero allocation failures. Raw results are
in [the board campaign](quiet-pressure/board.json). Production and instrumented
images link; the production image is build-qualified only. Cargo 1.99.1 drives
the existing Rust 1.97.1 ARM compiler because older Cargo rejects manifests in
the recently updated, unrelated foundation patch graph. Image hashes and target
sections are in [quiet-images](quiet-images/images.json).

## Custody and preservation

An isolated real Keeper wallet supplies station-scoped material through Djinn.
The original Tucket-only helper and clean probe match their previously qualified
binary hashes; [all six custody controls](custody-controls/qualification.json)
pass again, including wrong-app refusal and lock/resident-loss closure. The
current desktop probe consumes that supplier and honors the public DTR setting.
Wallets, USB setup bytes and raw flash remain excluded from this public receipt.

The old eight-key synthetic storage fixture is cleared by an explicit pre-radio
handoff image only after its exact scope, keys and sequence match. Its
[readback](fixture-handoff/receipt.json) records two page erases and zero writes,
limited to `0xe4000..0xe6000`. Other scopes, sequences and corruption refuse in
software tests. This handoff runs once, before both accepted exchanges.

SB1's original 1 MiB prefix and entire 64 KiB tail compare exactly with the
fresh preflight: [restoration](radio/sb1-live-restoration.json). T114 restores
original application `cb042329232`, name T114, identity `599997c8`, settings A/94,
US915 and modem mode: [live readback](radio/dut-restoration/board.json). T114 has
no protected-flash dump claim. The new dedicated pair retains the bench public
contact. Batteries remain connected; the reboot phase is a software reset.

Eighty-four Tucket and 180 radio-hand tests, strict core Clippy, owned formatting,
schema classification, unsafe audit and inventory self-tests pass. The existing
application LOC failures remain: companion.rs 781 lines and desktop main.rs 636.
Source hashes are in [the source ledger](source-sha256.json); frozen orchestration
is in `source/`, with the maintained board helper at `testing/tucket_t114_ops.py`.
The retained input/receipt budget remains below 100 MB, across the prior lanes
and this continuation; [the byte ledger](retained-bytes.json) records the scope.

Private setup remains volatile and must be supplied after boot. Installed
personal-daemon integration, UI, power cuts, physical driver-fault/deadline
controls, multiple-hop routing, repeater/server roles and full parity remain open.
