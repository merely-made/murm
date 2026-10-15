"""Do two stock RNS peers keep a working link MTU through a retinue transport node?

Topology:  RNS initiator  <->  retinue transport node (routing)  <->  RNS responder

Both RNS peers use TCP with link MTU discovery on, so the initiator signals its interface's
hardware MTU in the link request, and each transport hop clamps that signal to what it can
carry (Transport.py 2063-2088). The retinue node's TCP interfaces autoconfigure 16384 bytes.
A relay that forwarded a larger signal unchanged let the peers agree on frames it then
refused, so the link came up and every large frame died.

Cases, each linking the two RNS peers through the retinue node and sending a resource
larger than 64 KiB, passing only if both ends hold the expected MTU and the transfer
completes intact:
  * tcp:   stock TCP interfaces (16384), so 16384 end to end;
  * clamp: both peers set fixed_mtu = 65536, so only retinue's clamp keeps the link at 16384.
It also exercises the relay's bridge validation: the proof must pass the relay's signature
check before any link traffic is carried.

Run from the oracle/ directory:  python -u interop_transit_mtu.py
"""
from __future__ import annotations
import atexit, hashlib, os, shutil, subprocess, sys, tempfile, threading, time
from pathlib import Path
import RNS
from ingress_harness import free_port

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
APP, ASPECT = "retinue_gate", "transit_mtu"
PAYLOAD_LEN = 200 * 1024
EXPECTED_MTU = 16384
CASES = {"tcp": None, "clamp": 65536}


def rns_config(prefix: str) -> Path:
    port, fixed = os.environ["RETINUE_HUB_PORT"], os.environ.get("RETINUE_FIXED_MTU")
    fixed_line = f"    fixed_mtu = {fixed}\n" if fixed else ""
    cfg = Path(tempfile.mkdtemp(prefix=prefix))
    (cfg / "config").write_text(
        "[reticulum]\n  enable_transport = No\n  share_instance = No\n"
        "  panic_on_interface_error = No\n  link_mtu_discovery = Yes\n"
        "\n[logging]\n  loglevel = 3\n\n[interfaces]\n  [[hub]]\n    type = TCPClientInterface\n"
        f"    enabled = yes\n    target_host = 127.0.0.1\n    target_port = {port}\n{fixed_line}",
        encoding="utf-8")
    atexit.register(shutil.rmtree, cfg, ignore_errors=True)
    return cfg


def responder() -> int:
    """The far RNS peer: announce until linked, accept one resource, report it."""
    RNS.Reticulum(configdir=str(rns_config("retinue-transit-resp-")))
    identity = RNS.Identity()
    dest = RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE, APP, ASPECT)
    linked = threading.Event()
    finished = threading.Event()

    def concluded(resource):
        data = resource.data.read() if resource.status == RNS.Resource.COMPLETE else b""
        print(f"RESPONDER_RESOURCE status={resource.status} len={len(data)} "
              f"sha={hashlib.sha256(data).hexdigest()} mtu={resource.link.get_mtu()}", flush=True)
        finished.set()

    def established(link):
        linked.set()
        print(f"RESPONDER_LINK mtu={link.get_mtu()}", flush=True)
        link.set_resource_strategy(RNS.Link.ACCEPT_ALL)
        link.set_resource_concluded_callback(concluded)

    dest.set_link_established_callback(established)
    deadline = time.time() + 150
    while not linked.is_set() and time.time() < deadline:
        dest.announce()
        linked.wait(3)
    finished.wait(max(0, deadline - time.time()))
    time.sleep(1)
    RNS.exit(0)
    return 0


def spawn(cmd, env_extra, sink, label, cwd):
    env = dict(os.environ, **env_extra)
    p = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                         text=True, bufsize=1, env=env)
    def pump():
        for line in p.stdout:
            sink.append(line.rstrip()); print(f"  [{label}] {line.rstrip()}")
    threading.Thread(target=pump, daemon=True).start()
    return p


def field(line: str, key: str) -> str:
    return next(part.split("=", 1)[1] for part in line.split() if part.startswith(key + "="))


def run_case(name: str) -> int:
    print(f"RNS {RNS.__version__}, case {name}\n")
    sink: list[str] = []
    hub = spawn(["cargo", "run", "--quiet", "--example", "transport_node"],
                {"RETINUE_PORT": os.environ["RETINUE_HUB_PORT"]}, sink, "hub", REPO)
    deadline = time.time() + 180
    while time.time() < deadline and not any("TRANSPORT_NODE_UP" in l for l in sink):
        if hub.poll() is not None:
            print("hub exited"); return 1
        time.sleep(0.3)
    time.sleep(0.5)

    RNS.Reticulum(configdir=str(rns_config("retinue-transit-init-")))
    got: dict = {}
    done = threading.Event()
    payload = os.urandom(PAYLOAD_LEN)

    class Reach:
        aspect_filter = f"{APP}.{ASPECT}"
        def received_announce(self, destination_hash, announced_identity, app_data):
            if "link" in got:
                return
            got["link"] = True
            print(f"  RNS: learned {destination_hash.hex()} via the retinue node; linking")
            out = RNS.Destination(announced_identity, RNS.Destination.OUT,
                                  RNS.Destination.SINGLE, APP, ASPECT)
            def est(link):
                got["mtu"] = link.get_mtu()
                print(f"  RNS: link up through the retinue node, MTU {got['mtu']}")
                def sent(resource):
                    got["status"] = resource.status
                    print(f"  RNS: resource concluded with status {resource.status}")
                    done.set()
                RNS.Resource(payload, link, callback=sent)
            RNS.Link(out, established_callback=est)
    RNS.Transport.register_announce_handler(Reach())

    resp = spawn([sys.executable, "-u", str(Path(__file__).resolve()), "--responder"],
                 {}, sink, "resp", HERE)
    done.wait(timeout=150)
    end = time.time() + 10
    while time.time() < end and not any(l.startswith("RESPONDER_RESOURCE") for l in sink):
        time.sleep(0.2)
    for p in (resp, hub):
        try: p.kill()
        except Exception: pass

    report = next((l for l in sink if l.startswith("RESPONDER_RESOURCE")), None)
    init_mtu = got.get("mtu")
    resp_mtu = int(field(report, "mtu")) if report else None
    sent_ok = got.get("status") == RNS.Resource.COMPLETE
    recv_ok = (report is not None and int(field(report, "status")) == RNS.Resource.COMPLETE
               and int(field(report, "len")) == PAYLOAD_LEN
               and field(report, "sha") == hashlib.sha256(payload).hexdigest())
    mtu_ok = init_mtu == EXPECTED_MTU and resp_mtu == EXPECTED_MTU
    print("\n" + "=" * 68)
    print(f"Link MTU, initiator / responder ({EXPECTED_MTU}):   {init_mtu} / {resp_mtu}")
    print(f"{PAYLOAD_LEN} B resource completed at the sender:     {sent_ok}")
    print(f"Resource received intact by the RNS responder:  {recv_ok}")
    print("=" * 68)
    ok = mtu_ok and sent_ok and recv_ok
    print(f"TRANSIT-MTU RNS-LINK-THROUGH-RETINUE [{name}]: {'PASS' if ok else 'FAIL'}")
    exit_code = 0 if ok else 1
    RNS.exit(exit_code)
    return exit_code


def main() -> int:
    subprocess.run(["cargo", "build", "--quiet", "--example", "transport_node"], cwd=REPO, check=True)
    failed = []
    for name, fixed in CASES.items():
        env = dict(os.environ, RETINUE_HUB_PORT=str(free_port()))
        if fixed:
            env["RETINUE_FIXED_MTU"] = str(fixed)
        child = subprocess.run([sys.executable, "-u", __file__, "--case", name], env=env,
                               timeout=300)
        if child.returncode != 0:
            failed.append(name)
    print(f"TRANSIT-MTU: {'PASS' if not failed else 'FAIL (' + ', '.join(failed) + ')'}", flush=True)
    return 0 if not failed else 1


if __name__ == "__main__":
    if "--responder" in sys.argv[1:]:
        raise SystemExit(responder())
    if sys.argv[1:2] == ["--case"]:
        raise SystemExit(run_case(sys.argv[2]))
    raise SystemExit(main())
