"""Auto sniff gate, no root: Retinue hears stock AutoInterface tokens on a real NIC.

Stock runs an AutoInterface on DEVICE (en1 on the reference host); Retinue's `auto_sniff`
joins the same discovery group beside it and must receive 3 tokens, each equal to its own
`peering_token` over the link-local it adopts. Stock's own adopted address must be the
same one. That proves the group address, port, canonical text and token live. Then Retinue
runs a whole AutoInterface under a private group and must see its own multicast come back.

The run repeats with group `retinue-gate` at admin scope. Stock hashes its link-local but
sends from an unbound socket (`AutoInterface.py` 498-511), and for a group wider than the
link the kernel picks a wider source when the NIC has one (a global or ULA address), so
stock's tokens fail authentication everywhere, its own echo check included ("No multicast
echoes received"). The gate then takes the token from any source and requires stock's echo
check to agree: stock echoed exactly when its tokens left from its link-local. Retinue
sends from its link-local at every scope, so its own echo must still come back.

Both sides share the host's link-local, so they see each other as echoes and cannot peer:
that needs two links (interop_auto_feth.py). On macOS the application firewall must admit
incoming traffic for the oracle Python and the example binary, or no token arrives.

    .venv/bin/python -u interop_auto_sniff.py [DEVICE]
"""

from __future__ import annotations

import ipaddress
import sys

from datagram_gate import conclude, config_dir, free_udp_port, supervise, wait
from pty_bridge import Retinue, rns_config

SCENARIOS = {"default": (None, None), "gate-admin": ("retinue-gate", "admin")}


def sniff(name: str, device: str) -> dict[str, bool]:
    import RNS

    group, scope = SCENARIOS[name]
    keys = (f"    group_id = {group}\n" if group else "") + (
        f"    discovery_scope = {scope}\n" if scope else "")
    rns_config(config_dir(), "  [[auto]]\n    type = AutoInterface\n    enabled = yes\n"
               f"    devices = {device}\n    data_port = {free_udp_port()}\n" + keys)
    RNS.Reticulum(configdir=str(config_dir()))
    stock = next(i for i in RNS.Transport.interfaces if "auto" in str(i))
    args = [device, "--count", "3", "--self-test"] + (["--group", group] if group else []) + (
        ["--scope", scope, "--any-source"] if scope else [])
    retinue = Retinue("auto_sniff", "auto", args)
    try:
        adopted = retinue.wait_for(r"^ADOPTED (\S+) (\d+) (\S+) GROUP (\S+)", 300)
        results = {"retinue adopted the device": adopted is not None}
        if adopted is None:
            return results
        results["same link-local as stock adopted"] = (
            stock.adopted_interfaces.get(device) == adopted.group(3))
        results["same discovery group as stock"] = (
            adopted.group(4) == str(ipaddress.IPv6Address(stock.mcast_discovery_address)))
        ok = retinue.wait_for(r"^SNIFF_OK 3", 40) is not None
        results["3 stock tokens matched peering_token"] = ok
        if not ok:
            if retinue.wait_for(r"^TOKEN ", 1) is None:
                print("no token arrived: is the macOS application firewall blocking it?", flush=True)
            return results
        tokens = [line.split() for line in retinue.lines if line.startswith("TOKEN ")]
        own = all(t[3] == "MATCH" for t in tokens if t[3] != "OTHER")
        if name == "default":
            results["stock sent from its link-local"] = own
        print(f"  stock token sources: {sorted({t[1] for t in tokens if t[3] != 'OTHER'})}",
              flush=True)
        echoed = wait(lambda: device in stock.initial_echoes, 5 if own else 1)
        results[f"stock's echo check agrees (echoed={echoed})"] = echoed == own
        echo = retinue.wait_for(r"^SELF_TEST echoed=(\w+) peers=(\d+)", 30)
        results["retinue's own AutoInterface heard its echo"] = bool(echo) and echo.group(1) == "true"
        return results
    finally:
        retinue.close()


def main() -> int:
    if sys.argv[1:2] == ["--child"]:
        return conclude(f"AUTO SNIFF {sys.argv[2]}", lambda: sniff(sys.argv[2], sys.argv[3]))
    device = sys.argv[1] if len(sys.argv) > 1 else "en1"
    return supervise(__file__, "AUTO SNIFF", "auto_sniff", [[n, device] for n in SCENARIOS])


if __name__ == "__main__":
    raise SystemExit(main())
