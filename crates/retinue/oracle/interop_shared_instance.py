"""Does retinue attach to a stock shared instance as a stock local client does (review A6)?

Topology, all on loopback TCP with free ports (the user's rnsd is untouched):

    third party T --TCP--> P1 shared instance <--local-- P2, P3 stock clients
                                              <--local-- retinue (shared_instance_peer)

Passes on stock state only:
  (a) hop transparency: retinue's hops to P2's destination D equal P3's RNS.Transport.hops_to;
      to T's destination, retinue's wire hops plus one equal P2's and P3's hops_to (retinue
      keeps wire hops, one fewer than RNS's; Transport.py 1936-1939). D's 0 also follows from
      saturation, so retinue's route to D must name no transport node and its route to T
      must name P1's.
  (b) a link from retinue to D goes ACTIVE in P2, and P2 proves a packet on it.
  (c) after P1 is killed and restarted, P2 regains has_path(retinue's destination) within
      30 s with no announce from retinue: its path response 10 s after reconnecting
      (LocalInterface.py 153-185; Transport.py 3705-3709).

The node half (`--node CONFIG_DIR`) is also used by interop_tunnel.py.

Run: CARGO_TARGET_DIR=... ./.venv/bin/python -u interop_shared_instance.py
"""
from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import atexit
import time
from pathlib import Path

from ingress_harness import REPO, Child, free_port, verdict, wait_until

EXPECTED_RNS = "1.5.7"


def example(name: str, env: dict) -> Child:
    """Build and start a retinue example; returns once it printed UP."""
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--example", name], cwd=REPO, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
    child = Child([str(target / "debug" / "examples" / name)], name, env)
    if child.wait_for(r"UP ([0-9a-f]{32})", 30) is None:
        raise RuntimeError(f"{name} did not start")
    return child


def field(child: Child, key: str, timeout: float = 10) -> str:
    m = child.wait_for(rf"{key} (\S+)", timeout)
    if m is None:
        raise RuntimeError(f"{child.label} printed no {key}")
    return m.group(1)


def ask(child: Child, command: str, pattern: str, timeout: float = 5):
    start = child.mark()
    child.send(command)
    return child.wait_for(pattern, timeout, start)


class Node(Child):
    """A stock RNS process driven by JSON lines; `reticulum` and `interfaces` are config text."""

    def __init__(self, label: str, reticulum: str, interfaces: str = "", config: Path | None = None):
        if config is None:
            config = Path(tempfile.mkdtemp(prefix=f"retinue-local-{label}-"))
            atexit.register(shutil.rmtree, config, ignore_errors=True)
            (config / "config").write_text(
                f"[reticulum]\n{reticulum}  panic_on_interface_error = No\n\n"
                f"[logging]\n  loglevel = 3\n\n[interfaces]\n{interfaces}", encoding="utf-8")
        self.config = config
        super().__init__([sys.executable, "-u", str(Path(__file__)), "--node", str(config)], label)
        ready = self.wait_for(r"READY ([0-9a-f]{32}) (\w+)", 30)
        if ready is None:
            raise RuntimeError(f"stock node {label} did not start")
        self.transport_id, self.client = ready.group(1), ready.group(2) == "True"

    def call(self, op: str, timeout: float = 10, **args):
        start = self.mark()
        self.send(json.dumps({"op": op, **args}))
        reply = self.wait_for(r"REPLY (.*)", timeout, start)
        if reply is None:
            raise RuntimeError(f"{self.label}: no reply to {op}")
        result = json.loads(reply.group(1))
        if isinstance(result, dict) and "error" in result:
            raise RuntimeError(f"{self.label}: {op}: {result['error']}")
        return result


def node_main(config_dir: str) -> None:
    """The child side of Node: RNS plus stdin commands, with stock-state taps."""
    import RNS

    assert RNS.__version__ == EXPECTED_RNS, RNS.__version__
    proofs, announced, links = [0], {}, []
    prove, transmit = RNS.Packet.prove, RNS.Transport.transmit

    def tap_prove(packet, *args, **kwargs):
        proofs[0] += 1
        return prove(packet, *args, **kwargs)

    def tap_transmit(interface, raw):
        packet = RNS.Packet(None, raw)
        if packet.unpack() and packet.packet_type == RNS.Packet.ANNOUNCE:
            key = packet.destination_hash.hex()
            announced[key] = announced.get(key, 0) + 1
        return transmit(interface, raw)

    RNS.Packet.prove, RNS.Transport.transmit = tap_prove, tap_transmit
    reticulum = RNS.Reticulum(configdir=config_dir)
    identity = RNS.Identity()
    destinations: list = []

    def established(link):
        links.append(link)

    def handle(req):
        op = req["op"]
        dest = bytes.fromhex(req.get("dest", ""))
        if op == "register":
            d = RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE, "localgate", req["aspect"])
            d.set_proof_strategy(RNS.Destination.PROVE_ALL)
            d.set_link_established_callback(established)
            destinations.append(d)
            return d.hash.hex()
        if op == "announce":
            destinations[req.get("index", 0)].announce()
            return True
        if op == "has_path":
            return RNS.Transport.has_path(dest)
        if op == "hops_to":
            return RNS.Transport.hops_to(dest)
        if op == "links":
            return [link.status == RNS.Link.ACTIVE for link in links]
        if op == "proofs":
            return proofs[0]
        if op == "announced":
            return announced.get(dest.hex(), 0)
        if op == "link":
            peer = RNS.Identity.recall(dest)
            out = RNS.Destination(peer, RNS.Destination.OUT, RNS.Destination.SINGLE,
                                  req.get("app", "retinue"), req["aspect"])
            links.append(RNS.Link(out))
            return True
        if op == "tunnels":
            return {tid.hex(): str(entry[1]) for tid, entry in RNS.Transport.tunnels.items()}
        if op == "close_tunnel_iface":
            iface = RNS.Transport.tunnels[bytes.fromhex(req["tunnel"])][1]
            iface.socket.shutdown(socket.SHUT_RDWR)
            return str(iface)
        if op == "via_tunnel":
            iface = RNS.Transport.tunnels[bytes.fromhex(req["tunnel"])][1]
            return iface is not None and RNS.Transport.next_hop_interface(dest) is iface
        if op == "close_client":
            iface = next(i for i in RNS.Transport.interfaces if getattr(i, "name", None) == req["name"])
            iface.socket.shutdown(socket.SHUT_RDWR)
            return True
        if op == "online":
            iface = next(i for i in RNS.Transport.interfaces if getattr(i, "name", None) == req["name"])
            return bool(iface.online)
        raise ValueError(op)

    print(f"READY {RNS.Transport.identity.hash.hex()} {reticulum.is_connected_to_shared_instance}", flush=True)
    for line in sys.stdin:
        try:
            result = handle(json.loads(line))
        except Exception as e:  # report, keep serving
            result = {"error": repr(e)}
        print("REPLY " + json.dumps(result), flush=True)
    RNS.exit(0)


def main() -> int:
    shared_port, control_port, tcp_port = free_port(), free_port(), free_port()
    shared = (f"  share_instance = Yes\n  shared_instance_port = {shared_port}\n"
              f"  instance_control_port = {control_port}\n  enable_transport = No\n")
    p1 = Node("p1", shared, "  [[t]]\n    type = TCPServerInterface\n    enabled = yes\n"
              f"    listen_ip = 127.0.0.1\n    listen_port = {tcp_port}\n")
    t = Node("t", "  share_instance = No\n  enable_transport = No\n",
             "  [[p1]]\n    type = TCPClientInterface\n    enabled = yes\n"
             f"    target_host = 127.0.0.1\n    target_port = {tcp_port}\n")
    # Stock programs share rnsd's configuration directory; each still takes an ephemeral
    # transport identity, which is why retinue reaches P2's destination with header type 1.
    p2, p3 = Node("p2", shared, config=p1.config), Node("p3", shared, config=p1.config)
    ok = verdict("P2 and P3 are local clients of P1", p2.client and p3.client and not p1.client)

    retinue = example("shared_instance_peer", {"RETINUE_SHARED_PORT": str(shared_port)})
    r_dest = field(retinue, "DEST")
    d, td = p2.call("register", aspect="d"), t.call("register", aspect="t")
    p2.call("announce")
    t.call("announce")

    def route_via(dest):
        m = ask(retinue, f"route {dest}", rf"ROUTE {dest} (\d+|none)(?: (\d+) (\S+))?")
        return (None, None) if not m or m.group(1) == "none" else (int(m.group(2)), m.group(3))

    def route(dest):
        return route_via(dest)[0]

    wait_until(lambda: route(d) is not None and route(td) is not None, 20, 0.5)
    wait_until(lambda: p3.call("has_path", dest=d) and p2.call("has_path", dest=td), 20, 0.5)
    r_d, r_t = route(d), route(td)
    s_d, s_t2, s_t3 = (p3.call("hops_to", dest=d), p2.call("hops_to", dest=td), p3.call("hops_to", dest=td))
    ok &= verdict("(a) hops to P2's destination match P3's hops_to", r_d is not None and r_d == s_d,
                  f"retinue {r_d}, P3 {s_d}")
    ok &= verdict("(a) hops to the third party match P2's and P3's hops_to", r_t is not None
                  and r_t + 1 == s_t2 == s_t3, f"retinue wire {r_t}, P2 {s_t2}, P3 {s_t3}")
    # Saturation makes D read 0 either way, so check the hop-0 versus one-past distinction on
    # retinue's own route: D is reached directly, T through P1 (Transport.py 1416-1437).
    via_d, via_t = route_via(d)[1], route_via(td)[1]
    ok &= verdict("(a) D's route names no transport node, T's names P1", via_d == "-"
                  and via_t == p1.transport_id, f"D via {via_d}, T via {via_t}, P1 {p1.transport_id}")

    linked = ask(retinue, f"link {d}", r"(LINKED|LINK_FAILED) ?(.*)", 20)
    active = wait_until(lambda: any(p2.call("links")), 10, 0.3)
    proved = wait_until(lambda: p2.call("proofs") > 0, 10, 0.3)
    ok &= verdict("(b) a link from retinue goes ACTIVE in P2", bool(active),
                  linked.group(0) if linked else "no answer")
    ok &= verdict("(b) P2 proves a packet on it", bool(proved))

    wait_until(lambda: p2.call("has_path", dest=r_dest), 15, 0.5)
    ok &= verdict("P2 has a path to retinue before the restart", p2.call("has_path", dest=r_dest))
    p1.kill()
    gone = wait_until(lambda: not p2.call("has_path", dest=r_dest), 15, 0.5)
    ok &= verdict("P2 drops its paths when P1 disappears", bool(gone))
    p1 = Node("p1-again", shared, config=p1.config)
    started = time.monotonic()
    regained = wait_until(lambda: p2.call("has_path", dest=r_dest), 30, 0.5)
    ok &= verdict("(c) P2 regains a path to retinue within 30 s of the restart", bool(regained),
                  f"{time.monotonic() - started:.1f} s")
    online = ask(retinue, "online", r"ONLINE (\w+)")
    ok &= verdict("retinue is back online", online is not None and online.group(1) == "true")
    print(f"\nA6 SHARED-INSTANCE: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--node"]:
        node_main(sys.argv[2])
    else:
        raise SystemExit(main())
