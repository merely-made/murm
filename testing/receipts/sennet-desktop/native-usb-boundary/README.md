# Native launch refused at the USB client boundary

At `d4a91ab431a`, the selected stock public profile readback passed. The
native runner then stopped when its stock helper observed "Meshtastic rebooted
during handshake" after the Python client released USB. No exchange or
desktop journal was recorded; packet-ID state remained `next=1`. The runner
stopped its processes and isolated custodian. Both Tulle restorations passed.

The next wrapper requires a successful independent Signalman read-only probe
after configuration as well as before it. Only that setup probe can be retried
once after this specific reboot refusal. No RF operation is replayed and the
production actor's refusal remains unchanged. This is an observed client
transition, not evidence of a successful native/RF exchange or a proven cause
inside the stock firmware.
