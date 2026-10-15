"""Does tunnel synthesis keep a reconnecting client's paths, both ways (review B6)?

Forward: retinue (tunnel_peer) dials a stock transport hub H with tunnel synthesis and
announces once; a stock client C sits behind H. H closes retinue's spawned interface and
retinue redials. Passes when H's RNS.Transport.tunnels holds sha256(pubkey + interface_hash),
H routes retinue's destination over the tunnel's new interface with no second announce, and C
opens an ACTIVE link to retinue (Transport.py 2764-2860; TCPInterface.py 169-180, 298-299).

Reverse: a stock transport client S dials a retinue listener and announces once; a stock node
N behind retinue learns the path. S's connection is closed and S reconnects with synthesis;
N, still holding its old path, opens an ACTIVE link to S through retinue, and S has sent no
second announce or path response.

Run: CARGO_TARGET_DIR=... ./.venv/bin/python -u interop_tunnel.py
"""
from __future__ import annotations

import hashlib
import re

from ingress_harness import free_port, verdict, wait_until
from interop_shared_instance import Node, ask, example, field

NO_SHARE = "  share_instance = No\n"


def client(name: str, port: int) -> str:
    return (f"  [[{name}]]\n    type = TCPClientInterface\n    enabled = yes\n"
            f"    target_host = 127.0.0.1\n    target_port = {port}\n")


def counters(retinue) -> tuple[int, int]:
    m = ask(retinue, "counters", r"COUNTERS (\d+) (\d+)")
    return (int(m.group(1)), int(m.group(2))) if m else (0, 0)


def forward() -> bool:
    port = free_port()
    hub = Node("hub", NO_SHARE + "  enable_transport = Yes\n",
               "  [[server]]\n    type = TCPServerInterface\n    enabled = yes\n"
               f"    listen_ip = 127.0.0.1\n    listen_port = {port}\n")
    c = Node("c", NO_SHARE + "  enable_transport = No\n", client("hub", port))
    retinue = example("tunnel_peer", {"RETINUE_HUB": f"127.0.0.1:{port}"})
    pubkey, label, dest = bytes.fromhex(field(retinue, "PUBKEY")), field(retinue, "LABEL"), field(retinue, "DEST")
    tunnel = hashlib.sha256(pubkey + hashlib.sha256(label.encode()).digest()).hexdigest()

    ok = verdict("H holds retinue's tunnel id", bool(wait_until(lambda: tunnel in hub.call("tunnels"), 15, 0.3)),
                 tunnel[:16])
    ok &= verdict("H learns retinue's destination over the tunnel",
                  bool(wait_until(lambda: hub.call("via_tunnel", dest=dest, tunnel=tunnel), 15, 0.3)))
    wait_until(lambda: c.call("has_path", dest=dest), 15, 0.3)
    announced = hub.call("announced", dest=dest)
    old = hub.call("close_tunnel_iface", tunnel=tunnel)
    print(f"  closed {old}")
    redialed = wait_until(lambda: (hub.call("tunnels").get(tunnel) not in (None, "None", old)
                                   and hub.call("via_tunnel", dest=dest, tunnel=tunnel)), 20, 0.3)
    ok &= verdict("after the redial H routes retinue over the tunnel's new interface", bool(redialed),
                  hub.call("tunnels").get(tunnel, "gone"))
    ok &= verdict("H has_path(retinue) with no new announce",
                  hub.call("has_path", dest=dest) and hub.call("announced", dest=dest) == announced)
    synthesized, _ = counters(retinue)
    ok &= verdict("retinue synthesized on both connects", synthesized >= 2, str(synthesized))
    c.call("link", dest=dest, aspect="tunnel")
    ok &= verdict("C's link to retinue is ACTIVE", bool(wait_until(lambda: any(c.call("links")), 15, 0.3)))
    for child in (retinue, c, hub):
        child.kill()
    return ok


def reverse() -> bool:
    retinue = example("tunnel_peer", {"RETINUE_LISTEN": "0"})
    port = int(field(retinue, "LISTENING"))
    s = Node("s", NO_SHARE + "  enable_transport = Yes\n", client("retinue", port))
    n = Node("n", NO_SHARE + "  enable_transport = No\n", client("retinue", port))
    ds = s.call("register", aspect="s")
    s.call("announce")
    ok = verdict("N learns S's destination through retinue",
                 bool(wait_until(lambda: n.call("has_path", dest=ds), 15, 0.3)))
    sent = s.call("announced", dest=ds)
    s.call("close_client", name="retinue")
    back = wait_until(lambda: counters(retinue)[1] > 0, 20, 0.5)
    ok &= verdict("retinue restored S's path on reconnect", bool(back), f"restored {counters(retinue)[1]}")
    ok &= verdict("S is back online", bool(wait_until(lambda: s.call("online", name="retinue"), 10, 0.3)))
    ok &= verdict("N still holds its old path", n.call("has_path", dest=ds))
    n.call("link", dest=ds, app="localgate", aspect="s")
    ok &= verdict("N's link to S through retinue is ACTIVE",
                  bool(wait_until(lambda: any(n.call("links")), 15, 0.3)))
    ok &= verdict("S sent no second announce or path response", s.call("announced", dest=ds) == sent,
                  f"{sent} -> {s.call('announced', dest=ds)}")
    for child in (retinue, s, n):
        child.kill()
    return ok


def main() -> int:
    print("forward: retinue dials a stock hub")
    ok = forward()
    print("reverse: a stock client dials retinue")
    ok &= reverse()
    print(f"\nB6 TUNNEL-SYNTHESIS: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
