"""AX.25 gate: stock RNS AX25KISSInterface and Retinue's AX.25 carrier (`ax25_peer`) on ptys
joined by a null-modem bridge, both as N0CALL-7.

Each ignores the other's TNC parameter and READY frames, so packets flow end to end. Judged
on stock's state: has_path, an ACTIVE link, a request answered and a Resource COMPLETE. The
bridge records both directions: the 16 bytes after `FEND 00` in Retinue's DATA frames must
equal stock's, which a 6-character callsign makes byte-identical despite stock's 0x20
padding (`AX25KISSInterface.py` 286-298).

    .venv/bin/python -u interop_ax25_kiss.py
"""

from __future__ import annotations

import tempfile
from pathlib import Path

from pty_bridge import Bridge, Retinue, exercise_stock, kiss_frames, open_pty, report, rns_config

CALLSIGN, SSID = "N0CALL", 7
HEADER = 16


def stock_ax25(port: str) -> str:
    return ("  [[ax25]]\n    type = AX25KISSInterface\n    enabled = yes\n"
            f"    port = {port}\n    speed = 115200\n"
            f"    callsign = {CALLSIGN}\n    ssid = {SSID}\n")


def headers(bridge: Bridge, direction: str) -> set[bytes]:
    stream = b"".join(data for _, name, data in bridge.log if name == direction)
    return {f[1:1 + HEADER] for f in kiss_frames(stream) if f[0] & 0x0F == 0 and len(f) > HEADER + 1}


def run() -> dict[str, bool]:
    import RNS

    stock_master, stock_port = open_pty()
    retinue_master, retinue_port = open_pty()
    bridge = Bridge(stock_master, retinue_master)  # a2b: stock to Retinue
    retinue = Retinue("ax25_peer", "serial", [
        retinue_port, "--callsign", CALLSIGN, "--ssid", str(SSID)])
    if retinue.wait_for(r"^ATTACHED", 300) is None:
        return {"retinue attached": False}
    config_dir = Path(tempfile.mkdtemp(prefix="retinue-ax25-"))
    rns_config(config_dir, stock_ax25(stock_port))
    RNS.Reticulum(configdir=str(config_dir))
    try:
        results = exercise_stock(retinue)
    finally:
        retinue.close()
    stock, ours = headers(bridge, "a2b"), headers(bridge, "b2a")
    print(f"  headers: stock {[h.hex() for h in stock]} retinue {[h.hex() for h in ours]}")
    results["one header each way, Retinue's equal to stock's"] = (
        len(stock) == 1 and ours == stock)
    return results


def main() -> int:
    import RNS

    code = report("AX25 KISS INTEROP", run())
    RNS.exit(code)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
