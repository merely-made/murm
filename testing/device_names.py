"""Attended USB commissioning of a public firmware label, with reboot readback.

Requires pyserial, named firmware already installed, and the selected known USB
carrier. Does not flash firmware, change radio settings, or reset identities.
"""

import argparse
import hashlib
import json
import time
from pathlib import Path


def main():
    import serial
    from serial.tools.list_ports import comports

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("device", choices=["SB1", "SB2", "T114"])
    parser.add_argument("--name", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--provision", action="store_true")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("preserve earlier receipts")
    if not (1 <= len(args.name) <= 16 and args.name.isascii()
            and all(32 <= ord(c) <= 126 for c in args.name)
            and args.name == args.name.strip()):
        parser.error("name must be 1-16 printable ASCII characters without edge spaces")
    expected = {
        "SB1": (0x303A, 0x1001, "44:1B:F6:6A:FB:28"),
        "SB2": (0x303A, 0x1001, "44:1B:F6:6A:FA:64"),
        "T114": (0x1915, 0x521F, "TULLE-T114-01"),
    }[args.device]
    report = dict(device=args.device, name=args.name, expected_usb=expected,
                  source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  provision=args.provision, sessions=[], passed=False)

    def connect():
        found = [p for p in comports() if (p.vid, p.pid, p.serial_number) == expected]
        if len(found) != 1:
            raise RuntimeError("selected USB carrier is absent or ambiguous")
        link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
        link.port, link.rts, link.dtr = found[0].device, False, args.device == "T114"
        link.open()
        session = dict(port=found[0].device, attached="", responses=[])
        report["sessions"].append(session)
        time.sleep(.5)
        session["attached"] = link.read(4096).decode(errors="replace")
        return link, session

    def query(link, session, command):
        item = dict(command=command, response="")
        session["responses"].append(item)
        link.write(command.encode() + b"\n")
        link.flush()
        response = bytearray()
        deadline = time.monotonic() + 1.6
        try:
            while time.monotonic() < deadline:
                response.extend(link.read(4096))
                if command.startswith("name set ") and response.startswith(b"name saved; rebooting\r\n"):
                    break
        finally:
            item["response"] = response.decode(errors="replace")
        return item["response"]

    try:
        link, session = connect()
        with link:
            before = {cmd: query(link, session, cmd) for cmd in ["status", "name", "region", "channel", "ui"]}
            assert "phy online" in before["status"], "firmware is not online"
            assert "name=" in before["name"], "firmware lacks the name command"
            invalid = query(link, session, "name set bad\tname")
            assert invalid.startswith("name invalid:"), "invalid name was not refused"
            assert query(link, session, "name") == before["name"], "invalid name changed settings"
            if args.provision:
                answer = query(link, session, "name set " + args.name)
                assert answer == "name saved; rebooting\r\n" or answer == "name=" + args.name + "\r\n", answer
        time.sleep(6)
        link, session = connect()
        with link:
            after = {cmd: query(link, session, cmd) for cmd in ["status", "name", "region", "channel", "ui"]}
            assert after["name"] == "name=" + args.name + "\r\n"
            assert "name=" + args.name + "\r\n" in after["status"]
            assert after["region"] == before["region"], "region changed"
            assert after["channel"] == before["channel"], "channel changed"
            # An unchanged name must not write or reboot, including on the second session.
            assert query(link, session, "name set " + args.name) == after["name"]
            assert query(link, session, "status") == after["status"], "unchanged name caused a reset/write"
        report["passed"] = True
    except Exception as error:
        report["error"] = repr(error)
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
