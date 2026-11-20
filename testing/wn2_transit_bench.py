"""WN2 direct-PHY transit bench: relay control, relay run, and the node's figures around them.

The transit link receipt spec (design_docs/2026-08-14_transit_link_receipt_spec.md) with the
V4 node as the relay. Two `transit_peer` processes, one direct-PHY modem each, on this host;
the relay is the V4 node, switched by its own `channel` probe. Run one phase at a time:

  collect  save the relay's `status`, `channel`, `node`, `heap`, `air` and `face` lines
  relay    `channel node` (on) or `channel modem` (off) on the relay, then wait for its boot
  modems   put both end radios in the modem channel
  control  relay off: the exchange must fail for a full timeout, and the responder must
           record no inbound link
  run      relay on: the same exchange, with the relay's figures before and after

Relay off here is the software form, the relay's modem channel; powering the relay down is
the stronger control. Boards are found by USB serial, never by port name. Logs go under
--logs, outside the repository; nothing is written to a board but channel changes.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path
import re
import subprocess
import sys
import threading
import time

import serial
from serial.tools.list_ports import comports

# USB identity per role: (vid, pid, serial), and whether the port wants DTR asserted.
BOARDS = {
    "relay": ((0x303A, 0x1001, "44:1B:F6:6A:FA:64"), False),
    "requester": ((0x303A, 0x1001, "44:1B:F6:6A:FB:28"), False),
    "responder": ((0x1915, 0x521F, "TULLE-T114-01"), True),
}
RELAY_LINES = ["status", "channel", "node", "heap", "air", "face"]
SOFTWARE_FORM = ("relay off is the relay's modem channel (software form); "
                 "powering the relay down is the stronger control")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def find_port(role: str) -> str:
    want, _ = BOARDS[role]
    for port in comports():
        if (port.vid, port.pid, port.serial_number) == want:
            return port.device
    raise SystemExit(f"{role} board {want[2]} is not attached")


def open_board(role: str) -> serial.Serial:
    _, dtr = BOARDS[role]
    link = serial.Serial(port=None, baudrate=115200, timeout=0.1, write_timeout=2)
    link.port, link.rts, link.dtr = find_port(role), False, dtr
    link.open()
    time.sleep(0.4)
    link.read(4096)
    return link


def ask(link: serial.Serial, command: str, seconds: float = 1.5) -> str:
    link.write(command.encode() + b"\n")
    link.flush()
    reply = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        reply.extend(link.read(4096))
    return reply.decode(errors="replace")


def collect(role: str = "relay") -> dict:
    """The node's lines in the node channel. In the modem channel only its own probes are
    asked: `node`, `heap` and `face` would reach the direct-PHY parser as command bytes."""
    with open_board(role) as link:
        lines = {"channel": ask(link, "channel")}
        wanted = RELAY_LINES if "channel=node" in lines["channel"] else ["status", "air"]
        lines.update({command: ask(link, command) for command in wanted if command != "channel"})
        return lines


def counter(lines: dict, name: str) -> int | None:
    match = re.search(rf"\b{name}=(\d+)", lines.get("node", ""))
    return int(match.group(1)) if match else None


def set_channel(role: str, channel: str, boot_wait: float) -> dict:
    with open_board(role) as link:
        before = ask(link, "channel")
        if f"channel={channel}" in before:
            return {"before": before, "changed": False, "after": before}
        reply = ask(link, f"channel {channel}")
    # The board saves and reboots; an unclaimed V4 waits out its claim window first.
    time.sleep(boot_wait)
    with open_board(role) as link:
        after = ask(link, "channel")
    if f"channel={channel}" not in after:
        raise SystemExit(f"{role} did not come back in channel {channel}: {after!r}")
    return {"before": before, "reply": reply, "changed": True, "after": after}


def run_peer(binary: str, args: list[str], log: Path, out: dict, key: str) -> None:
    with log.open("w", encoding="utf-8") as sink:
        proc = subprocess.run([binary, *args], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              text=True, check=False)
        sink.write(proc.stdout)
    out[key] = {"exit": proc.returncode, "lines": proc.stdout.splitlines()}


def lines_with(lines: list[str], prefix: str) -> list[str]:
    return [line for line in lines if line.startswith(prefix)]


def sha(lines: list[str], prefix: str) -> str | None:
    for line in lines_with(lines, prefix):
        match = re.search(r"sha256=([0-9a-f]{64})", line)
        if match:
            return match.group(1)
    return None


def exchange(binary: str, folder: Path, length: int, form: str, timeout: int, settle: int) -> dict:
    window = timeout + settle + 60
    common = [f"len={length}", f"form={form}"]
    responder = ["role=responder", f"port={find_port('responder')}", "dtr=1",
                 f"window={window}", "announce=20", *common]
    requester = ["role=requester", f"port={find_port('requester')}", "dtr=0",
                 f"timeout={timeout}", f"settle={settle}", *common]
    out: dict = {"responder_args": responder[2:], "requester_args": requester[2:]}
    threads = [threading.Thread(target=run_peer,
                                args=(binary, responder, folder / "responder.log", out, "responder"))]
    threads[0].start()
    time.sleep(3)
    threads.append(threading.Thread(target=run_peer,
                                    args=(binary, requester, folder / "requester.log", out, "requester")))
    threads[1].start()
    for thread in threads:
        thread.join()
    req, res = out["requester"]["lines"], out["responder"]["lines"]
    inbound = next((line for line in res if line.startswith("INBOUND")), "")
    out["summary"] = {
        "requester_result": next((line for line in req if line.startswith("RESULT")), None),
        "responder_inbound": inbound,
        "announces": lines_with(req, "ANNOUNCE responder"),
        "route": next((line for line in req if line.startswith("ROUTE")), None),
        "linked": next((line for line in req if line.startswith("LINKED")), None),
        "requester_to_responder_match": sha(req, "SENT") is not None and sha(req, "SENT") == sha(res, "RECEIVED"),
        "responder_to_requester_match": sha(res, "SENT") is not None and sha(res, "SENT") == sha(req, "RECEIVED"),
        "requester_sent": sha(req, "SENT"), "responder_received": sha(res, "RECEIVED"),
        "responder_sent": sha(res, "SENT"), "requester_received": sha(req, "RECEIVED"),
    }
    accepted = re.search(r"accepted=(\d+)", inbound)
    out["summary"]["responder_accepted"] = int(accepted.group(1)) if accepted else None
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("phase", choices=["collect", "relay", "modems", "control", "run"])
    ap.add_argument("--logs", type=Path, required=True, help="log root, outside the repository")
    ap.add_argument("--binary", help="transit_peer executable (control and run)")
    ap.add_argument("--state", choices=["on", "off"], help="relay phase: on or off")
    ap.add_argument("--len", type=int, default=256)
    ap.add_argument("--form", choices=["packet", "stream"], default="stream")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--settle", type=int, default=20)
    ap.add_argument("--boot-wait", type=float, default=40.0)
    ap.add_argument("--tag", default="")
    args = ap.parse_args()

    repo = Path(__file__).resolve().parents[1]
    if args.logs.resolve().is_relative_to(repo):
        ap.error("logs go outside the repository")
    folder = args.logs / f"{utc_now()}-{args.phase}{'-' + args.tag if args.tag else ''}"
    folder.mkdir(parents=True, exist_ok=False)
    report: dict = {"phase": args.phase, "started_utc": utc_now(), "isolation": "none (unattenuated)",
                    "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()}

    if args.phase == "collect":
        report["relay"] = collect()
    elif args.phase == "relay":
        if not args.state:
            ap.error("relay needs --state on|off")
        report["note"] = SOFTWARE_FORM
        report["relay"] = set_channel("relay", "node" if args.state == "on" else "modem", args.boot_wait)
    elif args.phase == "modems":
        report["requester"] = set_channel("requester", "modem", 15)
        report["responder"] = set_channel("responder", "modem", 15)
    else:
        if not args.binary:
            ap.error(f"{args.phase} needs --binary")
        wanted = "modem" if args.phase == "control" else "node"
        report["note"] = SOFTWARE_FORM
        report["relay_channel"] = set_channel("relay", wanted, args.boot_wait)
        report["relay_before"] = collect()
        report["exchange"] = exchange(args.binary, folder, args.len, args.form, args.timeout, args.settle)
        report["relay_after"] = collect()
        summary = report["exchange"]["summary"]
        if args.phase == "run":
            before, after = report["relay_before"], report["relay_after"]
            summary["fwdannounce"] = (counter(before, "fwdannounce"), counter(after, "fwdannounce"))
            summary["fwdpacket"] = (counter(before, "fwdpacket"), counter(after, "fwdpacket"))
        else:
            failed = (summary["requester_result"] or "").startswith("RESULT FAIL")
            summary["control_held"] = failed and summary["responder_accepted"] == 0
    report["finished_utc"] = utc_now()
    (folder / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report.get("exchange", {}).get("summary", report), indent=2))
    print(f"report: {folder / 'report.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
