# First R7 setup attempt, 2026-10-16

At `df69fb4bdc1`, the stock image's three pinned package segments matched
and flashing completed, but the untouched CLI returned exit 1 during public
LongFast configuration. Its unrestricted diagnostics were intentionally not
retained, so the exact setup failure is unclassified. No desktop/native
scenario launched and no Sennet packet-ID interval was consumed (`next=1`
before and after). The custodian stopped. COM6's pinned Tulle image and exact
status/identity/region/channel restoration passed. COM7 only underwent the
guarded same-boot baseline snapshot/reapplication, without a resident launch.

The retry selects espflash's supported watchdog reset explicitly after stock
flashing and retains filtered public CLI error diagnostics. This is a new
setup trial, not a passing RF receipt or a proved explanation of this failure.
