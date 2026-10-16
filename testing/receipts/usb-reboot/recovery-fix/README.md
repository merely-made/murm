# SB2 USB startup recovery, 2026-10-16

**PASS: five consecutive save/software-reboot cycles recover without manual
USB reattachment.** SB2 now runs firmware `c8e5eb9309e12237d149d6bd1d8eb89ea6ec4a8f`,
image `v4-modem-usb`, on COM7, direct PC USB, modem/US915. Batteries remain
connected. The unchanged runner waits six seconds before reopening, with RTS
and DTR false. Settings advance from slot A sequence 8 to slot B sequence 13.
Every cycle preserves the name, channel, region and firmware stamp.

## Cause and correction

SB2's control and pending journals are erased. Its boot therefore enters the
20-second first-owner physical-claim observation window before ordinary host
reads. SB1 has existing control state and skips this window. An early host
query could fill the unread USB RX path and time out on a subsequent write.
The firmware was still progressing; previous replug observations did not
separate USB attachment from elapsed time.

The first candidate, `a7c4f67da39`, removes unsolicited USB boot output so TX
cannot block before the first read. UART retains its announcements. That
candidate still fails the early baseline probe, before any test reboot.
A later read succeeds at the same sequence 8 without physical reattachment.
Both observations are retained, rather than counting the later pass as a
successful early recovery.

The complete fix races a bounded 64-byte RX prefetch against the existing
gesture observer. Only when both journals already permit ordinary service
can a complete read-only query select it early. Fragmented prefixes accumulate;
received bytes are replayed exactly once. Mutations, binary/unknown traffic
and incomplete queries remain available to the physical carrier or the normal
parser after timeout. Pending/corrupt journals cannot select ordinary service.
Host bytes create neither physical presence nor owner authority. A physical
claim still requires release, a fresh press and a three-second hold within
the original 20-second window. Commissioning clients should send first-owner
traffic before ordinary modem queries on that boot.

## Stored-state and verification evidence

Both app-only installs leave the 64 KiB protected tail byte-identical to the
fresh pre-install backup. After the five expected settings saves, valid
CRC-checked active settings bodies still match exactly, including identity
and preferences. All protected bytes outside the settings A/B pair also
match exactly. Raw private bytes remain ignored; `state-comparison.json`
publishes only metadata, hashes and comparison results.

A final ROM flash read followed by two read-only USB sessions passes at
sequence 13 without a manual replug. The first candidate's immediate ROM
read had an enumeration failure; its successful retry is recorded separately.

Software evidence: 115 Tulle tests, 37 control tests, 26 first-owner contract
tests and nine host-handshake tests pass. Five tests execute the actual board
prefetch adapter, including cancellation/replay and denied eligibility.
Formatting, strict adapter Clippy, board Clippy with existing allowances,
the USB release link, UART compile check, flash classification, validation
inventory and unchanged unsafe audit pass. The LOC gate still reports five
pre-existing violations; main and control_boot fit their existing baselines.
No new physical first-owner, on-air, power-cut or memory high-water acceptance
is claimed.

`qualification.json` binds source/image/tool hashes and commands; `files.json`
seals this receipt. SB1 and T114 retain named firmware `cb042329232` and were
not flashed here. Refresh current-state backups before future stock excursions.
Existing shared Cargo caches were reused, with no isolated target, home or
worktree created. Ignored images and private comparisons under
`validation/results/usb-recovery` remain owned by this receipt and recovery.
