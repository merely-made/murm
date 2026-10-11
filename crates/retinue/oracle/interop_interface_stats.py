"""Interface statistics gate: Retinue's per-interface counters match stock's view of the link.

Topology: Retinue's `interface_stats_peer` listens with IFAC. A stock node dials it with the
same network name and passphrase and exchanges announces and a path request. A second stock
node dials the same port with a wrong passphrase and announces, so each of its frames is an
IFAC violation on Retinue's side.

Pass, after quiescence, when for the honest link Retinue's tx_bytes equals stock's `rxb`, its
tx_packets equals the frames stock's TCP interface took in, its rx_bytes and rx_packets equal
the frames and IFAC-sealed bytes stock handed its TCP interface to send, and its
ifac_violations is 0; and Retinue's ifac_violations on the second link equals the frames the
wrong-key node sent. Stock's `txb` counts HDLC flags and escapes (`TCPInterface.py` 320-327),
so the rx side is compared with a tap of the bytes stock sealed, before framing.

Build: cargo build -p retinue --example interface_stats_peer
Run:   python -u interop_interface_stats.py   (CARGO_TARGET_DIR as for the build)
"""
from __future__ import annotations

import atexit
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from ingress_harness import REPO, Child, kill_all, tcp, verdict, wait_until

TITLE = "INTERFACE STATS INTEROP"
NETWORK, PASSPHRASE = "retinue-stats", "matching-key"
WRONG = "wrong-key"
INJECTED_ANNOUNCES = 5


class Peer(Child):
    """The interface_stats_peer example."""

    def __init__(self) -> None:
        subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--example",
                        "interface_stats_peer"], cwd=REPO, check=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
        super().__init__([str(target / "debug/examples/interface_stats_peer")], "retinue",
                         {"RETINUE_IFAC": f"{NETWORK},{PASSPHRASE}"})
        self.port = int(self.wait_for(r"LISTENING (\d+)", 30).group(1))
        self.wait_for(r"UP [0-9a-f]{32}", 10)

    def stats(self) -> dict[int, dict]:
        start = self.mark()
        self.send("stats")
        self.wait_for("END", 5, start)
        return {int(m.group(1)): json.loads(m.group(2)) for m in self.matches(r"STATS (\d+) (.*)", start)}


class Node(Child):
    """A stock RNS node dialing Retinue, its TCP frames tapped."""

    def __init__(self, label: str, port: int, passphrase: str) -> None:
        self.config = Path(tempfile.mkdtemp(prefix=f"retinue-stats-{label}-"))
        atexit.register(shutil.rmtree, self.config, ignore_errors=True)
        (self.config / "config").write_text(
            "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
            "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 2\n\n[interfaces]\n"
            + tcp("client", "link", port, network_name=NETWORK, passphrase=passphrase),
            encoding="utf-8")
        super().__init__([sys.executable, "-u", str(Path(__file__)), "--node", str(self.config)], label)
        if self.wait_for(r"READY", 30) is None:
            raise RuntimeError(f"stock node {label} did not start")

    def call(self, op: str, **args):
        start = self.mark()
        self.send(json.dumps({"op": op, **args}))
        reply = self.wait_for(r"REPLY (.*)", 10, start)
        if reply is None:
            raise RuntimeError(f"{self.label}: no reply to {op}")
        return json.loads(reply.group(1))


def node_main(config_dir: str) -> None:
    import RNS
    from RNS.Interfaces.TCPInterface import TCPClientInterface

    taps = {"frames_in": 0, "frames_out": 0, "sealed_out": 0}
    incoming, outgoing = TCPClientInterface.process_incoming, TCPClientInterface.process_outgoing

    def tap_in(self, data):  # counted under process_incoming's own conditions
        if data and self.online and not self.detached:
            taps["frames_in"] += 1
        return incoming(self, data)

    def tap_out(self, data):
        if self.online and not self.detached:
            taps["frames_out"] += 1
            taps["sealed_out"] += len(data)
        return outgoing(self, data)

    TCPClientInterface.process_incoming, TCPClientInterface.process_outgoing = tap_in, tap_out
    reticulum = RNS.Reticulum(configdir=config_dir)
    identity = RNS.Identity()
    dests = []

    def handle(req):
        op = req["op"]
        if op == "online":
            return all(i.online for i in RNS.Transport.interfaces)
        if op == "announce":
            for _ in range(req["n"]):
                dests.append(RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE,
                                             "stats", "gate", str(len(dests))))
                dests[-1].announce()
            return True
        if op == "request":
            RNS.Transport.request_path(bytes.fromhex(req["dest"]))
            return True
        if op == "has_path":
            return RNS.Transport.has_path(bytes.fromhex(req["dest"]))
        if op == "stats":
            link = next(s for s in reticulum.get_interface_stats()["interfaces"] if s["short_name"] == "link")
            return {"rxb": link["rxb"], "txb": link["txb"], "ifac_violations": link["ifac_violations"], **taps}
        raise ValueError(op)

    print("READY", flush=True)
    for line in sys.stdin:
        try:
            result = handle(json.loads(line))
        except Exception as e:
            result = {"error": repr(e)}
        print("REPLY " + json.dumps(result), flush=True)


def main() -> int:
    peer = Peer()
    honest = Node("honest", peer.port, PASSPHRASE)
    wait_until(lambda: honest.call("online"), 20)
    wait_until(lambda: len(peer.stats()) == 1, 10)
    [honest_id] = peer.stats()

    honest.call("announce", n=3)
    start = peer.mark()
    peer.send("announce")
    dest = peer.wait_for(r"ANNOUNCED ([0-9a-f]{32})", 5, start).group(1)
    learned = wait_until(lambda: honest.call("has_path", dest=dest), 10)
    honest.call("request", dest=dest)
    peer.send("announce")

    wrong = Node("wrong", peer.port, WRONG)
    wait_until(lambda: wrong.call("online"), 20)
    wait_until(lambda: len(peer.stats()) == 2, 10)
    wrong.call("announce", n=INJECTED_ANNOUNCES)
    time.sleep(3.0)  # quiescence

    stock = honest.call("stats")
    injected = wrong.call("stats")["frames_out"]
    rows = peer.stats()
    ours = rows[honest_id]
    bad = next(row for id_, row in rows.items() if id_ != honest_id)
    print(f"  stock honest: {stock}", flush=True)
    print(f"  retinue honest: {ours}", flush=True)
    print(f"  wrong-key frames sent {injected}; retinue saw {bad['ifac_violations']} violations",
          flush=True)

    ok = verdict("stock learned Retinue's destination over IFAC", bool(learned))
    ok &= verdict("Retinue tx_bytes equals stock rxb", ours["tx_bytes"] == stock["rxb"] > 0,
                  f"{ours['tx_bytes']} vs {stock['rxb']}")
    ok &= verdict("Retinue tx_packets equals stock frames in", ours["tx_packets"] == stock["frames_in"] > 0,
                  f"{ours['tx_packets']} vs {stock['frames_in']}")
    ok &= verdict("Retinue rx_bytes equals stock sealed bytes out",
                  ours["rx_bytes"] == stock["sealed_out"] > 0, f"{ours['rx_bytes']} vs {stock['sealed_out']}")
    ok &= verdict("Retinue rx_packets equals stock frames out", ours["rx_packets"] == stock["frames_out"] > 0,
                  f"{ours['rx_packets']} vs {stock['frames_out']}")
    ok &= verdict("no IFAC violations on the honest link", ours["ifac_violations"] == 0 == stock["ifac_violations"])
    ok &= verdict("Retinue counted every wrong-key frame as an IFAC violation",
                  injected >= INJECTED_ANNOUNCES and bad["ifac_violations"] == injected,
                  f"{bad['ifac_violations']} of {injected}")
    ok &= verdict("Retinue classified the honest announces and path request",
                  ours["announces_rx"] == 3 and ours["path_requests_rx"] == 1 and ours["announces_tx"] >= 2,
                  f"arx {ours['announces_rx']}, prx {ours['path_requests_rx']}, atx {ours['announces_tx']}")
    print(f"\n{TITLE}: {'PASS' if ok else 'FAIL'}", flush=True)
    kill_all()
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--node"]:
        node_main(sys.argv[2])
    else:
        raise SystemExit(main())
