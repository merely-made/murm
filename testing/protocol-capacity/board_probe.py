"""Attended memory probe; exact carrier identity, bounded rounds, no settings writes."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import time

def canonical_status(text):
    """Strip only the observed truncated ROM segment log before a status reply."""
    prefix = re.match(r"I \(\d+\) esp_image: segment \d+: paddr=[0-9a-fA-F]+ vaddr=[0-9a-fA-F]+ size(?=tulle/)", text)
    return text[prefix.end():] if prefix else text


def main():
    import serial
    from serial.tools.list_ports import comports
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", choices=["v4", "t114"])
    parser.add_argument("--port", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, choices=range(17), default=0)
    parser.add_argument("--usb-serial", choices=["44:1B:F6:6A:FA:64", "44:1B:F6:6A:FB:28"])
    parser.add_argument("--min-stack-bytes", type=int, default=0)
    parser.add_argument("--min-free-heap-percent", type=int, choices=range(101), default=0)
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
              "stack_scope": "boot paint when stack_free is present; otherwise point-in-time samples only",
              "acceptance": {"min_stack_bytes": args.min_stack_bytes,
                             "min_free_heap_percent": args.min_free_heap_percent}}
    try:
        link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
        link.port, link.rts, link.dtr = args.port, False, False
        with link:
            if args.board == "t114":
                # Let firmware observe a real detach before a new CDC session.
                time.sleep(.3)
                link.dtr = True
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
                    if args.min_stack_bytes:
                        assert int(values["stack_free"]) >= args.min_stack_bytes, "stack reserve below profile limit"
                    assert (int(values["capacity"])-int(values["peak"]))*100 >= int(values["capacity"])*args.min_free_heap_percent, "heap reserve below profile limit"
                    print(json.dumps({"round": count, "measurement": values}), flush=True)
            statuses = [r["response"] for r in report["responses"] if r["command"] == "status"]
            assert all("phy online" in s for s in statuses), "firmware status absent"
            assert canonical_status(statuses[0]) == canonical_status(statuses[1]), "status changed during campaign"
            report["passed"] = True
    except Exception as error:
        report["error"] = repr(error)
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
