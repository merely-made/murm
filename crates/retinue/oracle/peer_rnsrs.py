"""Peer gate: rns-rs's daemon as the transport node between Retinue and stock RNS.

rns-rs (https://github.com/lelloman/rns-rs) is a Reticulum-licensed Rust port. This
driver runs its built `rnsd` as an untouched external process and reads none of its
source. Each case also runs against a Retinue hub and a stock hub, so a failure of the
harness itself shows up as a failed control rather than as an rns-rs finding.

    pair:      Retinue initiator <-> HUB <-> Retinue responder   (resolve, link, echo)
    leaf:      stock RNS leaf    <-> HUB <-> Retinue responder   (learn, link, echo)
    resource:  stock RNS leaf    <-> HUB <-> Retinue resource_recv (2.5 MB Resource)

rns-rs's node driver does not build for Windows, so the hub runs on a Unix host
reached over SSH. Its listener is forwarded to this machine and every socket in a case
is a loopback socket. Build the daemon there first, at the revision under test:

    cargo build -p rns-cli --bin rnsd --bin rnstatus --bin rnpath \\
        --no-default-features --features engine-parallel,iface-tcp

Run from the oracle/ directory:

    ./.venv/Scripts/python.exe -u peer_rnsrs.py --ssh-host HOST --rnsd PATH/TO/rnsd

A pass is a local TCP receipt for rns-rs as a transport node. It says nothing about
rns-rs as an endpoint, LXMF, radio carriers, IFAC, ratchets or loss.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import posixpath
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from ingress_harness import Child, free_port, verdict
from oracle_version import require_rns

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
EXAMPLES = ("link_peer", "transport_node", "resource_recv")
CASES = {"pair": ("retinue", "stock", "rnsrs"), "leaf": ("retinue", "stock", "rnsrs"),
         "resource": ("direct", "rnsrs")}
PAYLOAD_LEN = 2_500_000


def default_ssh() -> str:
    # Git for Windows' ssh cannot reach the Windows agent pipe; the system one can.
    system = Path(os.environ.get("SystemRoot", "")) / "System32" / "OpenSSH" / "ssh.exe"
    return str(system) if system.is_file() else "ssh"


def example(name: str) -> str:
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO.parent.parent / "target"))
    return str(target / "debug" / "examples" / name)


def hub_config(port: int, instance: str | None, client_port: int | None = None) -> str:
    shared = (f"  share_instance = Yes\n  instance_name = {instance}\n" if instance
              else "  share_instance = No\n")
    text = ("[reticulum]\n  enable_transport = Yes\n" + shared
            + "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 5\n\n[interfaces]\n"
            "  [[srv]]\n    type = TCPServerInterface\n    enabled = yes\n"
            f"    listen_ip = 127.0.0.1\n    listen_port = {port}\n")
    if client_port:
        text += ("  [[down]]\n    type = TCPClientInterface\n    enabled = yes\n"
                 f"    target_host = 127.0.0.1\n    target_port = {client_port}\n")
    return text


def leaf_config(port: int) -> Path:
    cfg = Path(tempfile.mkdtemp(prefix="retinue-rnsrs-leaf-"))
    (cfg / "config").write_text(
        "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
        "  panic_on_interface_error = No\n\n[logging]\n  loglevel = 4\n\n[interfaces]\n"
        "  [[hub]]\n    type = TCPClientInterface\n    enabled = yes\n"
        f"    target_host = 127.0.0.1\n    target_port = {port}\n", encoding="utf-8")
    return cfg


class Hub:
    """A transport node listening on 127.0.0.1:port, from one of three implementations."""

    def __init__(self, kind: str, args, port: int, reverse_to: int | None = None) -> None:
        self.kind, self.args, self.remote_dir, self.tmp = kind, args, None, None
        if kind == "retinue":
            self.proc = Child([example("transport_node")], "hub", {"RETINUE_PORT": str(port)})
            up = self.proc.wait_for(r"TRANSPORT_NODE_UP .*", 30)
        elif kind == "stock":
            self.tmp = Path(tempfile.mkdtemp(prefix="retinue-rnsrs-stockhub-"))
            (self.tmp / "config").write_text(hub_config(port, None), encoding="utf-8")
            # Unbuffered, or the piped start line arrives only when the daemon exits.
            self.proc = Child([str(Path(sys.executable).with_name("rnsd")), "--config", str(self.tmp), "-v"],
                              "hub", {"PYTHONUNBUFFERED": "1"})
            up = self.proc.wait_for(r".*Started rnsd.*", 40)
        else:
            self.remote_dir = f"/tmp/retinue-rnsrs-{os.getpid()}"
            client = free_port() if reverse_to else None
            cfg = hub_config(port, f"retinue_rnsrs_{os.getpid()}", client)
            encoded = base64.b64encode(cfg.encode()).decode()
            remote = (f"mkdir -p {self.remote_dir} && echo {encoded} | base64 -d > {self.remote_dir}/config"
                      f" && exec {args.rnsd} --config {self.remote_dir} -v")
            argv = [args.ssh, "-tt", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes",
                    "-L", f"{port}:127.0.0.1:{port}"]
            if reverse_to:
                argv += ["-R", f"{client}:127.0.0.1:{reverse_to}"]
            self.proc = Child(argv + [args.ssh_host, remote], "hub")
            up = self.proc.wait_for(r".*Started rnsd.*", 40)
        if up is None:
            self.close()
            raise RuntimeError(f"{kind} hub did not come up")
        time.sleep(1.0)

    def remote(self, command: str) -> str:
        done = subprocess.run([self.args.ssh, "-o", "BatchMode=yes", self.args.ssh_host, command],
                              capture_output=True, text=True, timeout=40, errors="replace")
        return (done.stdout + done.stderr).strip()

    def tool(self, name: str, *arguments: str) -> str:
        """Run one of rns-rs's utilities against the hub's shared instance."""
        binary = posixpath.join(posixpath.dirname(self.args.rnsd), name)
        return self.remote(f"{binary} --config {self.remote_dir} {' '.join(arguments)}")

    def close(self) -> None:
        self.proc.kill()
        if self.remote_dir:
            # The bracket keeps pkill from matching the shell that carries this command.
            self.remote(f"pkill -f '[r]nsd --config {self.remote_dir}'; rm -rf {self.remote_dir}")
        if self.tmp:
            shutil.rmtree(self.tmp, ignore_errors=True)


def case_pair(kind: str, args) -> tuple[dict, dict]:
    port = free_port()
    hub = Hub(kind, args, port)
    extra: dict = {}
    try:
        addr = {"RETINUE_ADDR": f"127.0.0.1:{port}"}
        init = Child([example("link_peer")], "init", dict(addr, RETINUE_ROLE="initiator"))
        time.sleep(0.4)
        resp = Child([example("link_peer")], "resp", dict(addr, RETINUE_ROLE="responder"))
        init.wait_for(r"DONE initiator", 45)
        if kind == "rnsrs":
            dest = resp.matches(r"RESPONDER_DEST <?([0-9a-f]{32})>?")
            if dest:
                extra["rnpath"] = (hub.tool("rnpath", dest[0].group(1)).splitlines() or [""])[-1]
        checks = {
            "initiator resolved the responder through the hub": bool(init.matches("RESOLVED_RESPONDER")),
            "link established": bool(init.matches(r"LINKED .*")),
            "echo returned": bool(init.matches("ECHO_OK")),
            "responder accepted": bool(resp.matches("RESPONDER_ACCEPTED")),
        }
        if "rnpath" in extra:
            checks["rns-rs rnpath finds the responder"] = "Path found" in extra["rnpath"]
        return checks, extra
    finally:
        hub.close()


def case_leaf(kind: str, args) -> tuple[dict, dict]:
    import RNS
    require_rns(RNS.__version__)
    port = free_port()
    hub = Hub(kind, args, port)
    got: dict = {}
    done = threading.Event()
    cfg = leaf_config(port)
    try:
        RNS.Reticulum(configdir=str(cfg))

        class Reach:
            aspect_filter = "retinue.peer"

            def received_announce(self, destination_hash, announced_identity, app_data):
                if "hops" in got:
                    return
                got["hops"] = RNS.Transport.hops_to(destination_hash)
                out = RNS.Destination(announced_identity, RNS.Destination.OUT,
                                      RNS.Destination.SINGLE, "retinue", "peer")
                link = RNS.Link(out)

                def established(l):
                    got["established"] = True

                    def on_packet(message, packet):
                        got["echo"] = bytes(message)
                        done.set()
                    l.set_packet_callback(on_packet)
                    RNS.Packet(l, b"ping-rns").send()
                link.set_link_established_callback(established)

        RNS.Transport.register_announce_handler(Reach())
        Child([example("link_peer")], "resp",
              {"RETINUE_ROLE": "responder", "RETINUE_ADDR": f"127.0.0.1:{port}"})
        done.wait(timeout=30)
        time.sleep(0.5)
        checks = {
            "stock leaf learned the responder at two hops": got.get("hops") == 2,
            "link established": got.get("established", False),
            "echo returned": got.get("echo") == b"echo:ping-rns",
        }
        return checks, {"hops": got.get("hops")}
    finally:
        hub.close()
        shutil.rmtree(cfg, ignore_errors=True)


def case_resource(kind: str, args) -> tuple[dict, dict]:
    import RNS
    require_rns(RNS.__version__)
    payload = bytes(((i * 2654435761) >> 8) & 0xFF for i in range(PAYLOAD_LEN))
    digest = hashlib.sha256(payload).hexdigest()
    recv = Child([example("resource_recv")], "retinue")
    listening = recv.wait_for(r"LISTENING (\d+)", 30)
    if listening is None:
        raise RuntimeError("resource_recv did not listen")
    listen = int(listening.group(1))
    hub = None
    port = listen
    if kind == "rnsrs":
        port = free_port()
        hub = Hub("rnsrs", args, port, reverse_to=listen)
    got: dict = {}
    done = threading.Event()
    cfg = leaf_config(port)
    try:
        RNS.Reticulum(configdir=str(cfg))

        class Linker:
            aspect_filter = "retinue.resource"

            def received_announce(self, destination_hash, announced_identity, app_data):
                if "hops" in got:
                    return
                got["hops"] = RNS.Transport.hops_to(destination_hash)
                out = RNS.Destination(announced_identity, RNS.Destination.OUT,
                                      RNS.Destination.SINGLE, "retinue", "resource")
                link = RNS.Link(out)

                def established(l):
                    started = time.monotonic()

                    def concluded(resource):
                        got["status"] = resource.status
                        got["seconds"] = round(time.monotonic() - started, 1)
                        done.set()
                    RNS.Resource(payload, l, callback=concluded, auto_compress=False)
                link.set_link_established_callback(established)

        RNS.Transport.register_announce_handler(Linker())
        done.wait(timeout=150)
        recv.wait_for(r"DATA_HASH ([0-9a-f]+)", 10)
        received = recv.matches(r"DATA_HASH ([0-9a-f]+)")
        checks = {
            "stock leaf learned the Retinue destination": "hops" in got,
            "stock sender concluded COMPLETE": got.get("status") == RNS.Resource.COMPLETE,
            "Retinue reassembled the payload intact": bool(received and received[0].group(1) == digest),
        }
        return checks, {"hops": got.get("hops"), "seconds": got.get("seconds"), "bytes": PAYLOAD_LEN}
    finally:
        if hub:
            hub.close()
        shutil.rmtree(cfg, ignore_errors=True)


def run_one(case: str, kind: str, args) -> int:
    checks, extra = {"pair": case_pair, "leaf": case_leaf, "resource": case_resource}[case](kind, args)
    for label, ok in checks.items():
        verdict(label, ok)
    ok = all(checks.values())
    print("RESULT " + json.dumps({"case": case, "hub": kind, "pass": ok, "checks": checks, "extra": extra}),
          flush=True)
    code = 0 if ok else 1
    if "RNS" in sys.modules:
        sys.modules["RNS"].exit(code)
    return code


def run_all(args) -> int:
    subprocess.run(["cargo", "build", "--quiet", "-p", "retinue",
                    *(part for name in EXAMPLES for part in ("--example", name))], cwd=REPO, check=True)
    probe = [args.ssh, "-o", "BatchMode=yes", args.ssh_host]
    record = {
        "retinue_revision": subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, capture_output=True,
                                           text=True, check=True).stdout.strip(),
        "rnsd_version": subprocess.run(probe + [f"{args.rnsd} --version"], capture_output=True,
                                       text=True, check=True, timeout=40).stdout.strip(),
        "results": [],
    }
    print(f"rns-rs daemon: {record['rnsd_version']}\n")
    for case in args.case or CASES:
        for kind in CASES[case]:
            print(f"\n{'=' * 72}\nCASE {case} / hub {kind}\n{'=' * 72}", flush=True)
            # One process per case: stock RNS owns process-global state.
            done = subprocess.run([sys.executable, "-u", __file__, "--ssh", args.ssh,
                                   "--ssh-host", args.ssh_host, "--rnsd", args.rnsd, "--one", case, kind],
                                  cwd=HERE, capture_output=True, text=True, errors="replace")
            sys.stdout.write(done.stdout)
            line = next((l for l in done.stdout.splitlines() if l.startswith("RESULT ")), None)
            record["results"].append(json.loads(line[7:]) if line
                                     else {"case": case, "hub": kind, "pass": False, "error": done.stderr[-2000:]})
    failed = [f"{r['case']}/{r['hub']}" for r in record["results"] if not r["pass"]]
    if args.output:
        args.output.mkdir(parents=True, exist_ok=True)
        (args.output / "result.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print("\n" + "=" * 72)
    count = len(record["results"])
    print(f"PEER RNS-RS: FAIL ({', '.join(failed)})" if failed else f"PEER RNS-RS: PASS ({count} cases)")
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ssh-host", required=True, help="Unix host that holds the built rns-rs daemon")
    parser.add_argument("--rnsd", required=True, help="path of rns-rs's rnsd on that host")
    parser.add_argument("--ssh", default=default_ssh(), help="ssh executable")
    parser.add_argument("--case", action="append", choices=sorted(CASES), help="default: every case")
    parser.add_argument("--output", type=Path, help="directory for result.json")
    parser.add_argument("--one", nargs=2, metavar=("CASE", "HUB"), help=argparse.SUPPRESS)
    args = parser.parse_args()
    return run_one(*args.one, args) if args.one else run_all(args)


if __name__ == "__main__":
    raise SystemExit(main())
