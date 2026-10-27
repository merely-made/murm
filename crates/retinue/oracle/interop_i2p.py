"""I2P4: stock RNS I2PInterface against retinue's I2P interface, on two different routers.

Stock RNS 1.5.7 runs in a child process with an I2PInterface on a local i2pd (its SAM
bridge at 127.0.0.1:7656, as i2plib defaults). Retinue runs as murm-i2p's `rns_i2p_peer`
example on an embedded emissary. A loopback TCP control runs the same exchange first.

Each variant: both sides announce until each has heard the other; retinue opens a link to
RNS's destination and echoes; RNS opens a link to retinue's and echoes; retinue sends RNS a
12,000-byte Resource; RNS sends retinue one. Variants: retinue dials RNS (RNS connectable),
and RNS dials retinue (retinue connectable), each with no access code and with a 16-byte
one. After the first dial variant, the keyfile check: retinue's `i2p_keyfile` names the file
RNS wrote and yields the b32 RNS reports. After that, the reconnect check: RNS is killed and
restarted on the same storage without announcing; retinue must redial about every 15 s,
come back online, keep its route, and echo again.

Needs i2pd running with SAM on 127.0.0.1:7656 (see the I2P3 receipt's i2pd.conf).

Run from the oracle/ directory:
    ./.venv/Scripts/python.exe -u interop_i2p.py [--receipt FILE] [--storage EMISSARY_DIR]

Use --example PATH to run a binary already built by the workspace's guarded launcher.
"""
from __future__ import annotations

import hashlib, json, os, re, shutil, socket, subprocess, sys, tempfile, threading, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
NATIVE = REPO / "native" / "Cargo.toml"
RNS_SEED = bytes.fromhex("4e" * 64)
LENGTH = 12_000
IFAC = ("i2p4-net", "i2p4-pass")
REDIAL = 15.0


def payload(n: int) -> bytes:
    return bytes((i * 31 + 7) % 256 for i in range(n))


class Proc:
    """A child speaking a line protocol: commands on stdin, events on stdout."""

    def __init__(self, label, argv, cwd=None):
        self.label, self.lines, self.cursor, self.t0 = label, [], 0, time.monotonic()
        self.p = subprocess.Popen(argv, cwd=cwd, text=True, encoding="utf-8", errors="replace",
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=1)
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        for line in self.p.stdout:
            line = line.rstrip()
            self.lines.append((time.monotonic(), line))
            print(f"  [{self.label}] {line}", flush=True)

    def send(self, line):
        try:
            self.p.stdin.write(line + "\n"); self.p.stdin.flush()
        except OSError:
            pass

    def expect(self, pattern, timeout):
        """The next line from the cursor matching `pattern`, as (time, match), or None."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            while self.cursor < len(self.lines):
                at, line = self.lines[self.cursor]; self.cursor += 1
                if m := re.match(pattern, line):
                    return at, m
            if self.p.poll() is not None and self.cursor >= len(self.lines):
                return None
            time.sleep(0.05)
        return None

    def seen(self, pattern, since=0.0):
        return [(at, m) for at, line in self.lines if at >= since and (m := re.match(pattern, line))]

    def kill(self):
        try:
            self.p.kill(); self.p.wait(10)
        except Exception:
            pass


def sam_up() -> bool:
    try:
        with socket.create_connection(("127.0.0.1", 7656), timeout=3):
            return True
    except OSError:
        return False


def rns_config(cfg: Path, block: str, static_identity: bool = False):
    """A non-transport RNS config. `static_identity` sets `static_transport_identity`: without
    it RNS swaps in a fresh ephemeral transport identity at every start (Transport.py
    332-335), and I2PInterface names its keyfile from that identity (I2PInterface.py 234),
    so a connectable node would get a new b32 on every restart."""
    static = "  static_transport_identity=Yes\n" if static_identity else ""
    (cfg / "config").write_text(
        "[reticulum]\n  enable_transport=No\n  share_instance=No\n  panic_on_interface_error=No\n"
        + static + f"\n[logging]\n  loglevel={RNS_LOGLEVEL[0]}\n\n[interfaces]\n" + block,
        encoding="utf-8")


def i2pd_resolves(b32: str, limit: float) -> float | None:
    """Seconds until i2pd answers a fresh-connection NAMING LOOKUP for `b32` with OK, or None.
    Stock RNS's first lookup of a peer must succeed: one failure closes its I2P event loop for
    good (I2PInterface.py 106-112)."""
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit:
        try:
            with socket.create_connection(("127.0.0.1", 7656), timeout=30) as s:
                f = s.makefile("rwb")
                f.write(b"HELLO VERSION MIN=3.1 MAX=3.3\n"); f.flush(); f.readline()
                f.write(f"NAMING LOOKUP NAME={b32}\n".encode()); f.flush()
                if b"RESULT=OK" in f.readline():
                    return round(time.monotonic() - t0, 1)
        except OSError:
            pass
        time.sleep(10)
    return None


def ifac_lines(ifac, explicit_size: bool = False) -> str:
    """The section's access code. `explicit_size` adds `ifac_size = 128` (bits, the stream
    default): RNS I2P peers created from `peers =` copy the parent's ifac_size before
    Reticulum.py fills in its default (I2PInterface.py 366), so without it they carry None
    and fail on every frame."""
    if not ifac:
        return ""
    size = "    ifac_size = 128\n" if explicit_size else ""
    return f"    networkname = {ifac[0]}\n    passphrase = {ifac[1]}\n" + size


EXAMPLE: list[str] = []
RETINUE_SAM: list[str] = []  # ["--sam", "HOST:PORT"] to put retinue on an external router
RETINUE_EXTRA: list[str] = []  # further example flags, such as --keep-session
RECOVERY_WINDOW = [900.0]
RNS_LOGLEVEL = [3]  # --rns-loglevel N for every RNS child; the restart check raises it to 7


def build_example() -> bool:
    """Build `rns_i2p_peer` once and record its path. It is run directly, not through
    `cargo run`: on Windows, killing cargo leaves its child, and the embedded router with
    it, running."""
    if "--example" in sys.argv:
        path = Path(sys.argv[sys.argv.index("--example") + 1]).resolve()
        if not path.is_file():
            print(f"retinue example does not exist: {path}", flush=True)
            return False
        EXAMPLE[:] = [str(path)]
        print(f"retinue example (prebuilt): {path}", flush=True)
        return True
    build = subprocess.run(["cargo", "build", "--manifest-path", str(NATIVE), "-p", "murm-i2p",
                            "--example", "rns_i2p_peer", "--message-format=json"],
                           cwd=REPO, capture_output=True, text=True)
    for line in build.stdout.splitlines():
        if '"executable"' in line:
            msg = json.loads(line)
            if msg.get("target", {}).get("name") == "rns_i2p_peer" and msg.get("executable"):
                EXAMPLE[:] = [msg["executable"]]
    if build.returncode or not EXAMPLE:
        print(build.stderr[-2000:], flush=True)
        return False
    print(f"retinue example: {EXAMPLE[0]}", flush=True)
    return True


def retinue(args) -> Proc:
    return Proc("retinue", EXAMPLE + args, cwd=REPO)


def rns(cfg: Path) -> Proc:
    return Proc("rns", [sys.executable, "-u", str(Path(__file__)), "--rns", str(cfg)], cwd=HERE)


def exchange(r: Proc, n: Proc, limit: float) -> dict:
    """Announce until both have heard each other, then echo and send a Resource each way."""
    out, t0 = {}, time.monotonic()
    ready = r.expect(r"READY ([0-9a-f]{32}) ([0-9a-f]{32})", 300)
    dest = n.expect(r"DEST ([0-9a-f]{32})", 120)
    if not ready or not dest:
        return {"ok": False, "failed": "startup"}
    r_stream, r_res = ready[1].group(1), ready[1].group(2)
    n_dest = dest[1].group(1)
    deadline = t0 + limit
    while time.monotonic() < deadline:
        r.send("ANNOUNCE"); n.send("ANNOUNCE")
        time.sleep(5)
        if r.seen(rf"HEARD {n_dest} ") and n.seen(r"HEARD i2p4\.retinue ") and n.seen(r"HEARD i2p4\.retinue-res "):
            break
    else:
        return {"ok": False, "failed": "announces"}
    out["announces_s"] = round(time.monotonic() - t0, 1)

    def step(name, side, command, waits):
        side.send(command)
        for proc, pattern in waits:
            got = proc.expect(pattern, max(5.0, deadline - time.monotonic()))
            if got is None:
                out["failed"] = name
                return False
        out[f"{name}_s"] = round(time.monotonic() - t0, 1)
        return True

    digest = hashlib.sha256(payload(LENGTH)).hexdigest()
    ok = (step("retinue_echo", r, f"ECHO {n_dest} hello-from-retinue",
               [(r, r"ECHO_OK echo:hello-from-retinue"), (n, r"RX hello-from-retinue")])
          and step("rns_echo", n, "ECHO hello-from-rns",
                   [(n, r"ECHO echo:hello-from-rns"), (r, r"RX hello-from-rns")])
          and step("retinue_resource", r, f"PUBLISH {n_dest} {LENGTH}",
                   [(r, rf"PUBLISH_OK {LENGTH} Resource {digest}"), (n, rf"RES_IN {LENGTH} {digest} COMPLETE")])
          and step("rns_resource", n, f"SEND_RESOURCE {LENGTH}",
                   [(r, rf"RES_IN {LENGTH} {digest}"), (n, r"RES_OUT COMPLETE")]))
    out["ok"] = ok
    out["rns_dest"], out["retinue_dests"] = n_dest, [r_stream, r_res]
    return out


def variant(name, carrier, ifac, storage, extra=None) -> dict:
    """One exchange. `carrier` is tcp, dial (retinue dials RNS) or listen (RNS dials retinue)."""
    print(f"\n=== {name} ===", flush=True)
    cfg = Path(tempfile.mkdtemp(prefix=f"retinue-i2p4-{name}-"))
    procs, result = [], {"name": name, "ifac": bool(ifac)}
    try:
        flag = ["--ifac", f"{ifac[0]}:{ifac[1]}"] if ifac else []
        if carrier == "tcp":
            with socket.socket() as s:
                s.bind(("127.0.0.1", 0)); port = s.getsockname()[1]
            rns_config(cfg, "  [[TCP]]\n    type = TCPServerInterface\n    enabled = yes\n"
                       f"    listen_ip = 127.0.0.1\n    listen_port = {port}\n" + ifac_lines(ifac))
            n = rns(cfg); procs.append(n)
            n.expect(r"DEST ", 60)
            n.cursor = 0
            r = retinue(["run", "--carrier", f"tcp:127.0.0.1:{port}"] + flag); procs.append(r)
        elif carrier == "dial":
            rns_config(cfg, "  [[I2P]]\n    type = I2PInterface\n    enabled = yes\n"
                       "    connectable = yes\n" + ifac_lines(ifac), static_identity=True)
            result["rns_static_transport_identity"] = True
            n = rns(cfg); procs.append(n)
            b32 = n.expect(r"B32 (\S+)", 600)
            if not b32:
                return dict(result, ok=False, failed="rns b32")
            n.cursor = 0
            result["rns_b32"] = b32[1].group(1)
            r = retinue(["run", "--storage", storage, "--carrier", f"i2p-dial:{result['rns_b32']}"]
                        + RETINUE_SAM + RETINUE_EXTRA + flag)
            procs.append(r)
        else:
            r = retinue(["run", "--storage", storage, "--carrier", "i2p-listen"] + RETINUE_SAM + RETINUE_EXTRA + flag)
            procs.append(r)
            b32 = r.expect(r"B32 (\S+)", 300)
            if not b32:
                return dict(result, ok=False, failed="retinue b32")
            r.cursor = 0
            result["retinue_b32"] = b32[1].group(1)
            waited = i2pd_resolves(result["retinue_b32"], 900)
            if waited is None:
                return dict(result, ok=False, failed="i2pd never resolved retinue's b32")
            result["i2pd_resolved_retinue_after_s"] = waited
            print(f"  i2pd resolves retinue's b32 after {waited}s", flush=True)
            rns_config(cfg, "  [[I2P]]\n    type = I2PInterface\n    enabled = yes\n"
                       f"    peers = {result['retinue_b32']}\n" + ifac_lines(ifac, explicit_size=True))
            n = rns(cfg); procs.append(n)
        result.update(exchange(r, n, 900 if carrier != "tcp" else 60))
        if extra and result["ok"]:
            result.update(extra(cfg, r, n, procs, result))
        r.send("EXIT"); n.send("EXIT")
        return result
    finally:
        for p in procs:
            p.kill()
        shutil.rmtree(cfg, ignore_errors=True)


def keyfile_and_reconnect(cfg: Path, r: Proc, n: Proc, procs: list, result: dict) -> dict:
    out = {}
    # F4: retinue names the keyfile RNS wrote and reads the b32 RNS reports.
    storage = cfg / "storage"
    k = subprocess.run(EXAMPLE + ["keyfile", str(storage), "I2P"], cwd=REPO, capture_output=True, text=True)
    m = re.search(r"KEYFILE (.+) (\S+\.b32\.i2p)", k.stdout)
    files = sorted(p.name for p in (storage / "i2p").glob("*.i2p"))
    out["keyfile_rns_files"] = files
    out["keyfile_ok"] = bool(m) and Path(m.group(1)).name in files and m.group(2) == result["rns_b32"]
    print(f"  keyfile: {k.stdout.strip()} vs RNS files {files}, RNS b32 {result['rns_b32']}", flush=True)

    # Reconnect: kill RNS, restart on the same storage without announcing.
    r.send(f"ROUTE {result['rns_dest']}")
    before = r.expect(rf"ROUTE {result['rns_dest']} (true|false)", 30)
    out["route_before"] = bool(before) and before[1].group(1) == "true"
    killed = time.monotonic()
    n.kill(); procs.remove(n)
    time.sleep(3)
    # The restarted node logs at level 7, so its log shows the accepted connection, the
    # spawned interface and any link request.
    config = cfg / "config"
    config.write_text(config.read_text(encoding="utf-8").replace(f"loglevel={RNS_LOGLEVEL[0]}", "loglevel=7"),
                      encoding="utf-8")
    n2 = rns(cfg); procs.append(n2)
    back = n2.expect(r"B32 (\S+)", 600)
    out["same_b32_after_restart"] = bool(back) and back[1].group(1) == result["rns_b32"]
    offline = r.expect(r"ONLINE false", 600)
    online = r.expect(r"ONLINE true", RECOVERY_WINDOW[0])
    out["back_online"] = bool(online)
    if online:
        out["recovery_s"] = round(online[0] - killed, 1)
    # Redial cadence (the owner's condition): the first redial after the stream ends, and
    # each redial after a refused one, comes about 15 s later.
    replies = [(at, m.group(1)) for at, m in r.seen(r"SAM_REPLY STREAM STATUS (\S+)", killed)]
    # A redial starts with its first SAM command: STREAM CONNECT on a kept session, or
    # SESSION CREATE when the peer recreates its session (`fresh_session_per_redial`).
    starts = [at for at, _ in r.seen(r"SAM_CMD (STREAM CONNECT|SESSION CREATE)", killed)]
    gaps = []
    if offline:
        nxt = [s for s in starts if s > offline[0]]
        if nxt:
            gaps.append(round(nxt[0] - offline[0], 1))
    for (at, res) in replies:
        nxt = [s for s in starts if s > at]
        if res != "RESULT=OK" and nxt:
            gaps.append(round(nxt[0] - at, 1))
    out["redial_gaps_s"] = gaps
    out["redials_about_15s"] = bool(gaps) and all(REDIAL - 1.0 <= g <= REDIAL + 3.0 for g in gaps)
    r.send(f"ROUTE {result['rns_dest']}")
    after = r.expect(rf"ROUTE {result['rns_dest']} (true|false)", 30)
    out["route_kept"] = bool(after) and after[1].group(1) == "true"
    # Beyond the owner's condition, reported on its own: a fresh link to the restarted node,
    # with interface counters around it, and one more attempt 30 s after a failure.
    r.send("STATS")
    r.send(f"ECHO {result['rns_dest']} after-restart")
    echo = r.expect(r"ECHO_OK echo:after-restart|ECHO_ERR .*", 300)
    out["link_after_restart"] = bool(echo) and echo[1].group(0).startswith("ECHO_OK")
    r.send("STATS")
    if not out["link_after_restart"]:
        time.sleep(30)
        r.send("STATS")
        r.send(f"ECHO {result['rns_dest']} after-restart-retry")
        retry = r.expect(r"ECHO_OK echo:after-restart-retry|ECHO_ERR .*", 300)
        out["link_after_restart_retry"] = bool(retry) and retry[1].group(0).startswith("ECHO_OK")
        r.send("STATS")
        time.sleep(1)
    out["stats"] = [line for _, line in r.lines if line.startswith("STATS ")][-6:]
    out["reconnect_ok"] = all(out[k] for k in ("route_before", "same_b32_after_restart", "back_online",
                                               "redials_about_15s", "route_kept"))
    n2.send("EXIT")
    return out


def run_rns(cfg: str) -> int:
    """The stock RNS side: one destination that echoes link data and accepts Resources."""
    import RNS
    RNS.Reticulum(configdir=cfg)
    print(f"TRANSPORT {RNS.Transport.identity.hash.hex()}", flush=True)
    dest = RNS.Destination(RNS.Identity.from_bytes(RNS_SEED), RNS.Destination.IN,
                           RNS.Destination.SINGLE, "i2p4", "rns")

    def on_resource(resource):
        data = resource.data.read() if hasattr(resource.data, "read") else bytes(resource.data or b"")
        status = "COMPLETE" if resource.status == RNS.Resource.COMPLETE else str(resource.status)
        print(f"RES_IN {len(data)} {hashlib.sha256(data).hexdigest()} {status}", flush=True)

    def on_link(link):
        def on_packet(message, packet):
            print(f"RX {bytes(message).decode(errors='replace')}", flush=True)
            RNS.Packet(link, b"echo:" + bytes(message)).send()
        link.set_packet_callback(on_packet)
        link.set_resource_strategy(RNS.Link.ACCEPT_ALL)
        link.set_resource_concluded_callback(on_resource)

    dest.set_link_established_callback(on_link)
    print(f"DEST {dest.hash.hex()}", flush=True)
    heard = {}

    class Heard:
        def __init__(self, aspect):
            self.aspect_filter = aspect

        def received_announce(self, destination_hash, announced_identity, app_data):
            heard[self.aspect_filter] = announced_identity
            print(f"HEARD {self.aspect_filter} {destination_hash.hex()}", flush=True)

    for aspect in ("i2p4.retinue", "i2p4.retinue-res"):
        RNS.Transport.register_announce_handler(Heard(aspect))

    def report_b32():
        while True:
            for iface in RNS.Transport.interfaces:
                if type(iface).__name__ == "I2PInterface" and getattr(iface, "b32", None):
                    print(f"B32 {iface.b32}.b32.i2p", flush=True)
                    return
            time.sleep(0.5)
    threading.Thread(target=report_b32, daemon=True).start()

    def link_to(aspect, on_up):
        identity = heard.get("i2p4." + aspect)
        if identity is None:
            print(f"NOT_HEARD {aspect}", flush=True); return
        link = RNS.Link(RNS.Destination(identity, RNS.Destination.OUT, RNS.Destination.SINGLE,
                                        "i2p4", aspect))
        link.set_link_established_callback(on_up)

    for line in sys.stdin:
        words = line.split()
        if words == ["ANNOUNCE"]:
            dest.announce()
        elif words[:1] == ["ECHO"] and len(words) == 2:
            msg = words[1].encode()
            def up(link, msg=msg):
                link.set_packet_callback(lambda m, p: print(f"ECHO {bytes(m).decode(errors='replace')}", flush=True))
                RNS.Packet(link, msg).send()
            link_to("retinue", up)
        elif words[:1] == ["SEND_RESOURCE"] and len(words) == 2:
            data = payload(int(words[1]))
            def done(resource):
                status = "COMPLETE" if resource.status == RNS.Resource.COMPLETE else str(resource.status)
                print(f"RES_OUT {status}", flush=True)
            link_to("retinue-res", lambda link: RNS.Resource(data, link, callback=done))
        elif words == ["EXIT"]:
            break
    RNS.exit(0)
    return 0


def main() -> int:
    receipt = sys.argv[sys.argv.index("--receipt") + 1] if "--receipt" in sys.argv else None
    storage = (sys.argv[sys.argv.index("--storage") + 1] if "--storage" in sys.argv
               else str(REPO.parent.parent / "testing" / "retinue" / "emissary"))
    if "--retinue-sam" in sys.argv:
        RETINUE_SAM[:] = ["--sam", sys.argv[sys.argv.index("--retinue-sam") + 1]]
    if "--rns-loglevel" in sys.argv:
        RNS_LOGLEVEL[0] = int(sys.argv[sys.argv.index("--rns-loglevel") + 1])
    if "--keep-session" in sys.argv:
        RETINUE_EXTRA.append("--keep-session")
    if "--recovery-window" in sys.argv:
        RECOVERY_WINDOW[0] = float(sys.argv[sys.argv.index("--recovery-window") + 1])
    import RNS
    print(f"RNS {RNS.__version__}", flush=True)
    if not sam_up():
        print("i2pd's SAM bridge is not listening on 127.0.0.1:7656", flush=True)
        return 2
    if not build_example():
        return 2
    started = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    plan = [("tcp-control", "tcp", None, None),
            ("retinue-dials-rns", "dial", None, keyfile_and_reconnect),
            ("retinue-dials-rns-ifac16", "dial", IFAC, None),
            ("rns-dials-retinue", "listen", None, None),
            ("rns-dials-retinue-ifac16", "listen", IFAC, None)]
    only = set(sys.argv[sys.argv.index("--only") + 1].split(",")) if "--only" in sys.argv else None
    results = []
    for name, carrier, ifac, extra in plan:
        if name == "tcp-control" or only is None or name in only:
            results.append(variant(name, carrier, ifac, storage, extra=extra))
            if name == "tcp-control" and not results[0]["ok"]:
                break
    print("\n" + "=" * 72)
    ok = True
    for res in results:
        checks = {"exchange": res.get("ok", False)}
        if "keyfile_ok" in res:
            checks["keyfile"] = res["keyfile_ok"]
        if "reconnect_ok" in res:
            checks["reconnect"] = res["reconnect_ok"]
            checks["relink"] = res["link_after_restart"]
        for check, passed in checks.items():
            ok &= passed
            print(f"{res['name']:<28} {check:<10} {'PASS' if passed else 'FAIL'}"
                  f"{'' if passed else '  (' + str(res.get('failed', '')) + ')'}")
    if len(results) < (5 if only is None else 1 + len(only)):
        ok = False
    print("=" * 72)
    print(f"I2P4 STOCK RNS INTEROP: {'PASS' if ok else 'FAIL'}", flush=True)
    if receipt:
        Path(receipt).write_text(json.dumps({"started": started, "rns": RNS.__version__,
                                             "retinue_router": RETINUE_SAM[1] if RETINUE_SAM else "emissary",
                                             "retinue_extra": RETINUE_EXTRA,
                                             "recovery_window_s": RECOVERY_WINDOW[0],
                                             "only": sorted(only) if only else None,
                                             "results": results}, indent=2) + "\n", encoding="utf-8")
    return 0 if ok else 1


if __name__ == "__main__":
    # Windows pipes and redirects default to cp1252; verbose RNS logs carry characters it
    # cannot encode, and one failed print would kill the thread relaying a child's lines.
    sys.stdout.reconfigure(encoding="utf-8", errors="backslashreplace")
    if sys.argv[1:2] == ["--rns"]:
        raise SystemExit(run_rns(sys.argv[2]))
    raise SystemExit(main())
