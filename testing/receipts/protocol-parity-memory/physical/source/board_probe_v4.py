"""Attended memory probe; exact carrier identity, bounded rounds, no settings writes."""
import argparse
import hashlib
import json
from pathlib import Path
import time

import serial
from serial.tools.list_ports import comports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", choices=["v4", "t114"])
    parser.add_argument("--port", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, choices=range(17), default=0)
    parser.add_argument("--usb-serial", choices=["44:1B:F6:6A:FA:64", "44:1B:F6:6A:FB:28"])
    args = parser.parse_args()
    if args.output.exists():
        parser.error("preserve earlier receipts")
    if args.board == "t114" and args.usb_serial is not None:
        parser.error("V4 serial selection does not apply to T114")
    expected = {"v4": (0x303A, 0x1001, args.usb_serial or "44:1B:F6:6A:FA:64"),
                "t114": (0x1915, 0x521F, "TULLE-T114-01")}[args.board]
    found = next((p for p in comports() if p.device == args.port), None)
    if found is None or (found.vid, found.pid, found.serial_number) != expected:
        parser.error("carrier identity mismatch")
    report = {"board": args.board, "usb": expected, "port": args.port,
              "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "requested_rounds": args.rounds, "responses": [], "passed": False,
              "stack_scope": "point-in-time samples only; no high-water claim"}
    try:
        link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
        link.port, link.rts, link.dtr = args.port, False, args.board == "t114"
        with link:
            time.sleep(.4)
            report["attached"] = link.read(4096).decode(errors="replace")
            commands = ["status", "region", "channel", "sync"]
            if args.board == "t114":
                commands += ["crash"]
            commands += ["protocol memory"] * args.rounds
            commands += ["status", "air"]
            if args.board == "t114":
                commands += ["crash"]
            for command in commands:
                started = time.monotonic()
                item = {"command": command, "response": ""}
                report["responses"].append(item)
                link.write(command.encode() + b"\n")
                link.flush()
                response = bytearray()
                deadline = started + (15 if command == "protocol memory" else 1.5)
                while time.monotonic() < deadline:
                    response.extend(link.read(4096))
                    if command == "protocol memory" and b"memory initial=" in response and response.endswith(b"\n"):
                        break
                text = response.decode(errors="replace")
                item.update(response=text, seconds=round(time.monotonic()-started, 3))
                if command == "protocol memory":
                    line = next(s for s in text.splitlines() if s.startswith("memory initial="))
                    values = dict(part.split("=") for part in line.split()[1:])
                    item["measurement"] = values
                    assert values["initial"] == values["final"], "heap did not return to baseline"
                    count = sum(r["command"] == "protocol memory" for r in report["responses"])
                    assert int(values["expected_refusals"]) == 1
                    assert int(values["failures"]) == count, "unexpected allocation refusal"
                    assert int(values["peak"]) < int(values["capacity"]), "heap exhausted"
                    print(json.dumps({"round": count, "measurement": values}), flush=True)
            statuses = [r["response"] for r in report["responses"] if r["command"] == "status"]
            assert all("phy online" in s for s in statuses), "firmware status absent"
            assert statuses[0] == statuses[1], "status changed during campaign"
            report["passed"] = True
    except Exception as error:
        report["error"] = repr(error)
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
