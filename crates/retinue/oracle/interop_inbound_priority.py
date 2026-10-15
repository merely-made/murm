"""Inbound priority gate: link traffic keeps its latency through an announce flood.

Topology: Retinue's `inbound_priority_peer` listens on TCP and serves a destination that proves
and echoes every link packet. A stock peer links to it and sends 64 packets of 200 B, one at a
time, each waiting for its proof: first idle, then while a stock flooder streams 1,000
distinct stock-signed announces at it as fast as the socket takes them, cycling until the echo
is done. The same run then repeats with a stock transport in Retinue's place, for comparison.

Pass when, with Retinue, every echo packet's receipt is DELIVERED in the stock peer, the p95
RTT under the flood stays within twice the idle p95, and the flood overflowed Retinue's
announce lane. Retinue's lane drop counters and the stock control's RTTs are recorded.

Build: cargo build --release -p retinue --example inbound_priority_peer
Run:   python -u interop_inbound_priority.py   (CARGO_TARGET_DIR as for the build)
"""
from __future__ import annotations

import atexit
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from ingress_harness import REPO, Child, free_port, kill_all, tcp, verdict, wait_until

TITLE = "INBOUND PRIORITY INTEROP"
ECHOES, SIZE, FLOOD = 64, 200, 1000
SEED = bytes([0x3B] * 64)


def config(label: str, interfaces: str, transport: bool = False) -> Path:
    path = Path(tempfile.mkdtemp(prefix=f"retinue-priority-{label}-"))
    atexit.register(shutil.rmtree, path, ignore_errors=True)
    (path / "config").write_text(
        f"[reticulum]\n  enable_transport = {'Yes' if transport else 'No'}\n  share_instance = No\n"
        f"  panic_on_interface_error = No\n\n[logging]\n  loglevel = 2\n\n[interfaces]\n{interfaces}",
        encoding="utf-8")
    return path


class Node(Child):
    """This script in a child process, in one of its stock roles."""

    def __init__(self, label: str, role: str, *args: str) -> None:
        super().__init__([sys.executable, "-u", str(Path(__file__)), role, *args], label)
        ready = self.wait_for(r"READY ?(.*)", 120)
        if ready is None:
            raise RuntimeError(f"{label} did not start")
        self.ready = ready.group(1)

    def call(self, op: str, timeout: float = 10, **args):
        start = self.mark()
        self.send(json.dumps({"op": op, **args}))
        reply = self.wait_for(r"REPLY (.*)", timeout, start)
        if reply is None:
            raise RuntimeError(f"{self.label}: no reply to {op}")
        return json.loads(reply.group(1))


class Peer(Child):
    """The inbound_priority_peer example."""

    def __init__(self) -> None:
        # Release: an unoptimised signature check alone outlasts the idle RTT many times over.
        subprocess.run(["cargo", "build", "--quiet", "--release", "-p", "retinue", "--example",
                        "inbound_priority_peer"], cwd=REPO, check=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
        super().__init__([str(target / "release/examples/inbound_priority_peer")], "retinue")
        self.port = int(self.wait_for(r"LISTENING (\d+)", 30).group(1))
        self.dest = self.wait_for(r"UP ([0-9a-f]{32})", 10).group(1)

    def announce(self) -> None:
        self.send("announce")

    def lanes(self) -> list[int]:
        start = self.mark()
        self.send("lanes")
        m = self.wait_for(r"LANES (\d+) (\d+) (\d+) (\d+)", 5, start)
        return [int(m.group(i)) for i in range(1, 5)]


def serve(handle) -> None:
    print("READY " + str(handle({"op": "ready"})), flush=True)
    for line in sys.stdin:
        try:
            result = handle(json.loads(line))
        except Exception as e:
            result = {"error": repr(e)}
        print("REPLY " + json.dumps(result), flush=True)


def echo_main(port: str) -> None:
    """The stock peer: links to the served destination and times proved packets."""
    import RNS
    RNS.Reticulum(configdir=str(config("echo", tcp("client", "hub", int(port)))))
    link = None

    def handle(req):
        nonlocal link
        op = req["op"]
        if op == "ready":
            return ""
        if op == "online":
            return all(i.online for i in RNS.Transport.interfaces)
        if op == "has_path":
            return RNS.Transport.has_path(bytes.fromhex(req["dest"]))
        if op == "link":
            dest = bytes.fromhex(req["dest"])
            out = RNS.Destination(RNS.Identity.recall(dest), RNS.Destination.OUT, RNS.Destination.SINGLE,
                                  "retinue", "priority")
            link = RNS.Link(out)
            wait_until(lambda: link.status == RNS.Link.ACTIVE, 15)
            return link.status == RNS.Link.ACTIVE
        if op == "echo":
            rtts = []
            for _ in range(req["n"]):
                done = threading.Event()
                receipt = RNS.Packet(link, os.urandom(req["size"])).send()
                if not receipt:
                    print(f"send refused, link status {link.status} reason {link.teardown_reason}", flush=True)
                    rtts.append(None)
                    continue
                receipt.set_delivery_callback(lambda r: done.set())
                done.wait(timeout=10)
                ok = receipt.status == RNS.PacketReceipt.DELIVERED
                rtts.append(receipt.get_rtt() if ok else None)
            return rtts
        raise ValueError(op)

    serve(handle)


def hub_main(port: str) -> None:
    """The stock control: a transport serving the same destination, proving and echoing."""
    import RNS
    RNS.Reticulum(configdir=str(config("hub", tcp("server", "hub", int(port)), transport=True)))
    dest = RNS.Destination(RNS.Identity.from_bytes(SEED), RNS.Destination.IN, RNS.Destination.SINGLE,
                           "retinue", "priority")
    dest.set_proof_strategy(RNS.Destination.PROVE_ALL)

    def established(link):
        link.set_packet_callback(lambda data, packet: RNS.Packet(link, data).send())

    dest.set_link_established_callback(established)

    def handle(req):
        if req["op"] == "ready":
            return dest.hash.hex()
        if req["op"] == "announce":
            dest.announce()
            return True
        raise ValueError(req["op"])

    serve(handle)


def flood_main(port: str) -> None:
    """The flooder: distinct stock-signed announces, HDLC-framed onto a raw socket."""
    import RNS
    from RNS.Interfaces.TCPInterface import HDLC
    RNS.Reticulum(configdir=str(config("flood", "")))
    frames = []
    for _ in range(FLOOD):
        packet = RNS.Destination(RNS.Identity(), RNS.Destination.IN, RNS.Destination.SINGLE,
                                 "priority", "flood").announce(send=False)
        packet.pack()
        frames.append(bytes([HDLC.FLAG]) + HDLC.escape(packet.raw) + bytes([HDLC.FLAG]))
    sock = socket.create_connection(("127.0.0.1", int(port)))
    stop = threading.Event()
    sent = [0]

    def blast():
        while not stop.is_set():
            for at in range(0, FLOOD, 50):
                sock.sendall(b"".join(frames[at:at + 50]))
                sent[0] += len(frames[at:at + 50])

    def handle(req):
        if req["op"] == "ready":
            return ""
        if req["op"] == "go":
            threading.Thread(target=blast, daemon=True).start()
            return True
        if req["op"] == "stop":
            stop.set()
            return sent[0]
        raise ValueError(req["op"])

    serve(handle)


def p95(rtts: list) -> float:
    ordered = sorted(rtts)
    return ordered[min(len(ordered) - 1, int(0.95 * len(ordered)))]


def run(label: str, port: int, dest: str, announce) -> dict:
    """Idle echo, then echo under the flood, against whatever serves `dest` on `port`."""
    flood = Node(f"{label}-flood", "--flood", str(port))  # signs its announces up front
    echo = Node(f"{label}-echo", "--echo", str(port))
    wait_until(lambda: echo.call("online"), 20)
    learned = wait_until(lambda: announce() or echo.call("has_path", dest=dest), 20, 1.0)
    linked = bool(learned) and echo.call("link", dest=dest, timeout=20)
    result = {"linked": linked}
    if linked:
        result["idle"] = echo.call("echo", n=ECHOES, size=SIZE, timeout=120)
        flood.call("go")
        time.sleep(0.5)
        result["flood"] = echo.call("echo", n=ECHOES, size=SIZE, timeout=700)
        result["flood_sent"] = flood.call("stop")
    flood.kill()
    echo.kill()
    return result


def summary(label: str, result: dict) -> tuple[bool, float, float]:
    if any(not isinstance(result.get(k, []), list) for k in ("idle", "flood")):
        print(f"  {label}: {result}", flush=True)
        return False, float("inf"), float("inf")
    rtts = result.get("idle", []) + result.get("flood", [])
    delivered = len(rtts) == 2 * ECHOES and all(r is not None for r in rtts)
    idle = p95(result["idle"]) if delivered else float("inf")
    flood = p95(result["flood"]) if delivered else float("inf")
    print(f"  {label}: linked {result['linked']}, delivered {sum(r is not None for r in rtts)} "
          f"of {2 * ECHOES}, p95 idle {idle * 1000:.1f} ms, p95 flood {flood * 1000:.1f} ms, "
          f"flood frames sent {result.get('flood_sent')}", flush=True)
    return delivered, idle, flood


def main() -> int:
    peer = Peer()
    ours = run("retinue", peer.port, peer.dest, peer.announce)
    lanes = peer.lanes()
    peer.kill()

    hub_port = free_port()
    hub = Node("stock-hub", "--hub", str(hub_port))
    control = run("stock", hub_port, hub.ready, lambda: hub.call("announce") and False)
    hub.kill()

    delivered, idle, flood = summary("retinue", ours)
    summary("stock control", control)
    print(f"  retinue lane drops (data, announce, path request, ingress-limited): {lanes}", flush=True)
    ok = verdict("every echo receipt is DELIVERED in the stock peer", delivered)
    ok &= verdict("p95 RTT under the flood within 2x idle", delivered and flood <= 2 * idle,
                  f"{flood * 1000:.1f} ms vs {idle * 1000:.1f} ms")
    ok &= verdict("the flood outlasted the echo and overflowed only the announce lane",
                  (ours.get("flood_sent") or 0) > FLOOD and lanes[1] > 0 and lanes[0] == 0, str(lanes))
    print(f"\n{TITLE}: {'PASS' if ok else 'FAIL'}", flush=True)
    kill_all()
    return 0 if ok else 1


if __name__ == "__main__":
    roles = {"--echo": echo_main, "--hub": hub_main, "--flood": flood_main}
    if sys.argv[1:2] and sys.argv[1] in roles:
        roles[sys.argv[1]](*sys.argv[2:])
    else:
        raise SystemExit(main())
