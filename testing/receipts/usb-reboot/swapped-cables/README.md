# SB2 reboot after swapping the V4 cables, 2026-10-16

Result: **the same failure reproduces on the first reboot**. Three cycles
were requested; zero completed, and no further reset was issued after failure.

The owner exchanged SB1/SB2's USB cables. Windows' captured parent chains put
both identified devices directly below the PC's USB root hub and host
controller, with no external hub in either captured path. The same
`testing/usb_reboot.py` runner, firmware `cb042329232`, modem personality,
US915 region, false RTS/DTR, six-second reopen delay and save-current-channel
reset method were used as in the earlier direct-PC control.

Baseline probes pass with name SB2 and loaded slot B sequence 7. The board
acknowledges `channel set; rebooting`. On the reopened COM7 carrier, status
returns no bytes and the next write times out. The carrier remains enumerated.
The failed `run.json` is retained as a failure independently of any later
manual recovery. This makes a defect confined to SB2's original cable less
likely. It does not isolate the board, firmware or Windows driver, and SB1
has not been tested with the same modem personality or reboot sequence.

## Red light and battery observation

The owner reports a red light inside SB2's housing, initially described as
probably steady, and confirms batteries are connected. The
[Heltec V4.2 schematic](https://resource.heltec.cn/download/WiFi_LoRa_32_V4/Schematic/WiFi_LoRa_32_V4.2.pdf)
shows red LED2 between VIN_Charge and the CN3165 charger's CHRG output. The
white LED3 is the GPIO-controlled firmware indicator. Identifying the observed
red light as the charge indicator is an inference from that schematic; its
physical position, battery voltage, current and charge state were not measured.
It supplies no established explanation for the USB timeout.

With connected batteries, unplugging USB does not establish that the board
lost power. Earlier USB-replug recoveries must not be promoted into cold-boot
or power-cut evidence. A lit display alone also does not prove continued CPU
execution: its existing frame may remain visible.

During USB disconnection with batteries connected, the owner reports that
nothing changed and the on-screen timer kept ticking. This is human-observed
CPU/UI progress while USB was disconnected, stronger than a merely persistent
frame. It supports continued battery-powered execution during that interval;
it does not prove that every application/radio task remained healthy. The
subsequent read-only USB recovery is recorded separately from the failed
reboot campaign.

Command: `python -B testing/usb_reboot.py --connection direct-pc --rounds 3
--output validation/results/device-names/sb2-swapped-cables-reboots.json`.
The runner SHA-256 is recorded in `run.json`; `files.json` seals this receipt.
No firmware was flashed or changed. Raw probes and the locally inspected
schematic/render remain ignored, receipt-owned artifacts under
`validation/results/device-names`. No Cargo build, isolated target/home or
worktree was created.


USB replies recover after the manual reattach. The separate read-only recovery
passes across two serial sessions with SB2/modem/US915 unchanged, now loaded
slot A sequence 8: exactly one save from the sequence-7 baseline. No further
reset was issued. This narrows investigation toward USB peripheral/firmware/
host session recovery; it does not identify which layer is responsible.
