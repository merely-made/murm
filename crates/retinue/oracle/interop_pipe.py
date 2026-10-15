"""Does a retinue PipeInterface interoperate with stock RNS 1.5.7 both ways (C4)?

(a) Stock's [[PipeInterface]] runs `pipe_peer` as its `command`, which serves retinue over
    its stdio (`PipeInterface.py` 41-207). Stock must hear retinue's announce, has_path,
    open a link that goes ACTIVE, and get a 431-byte link packet proved.
(b) `pipe_peer spawn` attaches a retinue pipe to `stdio_tcp_bridge.py`, which carries the
    HDLC to a stock TCPServerInterface. Stock must has_path and prove a link packet. The
    bridge is then killed: retinue must go offline, respawn it after the 5 s respawn_delay
    under the same interface id, and stock must re-learn the path over the new connection
    and prove a packet on a fresh link.

Each side runs in its own process, since stock RNS state is process-global.

Run from the oracle/ directory:  ./.venv/bin/python -u interop_pipe.py
"""
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from ingress_harness import HERE, REPO, Child, free_port, verdict, wait_until

ASPECT = "pipe-peer"
PACKET_LEN = 431


def build(example: str, features: str) -> Path:
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--features", features,
                    "--example", example], cwd=REPO, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
    return target / "debug" / "examples" / example


def start_stock(interface: str, config: Path) -> None:
    """Start stock RNS in this process with one interface section, configured in `config`."""
    import RNS

    (config / "config").write_text(
        "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
        "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 2\n\n[interfaces]\n"
        + interface, encoding="utf-8")
    RNS.Reticulum(configdir=str(config))


class Heard:
    """Stock announce handler recording each announce of `retinue.<aspect>`."""

    def __init__(self, aspect: str) -> None:
        import RNS

        self.aspect_filter = f"retinue.{aspect}"
        self.dests: list[bytes] = []
        RNS.Transport.register_announce_handler(self)

    def received_announce(self, destination_hash, announced_identity, app_data):
        self.dests.append(destination_hash)


def link_and_prove(dest: bytes, aspect: str, size: int, label: str) -> dict[str, bool]:
    """Open a stock link to retinue and send one packet, judged by stock's link and receipt."""
    import RNS

    identity = RNS.Identity.recall(dest)
    if identity is None:
        return {f"{label}: stock recalled retinue's identity": False}
    out = RNS.Destination(identity, RNS.Destination.OUT, RNS.Destination.SINGLE, "retinue", aspect)
    link = RNS.Link(out)
    results = {f"{label}: stock link ACTIVE": bool(
        wait_until(lambda: link.status == RNS.Link.ACTIVE, 30))}
    if results[f"{label}: stock link ACTIVE"]:
        receipt = RNS.Packet(link, os.urandom(size)).send()
        wait_until(lambda: receipt.status != RNS.PacketReceipt.SENT, 15)
        results[f"{label}: stock proved a {size}-byte link packet"] = (
            receipt.status == RNS.PacketReceipt.DELIVERED)
    link.teardown()
    return results


def stock_spawns(binary: str, config: Path) -> dict[str, bool]:
    import RNS

    start_stock("  [[retinue-pipe]]\n    type = PipeInterface\n    enabled = yes\n"
                f"    command = {binary}\n    respawn_delay = 5\n", config)
    heard = Heard(ASPECT)
    results = {"(a) stock heard retinue over its pipe": bool(wait_until(lambda: heard.dests, 60))}
    if not heard.dests:
        return results
    dest = heard.dests[0]
    results["(a) stock has_path(retinue)"] = bool(
        wait_until(lambda: RNS.Transport.has_path(dest), 15))
    results.update(link_and_prove(dest, ASPECT, PACKET_LEN, "(a)"))
    return results


def retinue_spawns(binary: str, config: Path) -> dict[str, bool]:
    import RNS

    port = free_port()
    start_stock("  [[hub]]\n    type = TCPServerInterface\n    enabled = yes\n"
                f"    listen_ip = 127.0.0.1\n    listen_port = {port}\n", config)
    heard = Heard(ASPECT)
    bridge = [sys.executable, "-I", str(HERE / "stdio_tcp_bridge.py"), str(port)]
    peer = Child([binary, "spawn", *bridge], "retinue")
    first = peer.wait_for(r"BRIDGE_PID (\d+)", 30)
    iface = peer.wait_for(r"IFACE (\d+)", 10)
    results = {"(b) retinue spawned the bridge": bool(first and iface)}
    if not results["(b) retinue spawned the bridge"]:
        return results
    results["(b) stock has_path(retinue)"] = bool(wait_until(
        lambda: heard.dests and RNS.Transport.has_path(heard.dests[0]), 60))
    if not heard.dests:
        return results
    dest = heard.dests[0]
    results.update(link_and_prove(dest, ASPECT, PACKET_LEN, "(b)"))

    old_hop, heard_before, mark = RNS.Transport.next_hop_interface(dest), len(heard.dests), peer.mark()
    os.kill(int(first.group(1)), signal.SIGKILL)
    killed = time.monotonic()
    offline = peer.wait_for(rf"ONLINE {iface.group(1)} false", 10, mark)
    second = peer.wait_for(r"BRIDGE_PID (\d+)", 30, mark)
    delay = time.monotonic() - killed
    results[f"(b) retinue went offline and respawned after {delay:.1f} s"] = bool(
        offline and second and 4.5 <= delay < 15)
    results["(b) the same interface came back online"] = bool(
        peer.wait_for(rf"ONLINE {iface.group(1)} true", 10, mark))
    results["(b) stock re-learned the path over the new connection"] = bool(wait_until(
        lambda: len(heard.dests) > heard_before and RNS.Transport.has_path(dest)
        and RNS.Transport.next_hop_interface(dest) not in (None, old_hop), 30))
    results.update(link_and_prove(dest, ASPECT, PACKET_LEN, "(b) after respawn"))
    return results


def child(direction: str, binary: str, config: Path) -> int:
    import RNS

    code = 1
    try:
        run = stock_spawns if direction == "a" else retinue_spawns
        results = run(binary, config)
        ok = [verdict(name, passed) for name, passed in results.items()]
        code = 0 if ok and all(ok) else 1
        return code
    finally:
        RNS.exit(code)


def main() -> int:
    binary = build("pipe_peer", "pipe")
    failed = []
    for direction in ("a", "b"):
        print(f"--- direction ({direction})", flush=True)
        config = tempfile.mkdtemp(prefix=f"retinue-pipe-{direction}-")
        argv = [sys.executable, "-u", __file__, "--child", direction, str(binary), config]
        try:
            code = subprocess.run(argv, timeout=300).returncode
        except subprocess.TimeoutExpired:
            code = None
        finally:
            shutil.rmtree(config, ignore_errors=True)
        if code != 0:
            failed.append(direction)
    print(f"PIPE INTEROP: {'PASS' if not failed else 'FAIL (' + ', '.join(failed) + ')'}", flush=True)
    return 0 if not failed else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        raise SystemExit(child(sys.argv[2], sys.argv[3], Path(sys.argv[4])))
    raise SystemExit(main())
