# Standalone Tucket public contact storage, 2026-10-17

The explicit V4 `resident-contact-store` feature saves eight full public peer
keys through the radio owner's exclusive quiet window. Scope and keys are
validated before reserving packet IDs or announce time, then installed in a
fresh runtime before protocol service. The local public
identity and exact receive profile select the cache; transmit power does not
change its scope. Volatile identity/setup must still be supplied after boot.
This stores peer associations, not credentials, commissioning, labels or trust.
The default relay retains host-only contacts.

Dedicated A/B erase pages occupy V4 `0x3fa000..0x3fc000` after the existing
packet-ID lease pair, and T114 `0xe4000..0xe6000` before the unchanged control,
reservation and settings pairs. T114's application linker window ends at the
new pair. Its adapter and pre-radio fixture qualify storage only: there is no
T114 Tucket service loop in this slice.

The maximum record is 344 bytes. The existing CRC-protected journal wraps a
versioned `TKC0` payload; the commit word is programmed last. Interrupted writes
retain the previous committed record, and an interrupted initial body can be
retried if neither commit word exists. Committed future formats, malformed keys,
scope mismatch, overflow, conflicting sequences and exhaustion refuse before
erasure. A readback failure after complete programming is an uncertain commit.
Unchanged keys do not write. Routes, deduplication, clocks and pending work are
rebuilt by their existing owners.

Seven journal tests cover 68 first/later-write word interruptions, cold reload,
refusals, unchanged snapshots and readback uncertainty. The complete radio-hand
gate passes 150 library tests, 19 instance tests and seven journal tests, alongside
84 Tucket tests. Strict
core/host-probe Clippy, firmware links, formatting, flash classification and the
unsafe inventory pass. The two existing application LOC failures are retained.
The optional journal suite is registered for normal PR validation.

The initial source allocation overlapped the existing packet-ID leases. The
prepare guard caught nonblank state before any firmware write; the next source
derives its address from the existing lease pair. The first V4 campaign then
saved all eight keys but its memory runner selected the other USB identity and
refused. The explicit-identity retry passed sixteen rounds and two reset cycles
with eight recovered keys, zero new commits and an unchanged cache checksum.
Its minimum untouched stack was 61,468 bytes and peak heap 18,452 of 65,536.

T114's first flash campaign saved all eight keys but measured only 13,892 bytes
of untouched stack, below the agreed 16,384-byte reserve. Moving the boot fixture
to a separate executor turn avoids nesting it under the large main poll frame;
both turns still share the same boot-painted stack. The repaired campaign passes
all sixteen rounds with 18,836 bytes untouched and peak heap 18,452 of 49,152.
An application-only reinstall recovers eight keys with zero erases/writes and
the same sequence. Original firmware, live identity and radio profile are
restored; only intentional watchdog handoff reset/crash residue is normalized.
This does not claim a byte-for-byte protected-flash dump on T114.

The first live V4 trial refused setup before changing counters. USB wake-byte
stripping had ignored a partly received resident setup frame and could remove
zero payload bytes at a chunk boundary. A regression fails before the fix and
passes afterward for every chunk width from 1 to 64. The latest firmware
preserves those bytes and reports a public failure reason if setup is refused.

Later setup attempts reached durable reservations but stalled in construction.
The USB poll frame (about 48 KiB), synchronous constructor (about 73 KiB), and
`Runtime::new` frame (about 20 KiB) exceeded the roughly 127 KiB stack region.
The separate fixture had not exercised that nested call path. Construction now
runs on another turn of the same executor and boot-painted stack. The radio
owner keeps flash access and its cancel-safe quiet guard; only owned volatile
setup, verified leases and validated public keys cross to the pure worker.
The returned runtime is boxed, removing large copies from the USB task.
The pressure fixture separately charges the exact boxed-state layout while
exercising the unchanged maximum protocol workload. It drops that allocation
before reporting final occupancy; this is storage accounting, not a second
concurrent protocol instance.

The first live exchanges saved the contact and received outgoing protocol ACKs,
but the stock return text was absent. The resident instance retried every second,
overlapping the response interval at this PHY. `InstanceConfig::for_radio` now
derives a retry interval from two caller-selected transmission budgets plus
turnaround, with checked arithmetic. This profile uses 4200 ms. A delayed-ACK
test confirms that the earlier one-second point issues no retry and that the
later matching ACK completes the operation. Existing explicit-interval callers
retain their chosen settings.

The final V4 trial at `fb7e0e6eacc` repeats the `21ac0176a22` pass: four
tagged texts and four protocol ACKs across two
boots. Before any new peer work after reset, the same identity has one installed
contact, with RX/TX counts both zero. The fixture sends no new stock-peer advert
on that boot. Both full tables recover unchanged, with no cache writes on
unchanged snapshots. The following measurements use explicit 16384-byte stack
and 25% free-heap acceptance limits:

| Tested profile | Minimum untouched stack | Peak occupied heap |
| --- | ---: | ---: |
| V4 live construction and both exchanges | 51792 | 21980 / 65536 |
| V4 full pressure with 20600-byte boxed-state charge | 101812 | 39052 / 65536 |
| T114 storage and full pressure | 18836 | 18452 / 49152 |

Both pressure campaigns pass sixteen rounds and return occupancy to zero.
Production V4 and T114 storage profiles also link. These are bounded workload
measurements, not formal stack proofs or qualification of a T114 live service.
Word-cut tests cover one- and two-key record writes; physical full eight-key
reload is separate. A battery-disconnected power cut is not qualified.

Both V4 original application prefixes are exact after restoration. Their immutable
tail ranges are exact; SB2's packet and announce ceilings advance without rollback.
The new cache pairs retain generated public test keys. Between independent bench
identities, only SB2's newly created cache pair was erased after matching its
exact scope, keys and raw preimage; full-tail comparisons protect every other
byte. The first post-check refused a valid ROM MD5 that omitted its leading zero;
a separate read-only verification passes. These resets are outside the passing
reboot exchange and do not implement a production cache-clear command.

The stock MeshCore 1.17.1 application runs under SB1's unchanged factory partition
table without SPIFFS. This qualifies wire exchange, not stock filesystem/reboot
persistence. T114 returns to its original identity/profile. No manual reconnect
is required; all owned peer and custodian processes stop. Multiple-hop/scoped
routing, broader commands/roles, desktop UI and unattended/driver-deadline
acceptance remain separate work.

The [qualification ledger](qualification.json) binds each source cohort and its
physical outcomes; [source hashes](source-sha256.json) and [file hashes](files.json)
bind the exact inputs. Failed attempts remain separate from passing results.
Frozen attended scripts in `source/` consume their historical private-input
layout and require the identified hardware; they are not unattended installers.

Private recovery flash images, raw protected-state comparisons and stopped
isolated Keeper wallets remain ignored under
`validation/results/tucket-firmware-contacts`, owned by this receipt. Public
receipts contain hashes, status and public keys only. Existing shared build
caches were reused; no isolated target, Cargo home or worktree was created.
