"""Stock TCP deposit and mixed-backend propagation fetch on the Windows bench."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import time

import LXMF
import RNS


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", nargs="+", choices=["outrider", "ratspeak"],
                        default=["outrider", "ratspeak"])
    parser.add_argument("--carrier", nargs="+", choices=["tcp", "raw", "paired"], default=["tcp"])
    parser.add_argument("--bin-dir", type=Path, required=True, help="Cargo debug or release directory")
    parser.add_argument("--lxmd", type=Path, required=True, help="Ratspeak lxmd-rs.exe")
    parser.add_argument("--output", type=Path, required=True, help="new receipt directory")
    parser.add_argument("--relay-radio", help="serial port on the propagation node side")
    parser.add_argument("--recipient-radio", help="serial port on the recipient side")
    parser.add_argument("--bandwidth", type=int, default=250000)
    parser.add_argument("--power", type=int, default=2)
    args = parser.parse_args()
    if set(args.carrier) - {"tcp"} and (not all((args.relay_radio, args.recipient_radio))
            or args.relay_radio.casefold() == args.recipient_radio.casefold()):
        parser.error("radio cases require two distinct serial ports")
    return args


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def interface(name, port_number, *, listen=False, passphrase=None):
    kind = "Server" if listen else "Client"
    host = "listen" if listen else "target"
    address_key = "listen_ip" if listen else "target_host"
    config = (f"[[{name}]]\ntype=TCP{kind}Interface\nenabled=yes\n"
              f"{address_key}=127.0.0.1\n{host}_port={port_number}\n")
    if passphrase:
        config += f"network_name=signalman-qualification\npassphrase={passphrase}\nifac_size=64\n"
    return config


def stop(process):
    if process.poll() is None:
        subprocess.run(["taskkill", "/pid", str(process.pid), "/t", "/f"], capture_output=True)
    process.wait()


class Bench:
    def __init__(self, args, root, stack):
        self.args = args
        self.root = root
        self.stack = stack
        self.children = {}
        self.examples = args.bin_dir / "examples"

    def start(self, name, command, env=None):
        log = self.stack.enter_context((self.root / f"{name}.log").open("wb"))
        # An operator's example overrides must not add another carrier or restore a store.
        clean = {k: v for k, v in os.environ.items() if not k.startswith(("OUTRIDER_", "RETINUE_ADDR"))}
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
                                   env=clean | (env or {}), creationflags=subprocess.CREATE_NO_WINDOW)
        self.stack.callback(stop, process)
        self.children[name] = process
        return process

    def log(self, name):
        return (self.root / f"{name}.log").read_text(errors="replace")

    def wait(self, label, predicate, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            for name, process in self.children.items():
                if process.poll() is not None and (name != "recipient" or process.returncode):
                    raise RuntimeError(f"{name} exited {process.returncode} during {label}")
            time.sleep(0.25)
        raise TimeoutError(label)

    def config(self, name, devices):
        directory = self.root / (name + "-rns")
        directory.mkdir()
        transport = "yes" if name == "relay" else "no"
        (directory / "config").write_text(f"[reticulum]\nshare_instance=no\nenable_transport={transport}\n"
                                          "[interfaces]\n" + devices)
        return directory

    def ratspeak(self, name, devices, identity, node_hash):
        directory = self.root / name
        directory.mkdir()
        identity.to_file(str(directory / "identity"))
        relay = name == "relay"
        setting = "enable_node=yes" if relay else f"outbound_node={node_hash.hex()}"
        (directory / "config").write_text("[lxmf]\nannounce_at_start=yes\nannounce_interval=1\n"
            "[propagation]\nautopeer=no\nannounce_at_start=yes\nannounce_interval=1\n"
            f"propagation_stamp_cost_target=8\n{setting}\n")
        return self.start(name, [self.args.bin_dir / "signalman.exe", "ratspeak", self.args.lxmd,
                                 directory, self.config(name, devices), "relay" if relay else "receive"])

    def radio(self, name, serial, dtr, carrier):
        self.start(name, [self.examples / "radio_bridge.exe", serial, "0", str(self.args.bandwidth),
                          str(self.args.power), dtr, carrier])
        match = self.wait(name, lambda: re.search(r"online at 127\.0\.0\.1:(\d+)", self.log(name)), 20)
        return int(match[1])


def exercise(bench, node, carrier, receipt):
    ours = node == "outrider"
    radio = carrier != "tcp"
    relay_id = RNS.Identity.from_bytes(bytes([0x70]) * 64) if ours else RNS.Identity()
    recipient_id = RNS.Identity() if ours else RNS.Identity.from_bytes(bytes([0x62]) * 64)
    node_hash = RNS.Destination.hash_from_name_and_identity("lxmf.propagation", relay_id)
    hub = port()
    local = interface("local", hub, listen=True)
    env = {"OUTRIDER_LARGE": "1", "OUTRIDER_PROPAGATION_COST": "8"}
    if radio:
        key = secrets.token_hex(16)
        a = bench.radio("relay-radio", bench.args.relay_radio, "true", carrier)
        b = bench.radio("recipient-radio", bench.args.recipient_radio, "false", carrier)
        env.update(OUTRIDER_BRIDGE_ADDR=f"127.0.0.1:{a if ours else b}",
                   OUTRIDER_BRIDGE_PASSPHRASE=key, OUTRIDER_LINK_MTU="247" if carrier == "raw" else "500")
        local += interface("air", a, passphrase=key)
    if ours:
        relay = bench.start("relay", [bench.examples / "stock_propagation_server.exe"], env)
        hub = int(bench.wait("TCP listener", lambda: re.search(r"LISTENING (\d+)", bench.log("relay")), 20)[1])
    else:
        relay = bench.ratspeak("relay", local, relay_id, node_hash)
    receipt["relay_pid"] = relay.pid
    RNS.Reticulum(configdir=str(bench.config("sender", interface("deposit", hub))))
    sender_id = RNS.Identity()
    router = LXMF.LXMRouter(identity=sender_id, storagepath=str(bench.root / "sender"))
    source = router.register_delivery_identity(sender_id, display_name="trial", stamp_cost=None)
    if not ours:
        recipient = bench.start("recipient", [bench.examples / "stock_propagation_fetch_receive.exe"],
                                env | ({} if radio else {"RETINUE_ADDR": f"127.0.0.1:{hub}"}))
    bench.wait("node announce", lambda: RNS.Identity.recall(node_hash), 100)
    router.set_outbound_propagation_node(node_hash)
    if ours:
        router.announce(source.hash)
    dest = RNS.Destination(recipient_id, RNS.Destination.OUT, RNS.Destination.SINGLE, "lxmf", "delivery")
    body = b"mixed fetch " + secrets.token_hex(6).encode()
    message = LXMF.LXMessage(dest, source, body, desired_method=LXMF.LXMessage.PROPAGATED)
    router.handle_outbound(message)
    bench.wait("TCP deposit", lambda: message.state in (message.SENT, message.FAILED), 100)
    if message.state != message.SENT:
        raise RuntimeError("stock sender failed")
    receipt["deposit"] = message.hash.hex()
    if ours:
        bench.wait("stored deposit", lambda: "STORE entries=1" in bench.log("relay"), 15)
        devices = interface("fetch", b, passphrase=key) if radio else interface("fetch", hub)
        bench.ratspeak("recipient", devices, recipient_id, node_hash)

        def inbox():
            for path in (bench.root / "recipient").rglob(message.hash.hex()):
                if path.parent.name == "messages":
                    with path.open("rb") as handle:
                        got = LXMF.LXMessage.unpack_from_file(handle)
                    return got and (got.hash, got.content, got.destination_hash) == (message.hash, body, dest.hash)
            return False

        bench.wait("matching persisted inbox", inbox, 235)
        if radio:
            log = bench.log("relay")
            bridge = re.search(r"PROTECTED_BRIDGE interface=(\d+)", log)[1]
            links = re.findall(r"ACCEPT interface=(\d+) mtu=(\d+)", log)
            if ((bridge, env["OUTRIDER_LINK_MTU"]) not in links
                    or not any(i != bridge and int(m) > 500 for i, m in links)):
                raise RuntimeError(f"unexpected negotiated MTUs {links}")
    else:
        bench.wait("recipient route", lambda: f"FETCH_READY {node_hash.hex()}" in bench.log("recipient"), 85)
        router.announce(source.hash)
        if radio:
            # The sender announce must cross the half-duplex pair before link setup.
            # Wait for a gap in bridge writes, restarting the quiet period on traffic.
            paths = [bench.root / f"{side}-radio.log" for side in ("relay", "recipient")]
            sizes = None
            last_write = time.monotonic()

            def quiet():
                nonlocal sizes, last_write
                current = tuple(p.stat().st_size for p in paths)
                if current != sizes:
                    sizes, last_write = current, time.monotonic()
                return time.monotonic() - last_write > 3

            bench.wait("radio quiet after sender announce", quiet, 35)
        else:
            time.sleep(2)
        recipient.stdin.write(b"fetch\n")
        recipient.stdin.flush()
        # A one-shot receiver closes its bridge as soon as it exits. Read its
        # own outcome first so a clean bridge EOF cannot hide a fetch failure.
        if recipient.wait(timeout=155) != 0:
            raise RuntimeError(bench.log("recipient").splitlines()[-1])
        lines = bench.log("recipient").splitlines()
        for expected in [f"MESSAGE_ID {message.hash.hex()}", f"CONTENT {body.hex()}",
                         "PRODUCTION_FETCH true", "VERIFIED true"]:
            if expected not in lines:
                raise RuntimeError(f"missing {expected}")
        receipt["matched"] = message.hash.hex()
        receipt["acknowledged"] = "ACKNOWLEDGED true" in lines
        if not receipt["acknowledged"]:
            raise RuntimeError("fetched and verified, acknowledgement unconfirmed")
    if relay.poll() is not None:
        raise RuntimeError("relay exited during fetch")
    receipt["matched"] = message.hash.hex()


def trial(args, root, node, carrier):
    result = {"node": node, "recipient": "ratspeak" if node == "outrider" else "outrider",
              "carrier": carrier, "status": "failed", "stock_rns": RNS.__version__, "stock_lxmf": LXMF.__version__}
    with args.lxmd.open("rb") as binary:
        result["lxmd_sha256"] = hashlib.file_digest(binary, "sha256").hexdigest()
    if carrier != "tcp":
        result["radio"] = {"relay_port": args.relay_radio, "recipient_port": args.recipient_radio,
                           "bandwidth_hz": args.bandwidth, "power_dbm": args.power, "ifac_bytes": 8}
    with contextlib.ExitStack() as stack:
        bench = Bench(args, root, stack)
        try:
            exercise(bench, node, carrier, result)
            result["status"] = "delivered"
        except Exception as error:
            result["error"] = str(error)
            if (node, carrier) == ("ratspeak", "raw") and "relay-radio" in bench.children:
                if (bench.children["relay-radio"].poll() not in (None, 0)
                        and re.search(r'rejected 258-byte frame: TooLong \{ max: 255 \}', bench.log("relay-radio"))
                        and str(error).startswith("relay-radio exited ")):
                    result["status"] = "known-raw-refusal"
        evidence = {}
        for name in bench.children:
            lines = bench.log(name).splitlines()
            selected = [line[:300] for line in lines if re.search(
                r"rejected \d+-byte frame|reassembled|fragment of|^ACCEPT |^PROTECTED_BRIDGE|^LINK_CLOSED|^STORE |^ACKNOWLEDGED|^VERIFIED", line)]
            sizes = [int(size) for line in lines for size in re.findall(r"outbound protected_bytes=(\d+)", line)]
            evidence[name] = {"events": selected[:80]}
            if sizes:
                evidence[name].update(packets=len(sizes), max_packet_bytes=max(sizes))
            if result["status"] == "failed":
                evidence[name]["tail"] = lines[-6:]
        result["evidence"] = evidence
    path = args.output / f"{node}-{carrier}.json"
    with path.open("x", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2)
    print(f"{node} node, {carrier}: {result['status']}", flush=True)
    return int(result["status"] == "failed")


def main():
    args = arguments()
    if "MIXED_FETCH_WORKER" in os.environ:
        root, node, carrier = json.loads(os.environ["MIXED_FETCH_WORKER"])
        # RNS owns daemon threads and Windows DB handles until interpreter exit.
        os._exit(trial(args, Path(root), node, carrier))
    required = [args.bin_dir / "signalman.exe", args.lxmd]
    for binary in ("stock_propagation_server", "stock_propagation_fetch_receive"):
        required.append(args.bin_dir / "examples" / (binary + ".exe"))
    if set(args.carrier) - {"tcp"}:
        required.append(args.bin_dir / "examples/radio_bridge.exe")
    for path in required:
        if not path.is_file():
            sys.exit(f"missing binary: {path}")
    args.output.mkdir(parents=True, exist_ok=False)
    failed = False
    for carrier in dict.fromkeys(args.carrier):
        for node in dict.fromkeys(args.node):
            with tempfile.TemporaryDirectory() as root:
                env = os.environ | {"MIXED_FETCH_WORKER": json.dumps([root, node, carrier])}
                code = subprocess.call([sys.executable, "-u", *sys.argv], env=env)
                failed |= code != 0
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
