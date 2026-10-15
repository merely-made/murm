"""Link MTU autoconfiguration gate: stock RNS 1.5.7 <-> retinue over TCP.

A stock TCP interface autoconfigures HW_MTU 16384 at its 10 Mbit/s guess and signals it in
link requests (Interface.py 251-265; Link.py 300-315; Transport.py 2543-2563, 3173-3178).
Each case moves a 1 MiB Resource each way over one link and passes on stock's own state:
  * stock's link reports get_mtu() == 16384 (500 in the baseline, where retinue sets a
    500-byte link MTU ceiling);
  * stock's outbound Resource reaches COMPLETE (retinue proved it);
  * stock's inbound Resource concludes COMPLETE with retinue's exact bytes.
Cases: stock initiates (TCPClientInterface -> retinue listener), and retinue initiates
(examples/link_mtu_peer.rs dialling a stock TCPServerInterface). Throughput is reported
against the 500-byte baseline.

Run: python -u interop_link_mtu.py   (CARGO_TARGET_DIR as for the build)
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import shutil
import threading
import time
from pathlib import Path

from ingress_harness import Child, REPO, free_port, tcp, verdict
from library_gate import RESOURCE_STATUS, payload

EXAMPLE = "link_mtu_peer"
EXPECTED_RNS = "1.5.7"
RESOURCE_LEN = 1024 * 1024
SEED = 0x4D7  # examples/link_mtu_peer.rs: stock sends SEED, retinue sends SEED + 1
TCP_MTU = 16384
CASES = [("stock", None), ("stock", 500), ("retinue", None), ("retinue", 500)]


def binary() -> Path:
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
    return target / "debug" / "examples" / EXAMPLE


def case(initiator: str, ceiling: int | None) -> int:
    import RNS

    assert RNS.__version__ == EXPECTED_RNS, RNS.__version__
    port = free_port()
    config = Path(os.environ["RETINUE_LINK_MTU_CONFIG"])
    iface = (tcp("client", "mtu", port) if initiator == "stock" else tcp("server", "mtu", port))
    (config / "config").write_text(
        "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
        "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 3\n\n"
        f"[interfaces]\n{iface}", encoding="utf-8")
    args = ["accept" if initiator == "stock" else "open", str(port)]
    args += [str(ceiling)] if ceiling else []
    if initiator == "stock":
        # Listen first, so stock's client connects on its first attempt.
        retinue = Child([str(binary()), *args], "retinue")
        if retinue.wait_for("LISTENING", 30) is None:
            raise RuntimeError("link_mtu_peer did not listen")
    RNS.Reticulum(configdir=str(config))
    outbound, expected = payload(RESOURCE_LEN, SEED), payload(RESOURCE_LEN, SEED + 1)
    state: dict = {}
    sent, received = threading.Event(), threading.Event()

    def send(link):
        state["sent_at"] = time.monotonic()
        def done(resource):
            state["sent"] = (resource.status, time.monotonic() - state["sent_at"])
            sent.set()
        state["resource"] = RNS.Resource(outbound, link, callback=done)

    def concluded(resource):
        data = resource.data.read() if resource.status == RNS.Resource.COMPLETE else b""
        state["received"] = (resource.status, data == expected)
        received.set()
        if initiator == "retinue":
            send(resource.link)

    def established(link):
        state["mtu"] = link.get_mtu()
        print(f"  RNS: link ACTIVE, MTU {state['mtu']}", flush=True)
        link.set_resource_strategy(RNS.Link.ACCEPT_ALL)
        link.set_resource_concluded_callback(concluded)
        if initiator == "stock":
            send(link)

    if initiator == "stock":
        class Linker:
            aspect_filter = "retinue.link-mtu"

            def received_announce(self, destination_hash, announced_identity, app_data):
                if "link" not in state:
                    remote = RNS.Destination(announced_identity, RNS.Destination.OUT,
                                             RNS.Destination.SINGLE, "retinue", "link-mtu")
                    state["link"] = RNS.Link(remote, established_callback=established)

        RNS.Transport.register_announce_handler(Linker())
    else:
        dest = RNS.Destination(RNS.Identity(), RNS.Destination.IN, RNS.Destination.SINGLE,
                               "rns", "link-mtu")
        def inbound(link):
            state["link"] = link
            established(link)

        dest.set_link_established_callback(inbound)
        retinue = Child([str(binary()), *args], "retinue")
        deadline = time.monotonic() + 60
        while "link" not in state and time.monotonic() < deadline:
            dest.announce()
            time.sleep(1)

    sent.wait(180)
    received.wait(180)
    link = state.get("link")
    if link is not None:
        link.teardown()
    try:
        code = retinue.proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        retinue.kill()
        code = None

    want = ceiling or TCP_MTU
    sent_status, sent_secs = state.get("sent", (None, 0.0))
    recv_status, recv_ok = state.get("received", (None, False))
    rate = RESOURCE_LEN / sent_secs / 1024 if sent_secs else 0.0
    label = f"{initiator} initiates, {'ceiling ' + str(ceiling) if ceiling else 'autoconfigured'}"
    print("\n" + "=" * 72)
    print(f"{label}: stock sent 1 MiB in {sent_secs:.2f} s ({rate:.0f} KiB/s)")
    published = retinue.matches(r"PUBLISH_OK \d+ ([0-9.]+)")
    back_secs = float(published[0].group(1)) if published else 0.0
    print(f"{label}: retinue sent 1 MiB in {back_secs:.2f} s")
    print(f"THROUGHPUT {initiator} {want} {sent_secs:.3f} {back_secs:.3f}", flush=True)
    ok = verdict(f"stock link MTU is {want}", state.get("mtu") == want, str(state.get("mtu")))
    ok &= verdict("stock's 1 MiB Resource reached COMPLETE", sent_status == RNS.Resource.COMPLETE,
                  RESOURCE_STATUS.get(sent_status, str(sent_status)))
    ok &= verdict("stock received retinue's 1 MiB Resource COMPLETE and intact",
                  recv_status == RNS.Resource.COMPLETE and recv_ok,
                  RESOURCE_STATUS.get(recv_status, str(recv_status)))
    ok &= verdict("retinue saw the same MTU", bool(retinue.matches(f"LINK_MTU {want}")))
    ok &= verdict("retinue exited zero", code == 0, str(code))
    print("=" * 72, flush=True)
    RNS.exit(0 if ok else 1)
    return 0 if ok else 1


def main() -> int:
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--example", EXAMPLE],
                   cwd=REPO, check=True)
    failed, times = [], {}
    for initiator, ceiling in CASES:
        config = tempfile.mkdtemp(prefix="retinue-link-mtu-")
        args = [initiator, str(ceiling or 0)]
        child = subprocess.Popen([sys.executable, "-u", __file__, "--case", *args],
                                 env={**os.environ, "RETINUE_LINK_MTU_CONFIG": config},
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            out, _ = child.communicate(timeout=480)
        except subprocess.TimeoutExpired:
            child.kill()
            out, _ = child.communicate()
        finally:
            shutil.rmtree(config, ignore_errors=True)
        print(out, end="", flush=True)
        for line in out.splitlines():
            if line.startswith("THROUGHPUT "):
                _, who, mtu, *secs = line.split()
                times[(who, int(mtu))] = [float(x) for x in secs]
        if child.returncode != 0:
            failed.append(" ".join(args))
    for who in ("stock", "retinue"):
        fast, base = times.get((who, TCP_MTU)), times.get((who, 500))
        for side, index in (("stock", 0), ("retinue", 1)):
            if fast and base and fast[index] and base[index]:
                print(f"{who} initiator, {side} sends: 1 MiB in {fast[index]:.2f} s at {TCP_MTU} "
                      f"vs {base[index]:.2f} s at 500 ({base[index] / fast[index]:.1f}x)", flush=True)
    print(f"LINK-MTU INTEROP: {'PASS' if not failed else 'FAIL (' + ', '.join(failed) + ')'}",
          flush=True)
    return 0 if not failed else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--case"]:
        raise SystemExit(case(sys.argv[2], int(sys.argv[3]) or None))
    raise SystemExit(main())
