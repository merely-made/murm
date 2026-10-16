"""RNS config mapping gate: one [[section]] text configures stock RNS 1.5.7 and retinue.

Two sections share their IFAC and mode keys: networkname, passphrase, ifac_size = 64 (bits,
so an 8-byte code where the TCP default is 16) and mode = gateway (Reticulum.py 843-1160).
  * server: retinue (examples/rns_config_peer.rs) runs the TCPServerInterface section and
    stock the TCPClientInterface one;
  * client: the same two texts with the roles swapped.
Each case passes on stock's own state: stock's interface holds an 8-byte IFAC and gateway
mode, stock has_path to retinue's announced destination, and stock's link to it goes ACTIVE.
Wrong IFAC size or credentials on either side drop every frame, so neither can happen.

Run: python -u interop_rns_config.py   (CARGO_TARGET_DIR as for the build)
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from ingress_harness import Child, REPO, free_port, verdict

EXAMPLE = "rns_config_peer"
EXPECTED_RNS = "1.5.7"
COMMON = ("    networkname = retinue-config-gate\n    passphrase = same-section-text\n"
          "    ifac_size = 64\n    mode = gateway\n")


def server_section(port: int) -> str:
    return ("  [[config-server]]\n    type = TCPServerInterface\n    enabled = yes\n"
            f"    listen_ip = 127.0.0.1\n    listen_port = {port}\n{COMMON}")


def client_section(port: int) -> str:
    return ("  [[config-client]]\n    type = TCPClientInterface\n    enabled = yes\n"
            f"    target_host = 127.0.0.1\n    target_port = {port}\n{COMMON}")


def binary() -> Path:
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
    return target / "debug" / "examples" / EXAMPLE


def case(name: str) -> int:
    import RNS

    assert RNS.__version__ == EXPECTED_RNS, RNS.__version__
    work = Path(os.environ["RETINUE_RNS_CONFIG_DIR"])
    port = free_port()
    retinue_text, stock_text = ((server_section(port), client_section(port)) if name == "server"
                                else (client_section(port), server_section(port)))
    print(f"Section text, retinue:\n{retinue_text}\nSection text, stock:\n{stock_text}", flush=True)
    (work / "retinue.conf").write_text(f"[interfaces]\n{retinue_text}", encoding="utf-8")
    stock_dir = work / "stock"
    stock_dir.mkdir()
    (stock_dir / "config").write_text(
        "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
        "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 3\n\n"
        f"[interfaces]\n{stock_text}", encoding="utf-8")

    retinue = None
    if name == "server":  # listen before stock dials
        retinue = Child([str(binary()), str(work / "retinue.conf")], "retinue")
        if retinue.wait_for(r"ATTACHED config-server", 30) is None:
            raise RuntimeError("retinue did not attach its server")
    RNS.Reticulum(configdir=str(stock_dir))
    iface = next(i for i in RNS.Transport.interfaces if "config-" in str(i))
    if retinue is None:
        retinue = Child([str(binary()), str(work / "retinue.conf")], "retinue")

    dest = retinue.wait_for(r"DEST <?([0-9a-f]{32})>?", 30)
    dest_hash = bytes.fromhex(dest.group(1)) if dest else b""
    deadline = time.monotonic() + 30
    while dest and not RNS.Transport.has_path(dest_hash) and time.monotonic() < deadline:
        time.sleep(0.2)
    has_path = bool(dest) and RNS.Transport.has_path(dest_hash)
    link = None
    if has_path:
        identity = RNS.Identity.recall(dest_hash)
        remote = RNS.Destination(identity, RNS.Destination.OUT, RNS.Destination.SINGLE,
                                 "retinue", "rns-config")
        link = RNS.Link(remote)
        deadline = time.monotonic() + 20
        while link.status != RNS.Link.ACTIVE and time.monotonic() < deadline:
            time.sleep(0.1)
    active = link is not None and link.status == RNS.Link.ACTIVE
    # On the server side stock's traffic rides the spawned interface, which inherits these.
    carriers = [iface, *(getattr(iface, "spawned_interfaces", None) or [])]
    ifac_ok = all(i.ifac_size == 8 and i.ifac_identity is not None for i in carriers)
    mode_ok = all(i.mode == RNS.Interfaces.Interface.Interface.MODE_GATEWAY for i in carriers)
    if link is not None:
        link.teardown()
    retinue.wait_for("HOLD_END", 10)
    retinue.kill()

    print("\n" + "=" * 72)
    print(f"case {name}: retinue runs config-{name}, stock the other section")
    ok = verdict("stock interfaces hold an 8-byte IFAC from ifac_size = 64", ifac_ok,
                 ", ".join(str(i.ifac_size) for i in carriers))
    ok &= verdict("stock interfaces are in gateway mode", mode_ok)
    ok &= verdict("stock has_path to retinue's destination", has_path)
    ok &= verdict("stock's link to retinue went ACTIVE", active,
                  str(getattr(link, "status", None)))
    ok &= verdict("retinue saw the link", bool(retinue.matches(r"LINK .*")))
    print("=" * 72, flush=True)
    RNS.exit(0 if ok else 1)
    return 0 if ok else 1


def main() -> int:
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue", "--example", EXAMPLE],
                   cwd=REPO, check=True)
    failed = []
    for name in ("server", "client"):
        work = tempfile.mkdtemp(prefix="retinue-rns-config-")
        try:
            child = subprocess.run([sys.executable, "-u", __file__, "--case", name],
                                   env={**os.environ, "RETINUE_RNS_CONFIG_DIR": work}, timeout=240)
            code = child.returncode
        except subprocess.TimeoutExpired:
            code = None
        finally:
            shutil.rmtree(work, ignore_errors=True)
        if code != 0:
            failed.append(name)
    print(f"RNS-CONFIG INTEROP: {'PASS' if not failed else 'FAIL (' + ', '.join(failed) + ')'}",
          flush=True)
    return 0 if not failed else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--case"]:
        raise SystemExit(case(sys.argv[2]))
    raise SystemExit(main())
