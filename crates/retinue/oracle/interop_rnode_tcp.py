"""RNode over TCP without hardware: stock RNodeInterface (`port = tcp://127.0.0.1`) and
Retinue's `rnode_tcp_peer` each reach a fake RNode on `rnode_air`, served on loopback TCP.

Stock's `TCPConnection.TARGET_PORT` (7633) is patched in this process to its device's free
port, and Retinue is given its own with `--tcp-port`, so lanes can run in parallel.

Judged on stock's state and the air's byte log:

- has_path and an ACTIVE link across the air;
- after an idle stretch of 12 s, the stock interface is still online, the link still ACTIVE
  and a request still answered; neither host reconnected, and Retinue's writes were never
  more than 3.5 s apart (its DETECT keepalives) while idle;
- the telemetry Retinue parsed from the air's fixed CHTM, PHYPRM, CSMA, BAT and TEMP reports
  equals stock's `get_interface_stats()` and the interface's own `r_*` values.

    .venv/bin/python -u interop_rnode_tcp.py
"""

from __future__ import annotations

import json
import tempfile
import time
from pathlib import Path

from pty_bridge import REQUEST_SEED, Retinue, payload, report, rns_config
from rnode_air_gate import DETECT, Air, stock_rnode, wait_until

STOCK, RETINUE = 0, 1
IDLE = 12.0
# ACTIVITY_KEEPALIVE (`RNodeInterface.py` 1458), plus scheduling slack.
KEEPALIVE, SLACK = 3.5, 0.35
STATS_KEYS = ("airtime_short", "airtime_long", "channel_load_short", "channel_load_long",
              "noise_floor", "interference", "cpu_temp", "battery_state", "battery_percent",
              "bitrate")
ATTR_KEYS = ("r_current_rssi", "r_symbol_time_ms", "r_symbol_rate", "r_preamble_symbols",
             "r_premable_time_ms", "r_csma_slot_time_ms", "r_csma_difs_ms", "r_csma_cw_band",
             "r_csma_cw_min", "r_csma_cw_max")


def start_stock(tcp_port: int):
    import RNS
    import RNS.Interfaces.RNodeInterface as rnode

    rnode.TCPConnection.TARGET_PORT = tcp_port
    config_dir = Path(tempfile.mkdtemp(prefix="retinue-rnode-tcp-"))
    rns_config(config_dir, stock_rnode("tcp://127.0.0.1"))
    RNS.Reticulum(configdir=str(config_dir))
    return RNS.Transport.interfaces[0]


def telemetry(retinue: Retinue) -> dict:
    lines = [line for line in retinue.lines if line.startswith("TELEMETRY ")]
    return json.loads(lines[-1][len("TELEMETRY "):]) if lines else {}


def run() -> dict[str, bool]:
    import RNS

    air = Air(2, ("--telemetry",), tcp=True)
    stock = start_stock(air.tcp_ports[STOCK])
    retinue = Retinue("rnode_tcp_peer", "rnode-tcp", [
        "tcp://127.0.0.1", "--tcp-port", str(air.tcp_ports[RETINUE])])
    results: dict[str, bool] = {}
    try:
        match = retinue.wait_for(r"DEST ([0-9a-f]{32})", 300)
        if match is None:
            return {"retinue printed its destination": False}
        dest = bytes.fromhex(match.group(1))
        results["stock has_path(retinue)"] = wait_until(lambda: RNS.Transport.has_path(dest), 120)
        identity = RNS.Identity.recall(dest)
        if identity is None:
            return results
        out = RNS.Destination(identity, RNS.Destination.OUT, RNS.Destination.SINGLE,
                              "retinue", "serial-peer")
        link = RNS.Link(out)
        results["stock link ACTIVE"] = wait_until(lambda: link.status == RNS.Link.ACTIVE, 60)
        if not results["stock link ACTIVE"]:
            return results

        idle_from = max(t for t, *_ in air.frames())
        time.sleep(IDLE)
        results["after idle: stock interface online"] = bool(stock.online)
        results["after idle: stock link ACTIVE"] = link.status == RNS.Link.ACTIVE
        writes = [t for t, f in air.host_frames(RETINUE) if t > idle_from]
        detects = [t for t, f in air.host_frames(RETINUE, DETECT) if t > idle_from]
        gaps = [b - a for a, b in zip([idle_from, *writes], writes)]
        print(f"  retinue: {len(detects)} DETECTs in {IDLE:.0f} s idle, "
              f"largest write gap {max(gaps, default=0):.2f} s")
        results["idle: retinue DETECT keepalives, writes <= 3.5 s apart"] = (
            len(detects) >= int(IDLE / KEEPALIVE) - 1 and max(gaps, default=99) <= KEEPALIVE + SLACK)

        question = payload(64, REQUEST_SEED)
        receipt = link.request("/echo", question, timeout=30)
        done = (RNS.RequestReceipt.READY, RNS.RequestReceipt.FAILED)
        wait_until(lambda: receipt.get_status() in done, 30)
        results["after idle: stock request answered"] = receipt.get_response() == question
        results["no host reconnected"] = (
            [device.connections for device in air.tcp] == [1, 1]
            and not any("STATUS Fault" in line for line in retinue.lines))

        wait_until(lambda: telemetry(retinue).get("r_csma_cw_max") is not None, 10)
        ours = telemetry(retinue)
        stats = next((s for s in RNS.Reticulum.get_instance().get_interface_stats()["interfaces"]
                      if "airtime_short" in s), {})
        theirs = {key: stats.get(key) for key in STATS_KEYS}
        theirs.update({key: getattr(stock, key, None) for key in ATTR_KEYS})
        mismatched = {key: (ours.get(key), theirs[key]) for key in theirs
                      if ours.get(key) != theirs[key]}
        print(f"  stock telemetry {theirs}")
        if mismatched:
            print(f"  mismatched (retinue, stock): {mismatched}")
        results["telemetry equals stock's get_interface_stats()"] = (
            not mismatched and theirs["airtime_short"] is not None)
        link.teardown()
        time.sleep(1)
        return results
    finally:
        retinue.close()
        air.close()


def main() -> int:
    import RNS

    code = report("RNODE TCP INTEROP", run())
    RNS.exit(code)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
