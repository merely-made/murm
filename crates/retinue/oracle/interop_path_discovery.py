"""Does a retinue transport answer and search path requests as RNS 1.5.7 does?

Topology (loopback TCP, each stock node its own RNS process):

    A (stock client) --+
    A2 (stock client) -+-- T (retinue discovery_node, or a stock transport as the control)
                       +-- C (stock transport) -- B (stock destination)

B announces before T attaches to C, so T has no path to B. Cases, each read from the stock
side's own state and compared with the same case run against the stock control:

  gateway  A's interface on T is GATEWAY. A requests B: T searches on C's side and passes C's
           answer back, so A's has_path(B) turns true with the control's hops_to, and a link
           from A to B goes ACTIVE through T (`Transport.py` 2430-2455, 3528-3583).
  cache    Then A2, on a FULL interface, requests B: T answers from its announce cache, and
           A2 has the path within 2 s (3462-3524).
  full     A's interface on T is FULL: no search, so A never learns B (3420-3436).
  roaming  T dials B directly on a ROAMING interface and B asks for its own destination: the
           path was learned on that same roaming interface, so no path response reaches B
           (3468-3471). On a FULL interface (the check that the tap sees answers) one does.

Build: cargo build -p retinue --example discovery_node
Run:   python -u interop_path_discovery.py   (CARGO_TARGET_DIR as for the build)
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from ingress_harness import Child, REPO, free_port, kill_all, tcp, verdict, wait_until

TITLE = "R-A4 PATH-DISCOVERY INTEROP"
SEARCH_WINDOW = 15.0  # RNS's PATH_REQUEST_TIMEOUT, the stock control's own wait
CACHE_WINDOW = 2.0
QUIET_WINDOW = 5.0
# C relays B's announce once more after its 5 s grace (Transport.py 765-829): T attaches only
# after that retry, so the path can only come from a path request.
RETRY_SETTLE = 8.0


class Node(Child):
    """A stock RNS node driven by line commands (see node_main)."""

    def __init__(self, label: str, transport: bool, interfaces: str) -> None:
        config = Path(tempfile.mkdtemp(prefix=f"retinue-discovery-{label}-"))
        (config / "config").write_text(
            f"[reticulum]\n  enable_transport = {'Yes' if transport else 'No'}\n"
            "  share_instance = No\n  panic_on_interface_error = No\n\n"
            f"[logging]\n  loglevel = 2\n\n[interfaces]\n{interfaces}", encoding="utf-8")
        super().__init__([sys.executable, "-u", str(Path(__file__).resolve()), "--node", str(config)], label)
        ready = self.wait_for(r"READY ([0-9a-f]{32})", 30)
        if ready is None:
            raise RuntimeError(f"{label} did not start")
        self.transport_id = ready.group(1)

    def ask(self, command: str, pattern: str, timeout: float = 20):
        start = self.mark()
        self.send(command)
        return self.wait_for(pattern, timeout, start)

    def has(self, dest: str) -> bool:
        reply = self.ask(f"has {dest}", rf"HAS {dest} (\d)")
        return reply is not None and reply.group(1) == "1"

    def route(self, dest: str):
        return self.has(dest)


class Retinue(Child):
    """The discovery_node example, dialing `port:mode` peers."""

    def __init__(self, peers: list[str]) -> None:
        target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
        binary = target / "debug" / "examples" / "discovery_node"
        super().__init__([str(binary), *peers], "retinue")
        up = self.wait_for(r"UP ([0-9a-f]{32})", 30)
        if up is None:
            raise RuntimeError("discovery_node did not start")
        self.transport_id = up.group(1)

    def ask(self, command: str, pattern: str, timeout: float = 5):
        start = self.mark()
        self.send(command)
        return self.wait_for(pattern, timeout, start)

    def route(self, dest: str):
        reply = self.ask(f"route {dest}", rf"ROUTE {dest} (\S+)(?: (\d+))?")
        return reply is not None and reply.group(1) != "none"


def transport(stock: bool, peers: list[tuple[int, str]]):
    if not stock:
        return Retinue([f"{port}:{mode}" for port, mode in peers])
    sections = "".join(tcp("client", f"peer{i}", port, mode=mode) for i, (port, mode) in enumerate(peers))
    return Node("control", True, sections)


def discovery(stock: bool, a_mode: str) -> dict:
    """The gateway and cache cases (a_mode gateway), or the full case."""
    pc, pa, pa2 = free_port(), free_port(), free_port()
    c = Node("C", True, tcp("server", "c", pc))
    b = Node("B", False, tcp("client", "c", pc))
    dest = b.ask("dest", r"DEST ([0-9a-f]{32})").group(1)
    b.send("announce")
    if not wait_until(lambda: c.has(dest), 10):
        raise RuntimeError("C did not learn B")
    time.sleep(RETRY_SETTLE)
    a = Node("A", False, tcp("server", "a", pa))
    a2 = Node("A2", False, tcp("server", "a2", pa2))
    t = transport(stock, [(pa, a_mode), (pa2, "full"), (pc, "full")])
    time.sleep(3.0)
    result = {"before": t.route(dest)}
    a.send(f"request {dest}")
    result["has_path"] = bool(wait_until(lambda: a.has(dest), SEARCH_WINDOW, 0.25))
    if a_mode == "gateway":
        hops = a.ask(f"hops {dest}", rf"HOPS {dest} (\d+)")
        result["hops"] = int(hops.group(1)) if hops else None
        link = a.ask(f"link {dest}", rf"LINK {dest} (\w+)", 25)
        result["link"] = link.group(1) if link else None
        asked = time.monotonic()
        a2.send(f"request {dest}")
        cached = wait_until(lambda: a2.has(dest), CACHE_WINDOW + 3, 0.1)
        result["cache_s"] = round(time.monotonic() - asked, 2) if cached else None
        if not stock:
            counters = t.ask("counters", r"COUNTERS (\d+) (\d+) \d+ \d+")
            result["answered_searched"] = tuple(map(int, counters.groups())) if counters else None
    kill_all_of([t, a, a2, b, c])
    return result


def roaming(stock: bool, mode: str) -> int:
    """Path responses B hears for its own destination after asking for it via T."""
    pb = free_port()
    b = Node("B", False, tcp("server", "b", pb))
    dest = b.ask("dest", r"DEST ([0-9a-f]{32})").group(1)
    t = transport(stock, [(pb, mode)])
    time.sleep(3.0)
    b.send("announce")
    if not wait_until(lambda: t.route(dest), 10):
        raise RuntimeError("the transport did not learn B")
    b.send(f"request {dest}")
    time.sleep(QUIET_WINDOW)
    reply = b.ask(f"responses {dest}", rf"RESPONSES {dest} (\d+)")
    kill_all_of([t, b])
    return int(reply.group(1)) if reply else -1


def kill_all_of(children) -> None:
    for child in children:
        child.kill()


def node_main(config_dir: str) -> None:
    """The stock side: RNS plus line commands, with an inbound path-response tap."""
    import RNS

    responses: dict[str, int] = {}
    inbound = RNS.Transport.inbound

    def tap(raw, *args, **kwargs):
        packet = RNS.Packet(None, raw)
        if packet.unpack() and packet.packet_type == RNS.Packet.ANNOUNCE \
                and packet.context == RNS.Packet.PATH_RESPONSE:
            key = packet.destination_hash.hex()
            responses[key] = responses.get(key, 0) + 1
        return inbound(raw, *args, **kwargs)

    RNS.Transport.inbound = tap
    RNS.Reticulum(configdir=config_dir)
    destination = None
    links = []
    print(f"READY {RNS.Transport.identity.hash.hex()}", flush=True)
    for line in sys.stdin:
        cmd = line.split()
        if cmd == ["dest"]:
            destination = RNS.Destination(RNS.Identity(), RNS.Destination.IN, RNS.Destination.SINGLE,
                                          "discovery_gate", "b")
            destination.set_link_established_callback(links.append)
            print(f"DEST {destination.hash.hex()}", flush=True)
        elif cmd == ["announce"]:
            destination.announce()
        elif cmd[:1] == ["request"]:
            RNS.Transport.request_path(bytes.fromhex(cmd[1]))
        elif cmd[:1] == ["has"]:
            print(f"HAS {cmd[1]} {int(RNS.Transport.has_path(bytes.fromhex(cmd[1])))}", flush=True)
        elif cmd[:1] == ["hops"]:
            print(f"HOPS {cmd[1]} {RNS.Transport.hops_to(bytes.fromhex(cmd[1]))}", flush=True)
        elif cmd[:1] == ["responses"]:
            print(f"RESPONSES {cmd[1]} {responses.get(cmd[1], 0)}", flush=True)
        elif cmd[:1] == ["link"]:
            dest = bytes.fromhex(cmd[1])
            remote = RNS.Destination(RNS.Identity.recall(dest), RNS.Destination.OUT,
                                     RNS.Destination.SINGLE, "discovery_gate", "b")
            link = RNS.Link(remote)
            deadline = time.time() + 20
            while time.time() < deadline and link.status not in (RNS.Link.ACTIVE, RNS.Link.CLOSED):
                time.sleep(0.1)
            status = "ACTIVE" if link.status == RNS.Link.ACTIVE else "FAILED"
            print(f"LINK {cmd[1]} {status}", flush=True)
    RNS.exit(0)


def main() -> int:
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--example", "discovery_node"],
                   cwd=REPO, check=True)
    import RNS
    print(f"RNS {RNS.__version__}\n", flush=True)
    ok = True
    gateway = {side: discovery(side == "stock", "gateway") for side in ("retinue", "stock")}
    full = {side: discovery(side == "stock", "full") for side in ("retinue", "stock")}
    roam = {side: {mode: roaming(side == "stock", mode) for mode in ("roaming", "full")}
            for side in ("retinue", "stock")}
    r, s = gateway["retinue"], gateway["stock"]
    print("\n" + "=" * 72)
    print(f"gateway: retinue {r}\n         stock   {s}")
    print(f"full:    retinue {full['retinue']}\n         stock   {full['stock']}")
    print(f"roaming: retinue {roam['retinue']}\n         stock   {roam['stock']}")
    ok &= verdict("no path to B before A asks", not r["before"] and not s["before"], "")
    ok &= verdict("gateway: A learns B through the search", r["has_path"] and s["has_path"])
    ok &= verdict("gateway: hops_to matches the control", r["hops"] is not None and r["hops"] == s["hops"],
                  f"retinue {r['hops']}, stock {s['hops']}")
    ok &= verdict("gateway: A's link to B goes ACTIVE through T", r["link"] == s["link"] == "ACTIVE",
                  f"retinue {r['link']}, stock {s['link']}")
    ok &= verdict("cache: A2 has the path within 2 s",
                  r["cache_s"] is not None and r["cache_s"] <= CACHE_WINDOW
                  and s["cache_s"] is not None and s["cache_s"] <= CACHE_WINDOW,
                  f"retinue {r['cache_s']} s, stock {s['cache_s']} s")
    ok &= verdict("retinue searched once and answered once from its cache",
                  r.get("answered_searched") == (1, 1), f"{r.get('answered_searched')}")
    ok &= verdict("full: A does not learn B, as the control",
                  not full["retinue"]["has_path"] and not full["stock"]["has_path"])
    ok &= verdict("roaming: no answer on the roaming interface the path came by",
                  roam["retinue"]["roaming"] == roam["stock"]["roaming"] == 0)
    ok &= verdict("roaming: the same request on a full interface is answered",
                  roam["retinue"]["full"] > 0 and roam["stock"]["full"] > 0)
    print("=" * 72)
    print(f"{TITLE}: {'PASS' if ok else 'FAIL'}", flush=True)
    kill_all()
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--node"]:
        node_main(sys.argv[2])
    else:
        raise SystemExit(main())
