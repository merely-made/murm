"""Does a retinue KISS listener carry a stock `kiss_framing = yes` TCP client (C5)?

`tcp_kiss_peer` listens with KISS framing; a stock TCPClientInterface dials it with
`kiss_framing = yes` (`TCPInterface.py` 55-67, 322-388). Announces must go both ways
(stock has_path to retinue, retinue learns stock's destination), a stock link must go
ACTIVE and get a 431-byte packet (the link MDU, a full 500-byte packet) proved, and stock
must send no tunnel synthesis on a KISS client (`TCPInterface.py` 178-179).

Run from the oracle/ directory:  ./.venv/bin/python -u interop_tcp_kiss.py
"""
from __future__ import annotations

import shutil
import sys
import tempfile
import subprocess
from pathlib import Path

from ingress_harness import Child, verdict, wait_until
from interop_pipe import Heard, build, link_and_prove, start_stock

ASPECT = "kiss-peer"


def run(binary: str, config: Path) -> dict[str, bool]:
    import RNS

    peer = Child([binary], "retinue")
    port = peer.wait_for(r"LISTENING (\d+)", 30)
    if port is None:
        return {"retinue listened": False}
    synth = RNS.Destination.hash_from_name_and_identity("rnstransport.tunnel.synthesize", None)
    tunnels: list[bytes] = []
    transmit = RNS.Transport.transmit

    def tap(interface, raw):
        packet = RNS.Packet(None, raw)
        if packet.unpack() and packet.destination_hash == synth:
            tunnels.append(raw)
        return transmit(interface, raw)

    RNS.Transport.transmit = tap
    start_stock("  [[retinue-kiss]]\n    type = TCPClientInterface\n    enabled = yes\n"
                f"    target_host = 127.0.0.1\n    target_port = {port.group(1)}\n"
                "    kiss_framing = yes\n", config)
    heard = Heard(ASPECT)
    stock_iface = next(i for i in RNS.Transport.interfaces if i.name == "retinue-kiss")
    results = {"stock client runs KISS framing": stock_iface.kiss_framing is True}
    results["stock has_path(retinue)"] = bool(wait_until(
        lambda: heard.dests and RNS.Transport.has_path(heard.dests[0]), 60))
    if not heard.dests:
        return results
    mine = RNS.Destination(RNS.Identity(), RNS.Destination.IN, RNS.Destination.SINGLE,
                           "stock", "kiss-client")
    mine.announce()
    results["retinue learned stock's announce"] = bool(
        peer.wait_for(rf"LEARNED {mine.hash.hex()}", 15))
    results.update(link_and_prove(heard.dests[0], ASPECT, 431, "link"))
    results["no tunnel synthesis on the KISS client"] = (
        not tunnels and not getattr(stock_iface, "wants_tunnel", False))
    return results


def child(binary: str, config: Path) -> int:
    import RNS

    code = 1
    try:
        ok = [verdict(name, passed) for name, passed in run(binary, config).items()]
        code = 0 if ok and all(ok) else 1
        return code
    finally:
        RNS.exit(code)


def main() -> int:
    binary = build("tcp_kiss_peer", "tokio")
    config = tempfile.mkdtemp(prefix="retinue-tcp-kiss-")
    try:
        code = subprocess.run([sys.executable, "-u", __file__, "--child", str(binary), config],
                              timeout=300).returncode
    except subprocess.TimeoutExpired:
        code = None
    finally:
        shutil.rmtree(config, ignore_errors=True)
    print(f"TCP KISS INTEROP: {'PASS' if code == 0 else 'FAIL'}", flush=True)
    return 0 if code == 0 else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        raise SystemExit(child(sys.argv[2], Path(sys.argv[3])))
    raise SystemExit(main())
