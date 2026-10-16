# SB2 reboot with a direct PC connection, 2026-10-16

Result: **failure reproduced on the first reboot**. Three cycles were requested;
zero completed, and the runner stopped at the first failed USB recovery.

The owner moved SB2 from the USB hub/cooling stand to a direct PC USB connection.
Windows' captured parent chain also places the board's composite device
directly below the PC's USB root hub and AMD host controller, with no external
hub in that path. The runner identified the board by
VID/PID/USB serial, then verified its SB2 name, US915 region, modem channel,
firmware revision `cb042329232` and loaded slot A sequence 6 before mutation.

The existing `channel modem` command rewrites the current settings through the
verified A/B journal and calls the same software reset used after a name save.
It preserves the logical channel, region, name and identity. Each attempted
cycle makes one settings write; this is not a hardware power-cycle test.
RTS/DTR were false, as in the earlier commissioning campaign. The host closed
the serial handle after the complete `channel set; rebooting` reply, waited
six seconds and reopened the identified carrier.

The first reboot was acknowledged in 0.105 seconds. After reopening COM7,
`status` returned no bytes during 1.601 seconds, then `name` raised
`SerialTimeoutException('Write timeout')` after 2.005 seconds. The identified
COM7 carrier remained enumerated. `run.json` preserves the failed result and
every session/reply; it must not be counted as a passing reconnect test.

This proves that the failure can occur without the external hub. It does not
identify the root cause or rule out cable/connector, board hardware, firmware
startup, host handle timing or Windows USB-driver state. Firmware and mode were
not changed, and no firmware was flashed. No known-good cable substitution or
matched-mode SB1 comparison is claimed. The full stored identity is not exposed
by these probes; verification is the existing journal load/status boundary.

The owner unplugged/reconnected SB2 and communication recovered. The separate
read-only `recovery.json` passes across two serial sessions, with SB2, US915 and
modem unchanged and loaded slot B sequence 7: exactly one journal increment
from the baseline. Only read-only queries were sent during recovery. No further
reset was issued. Recovery does not turn the reboot campaign into a pass.

Runner: `testing/usb_reboot.py`, SHA-256 recorded in `run.json`.
Command: `python -B testing/usb_reboot.py --connection direct-pc --rounds 3
--output validation/results/device-names/sb2-direct-reboots.json`.
`files.json` seals this receipt's public bytes. Raw results remain ignored under
`validation/results/device-names`; no Cargo build, isolated target/home or
worktree was created.
