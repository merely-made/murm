"""Attended SB2 USB reboot test using its existing save-current-channel path.

No firmware flash or change of personality. Each cycle writes one settings
journal record and software-reboots. Stop at the first failed recovery.
"""

import argparse
import hashlib
import json
import re
import time
from pathlib import Path


def main():
    import serial
    from serial.tools.list_ports import comports

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, choices=range(1, 6), default=3)
    parser.add_argument("--connection", required=True, choices=["direct-pc", "hub"])
    args = parser.parse_args()
    if args.output.exists():
        parser.error("preserve earlier receipts")
    expected = (0x303A, 0x1001, "44:1B:F6:6A:FA:64")
    report = dict(device="SB2", expected_usb=expected,
                  connection_user_reported=args.connection, requested_rounds=args.rounds,
                  source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  sessions=[], completed_rounds=0, passed=False,
                  reboot_method="channel modem: verified settings save then software_reset",
                  serial_lines=dict(dtr=False, rts=False), reopen_delay_seconds=6)

    def connect(phase):
        found = [p for p in comports() if (p.vid, p.pid, p.serial_number) == expected]
        assert len(found) == 1, "identified SB2 carrier absent or ambiguous"
        session = dict(phase=phase, port=found[0].device, responses=[])
        report["sessions"].append(session)
        link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
        link.port, link.dtr, link.rts = found[0].device, False, False
        link.open()
        time.sleep(.5)
        session["attached"] = link.read(4096).decode(errors="replace")
        return link, session

    def query(link, session, command, reboot=False):
        item = dict(command=command, response="")
        session["responses"].append(item)
        response = bytearray()
        started = time.monotonic()
        try:
            link.write(command.encode() + b"\n")
            link.flush()
            while time.monotonic() - started < 1.6:
                response.extend(link.read(4096))
                if reboot and response.startswith(b"channel set; rebooting\r\n"):
                    break
        finally:
            item.update(response=response.decode(errors="replace"),
                        seconds=round(time.monotonic() - started, 3))
        return item["response"]

    def snapshot(link, session):
        values = {cmd: query(link, session, cmd) for cmd in ["status", "name", "region", "channel", "ui"]}
        assert "phy online" in values["status"], "online status absent"
        assert values["name"] == "name=SB2\r\n", "wrong device label"
        assert values["region"] == "region=US915\r\n", "unexpected region"
        assert values["channel"] == "channel=modem\r\n", "unexpected channel"
        journal = re.search(r"identity=loaded slot=([AB]) seq=(\d+)\r\n", values["status"])
        assert journal, "stored identity unavailable"
        values["slot"], values["sequence"] = journal[1], int(journal[2])
        values["firmware"] = values["status"].split("\r\n")[0]
        return values

    try:
        link, session = connect("baseline")
        with link:
            before = snapshot(link, session)
        report["baseline"] = before
        for index in range(1, args.rounds + 1):
            link, session = connect(f"reboot-{index}")
            with link:
                assert query(link, session, "name") == "name=SB2\r\n"
                answer = query(link, session, "channel modem", reboot=True)
                assert answer == "channel set; rebooting\r\n", "reboot/save not acknowledged"
            print(f"round {index}: save/reboot acknowledged; waiting for USB", flush=True)
            time.sleep(6)
            link, session = connect(f"readback-{index}")
            with link:
                after = snapshot(link, session)
            assert after["firmware"] == before["firmware"], "firmware changed"
            assert after["sequence"] == (before["sequence"] + 1) % (1 << 32)
            assert after["slot"] != before["slot"], "journal did not alternate"
            report["completed_rounds"] = index
            before = after
            report["last_verified"] = after
            print(f"round {index}: USB recovered, name/settings intact, sequence={after['sequence']}", flush=True)
        report["passed"] = True
    except Exception as error:
        report["error"] = repr(error)
        report["enumerated_after_failure"] = [dict(port=p.device, usb=[p.vid, p.pid, p.serial_number])
                                              for p in comports() if (p.vid, p.pid, p.serial_number) == expected]
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ["passed", "completed_rounds"]} |
                     ({"error": report["error"]} if "error" in report else {})), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
