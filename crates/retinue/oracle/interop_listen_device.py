"""Do retinue listeners bound by device and by host name reach stock RNS (C7)?

`listen_device_peer` listens on the loopback device preferring IPv6, as stock's `device` and
`prefer_ipv6` keys bind (`TCPInterface.py` 469-501), and on `localhost` preferring IPv4, as
a hostname `listen_ip` resolves. A stock TCPClientInterface dials each, at ::1 and 127.0.0.1;
each stock node must learn a path to retinue.

Run from the oracle/ directory:  ./.venv/bin/python -u interop_listen_device.py
"""
from __future__ import annotations

import sys

from ingress_harness import Child, Stock, verdict, wait_until
from interop_pipe import build

DEVICE = "lo0" if sys.platform == "darwin" else "lo"


def client(host: str, port: str) -> str:
    return ("  [[retinue]]\n    type = TCPClientInterface\n    enabled = yes\n"
            f"    target_host = {host}\n    target_port = {port}\n")


def main() -> int:
    peer = Child([str(build("listen_device_peer", "auto")), DEVICE], "retinue")
    device = peer.wait_for(r"LISTENING device \[(::1)\]:(\d+)", 30)
    host = peer.wait_for(r"LISTENING host (127\.0\.0\.1):(\d+)", 10)
    dest = peer.wait_for(r"DEST ([0-9a-f]{32})", 10)
    checks = [verdict(f"{DEVICE} with prefer_ipv6 binds ::1", device is not None),
              verdict("localhost binds 127.0.0.1", host is not None)]
    if device and host and dest:
        for label, bound in (("device", device), ("host", host)):
            stock = Stock(label, False, client(bound.group(1), bound.group(2)))
            checks.append(verdict(f"stock via {bound.group(1)} has_path(retinue)", bool(wait_until(
                lambda: stock.call("has_path", dest=dest.group(1)), 30, 0.5))))
            stock.kill()
    ok = len(checks) == 4 and all(checks)
    print(f"LISTEN DEVICE INTEROP: {'PASS' if ok else 'FAIL'}", flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
