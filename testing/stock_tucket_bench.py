"""Attended Tucket/stock-MeshCore fixtures. Private material is never printed.

Each command requires a new public receipt path. Radio commands require an
exact USB serial identity; flashing remains a separate, pinned bench action.
"""
import argparse
import json
from pathlib import Path
import struct
import time

import serial
from serial.tools.list_ports import comports


def identified(port, usb_serial, dtr):
    found = next((p for p in comports() if p.device.casefold() == port.casefold()), None)
    if found is None or (found.vid, found.pid, found.serial_number) != (0x303A, 0x1001, usb_serial):
        raise RuntimeError(f"USB identity mismatch on {port}")
    link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
    link.port, link.rts, link.dtr = port, False, dtr
    link.open()
    return link


class Stock:
    def __init__(self, link):
        self.link = link

    def read(self):
        until = time.monotonic() + 10
        def exact(n):
            data = bytearray()
            while len(data) < n:
                if time.monotonic() >= until:
                    raise TimeoutError("stock response deadline")
                data.extend(self.link.read(n - len(data)))
            return data
        while exact(1) != b">":
            pass
        length = int.from_bytes(exact(2), "little")
        if not 0 < length <= 4096:
            raise RuntimeError("stock response length")
        return bytes(exact(length))

    def command(self, payload, expected=0):
        self.link.write(b"<" + struct.pack("<H", len(payload)) + payload)
        self.link.flush()
        for _ in range(128):
            response = self.read()
            if response[0] == 1:
                raise RuntimeError(f"stock command {payload[0]} refused: {response.hex()}")
            if response[0] == expected:
                return response
        raise RuntimeError("stock response count limit")

    def info(self):
        response = self.command(bytes([1]) + bytes(7) + b"Signalman", 5)
        if len(response) < 58:
            raise RuntimeError("short stock self info")
        device = self.command(bytes([22, 3]), 13)
        version = device[60:80].split(b"\0")[0].decode().strip()
        if version != "v1.17.1-d929643":
            raise RuntimeError(f"unexpected stock release: {version}")
        return {"key": response[4:36].hex(), "power_dbm": int.from_bytes(response[2:3], "little", signed=True),
                "frequency_khz": int.from_bytes(response[48:52], "little"),
                "bandwidth_hz": int.from_bytes(response[52:56], "little"),
                "sf": response[56], "cr": response[57], "version": version}

    def contacts(self):
        first = self.command(bytes([4]), 2)
        if len(first) != 5 or int.from_bytes(first[1:], "little") > 1024:
            raise RuntimeError("stock contact-list bounds")
        contacts = []
        for _ in range(2048):
            response = self.read()
            if response[0] == 3 and len(response) >= 148:
                contacts.append(response[1:33].hex())
            elif response[0] == 4:
                return contacts
            elif response[0] < 128:
                raise RuntimeError("unexpected stock contact-list response")
        raise RuntimeError("stock contact-list frame limit")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("mode", choices=["status", "snapshot", "restore-profile", "stock-setup", "advertise"])
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--port")
    p.add_argument("--usb-serial")
    p.add_argument("--baseline", type=Path)
    p.add_argument("--peer-key")
    p.add_argument("--public-card", type=Path)
    p.add_argument("--frequency-khz", type=int, default=906875)
    p.add_argument("--bandwidth-hz", type=int, default=250000)
    p.add_argument("--sf", type=int, default=10)
    p.add_argument("--cr", type=int, default=5)
    p.add_argument("--power-dbm", type=int, default=0)
    a = p.parse_args()
    if a.output.exists():
        p.error("preserve earlier receipts; select a new output file")
    report = {"mode": a.mode, "result": "not_completed", "port": a.port, "usb_serial": a.usb_serial}
    try:
        if not a.port or not a.usb_serial:
            p.error("radio commands require --port and --usb-serial")
        if a.mode in ("snapshot", "restore-profile"):
            probe = identified(a.port, a.usb_serial, False)
            probe.close()
            from mc3_personality_bench import snapshot, restore_and_check
            transcript = []
            if a.mode == "snapshot":
                previous = json.loads(a.baseline.read_text())["baseline"] if a.baseline else None
                report["baseline"] = snapshot(a.port, False, transcript, previous)
                report["baseline_reapplied"] = previous is not None
            else:
                baseline = json.loads(a.baseline.read_text())["baseline"]
                report["restored"] = restore_and_check(a.port, False, baseline, transcript)
            report["transcript"] = transcript
        else:
            with identified(a.port, a.usb_serial, a.mode in ("stock-setup", "advertise")) as link:
                if a.mode == "status":
                    for command in ["status", "region", "channel"]:
                        link.write(command.encode() + b"\n")
                        link.flush()
                        until, received = time.monotonic() + 1.2, bytearray()
                        while time.monotonic() < until:
                            received.extend(link.read(4096))
                        report[command] = received.decode(errors="replace")
                    if "tulle/heltec-v4 phy online" not in report["status"] or "region=US915" not in report["region"]:
                        raise RuntimeError("expected live V4 Tulle in US915")
                else:
                    stock = Stock(link)
                    report["before"] = stock.info()
                    if a.mode == "stock-setup":
                        if not a.peer_key or len(bytes.fromhex(a.peer_key)) != 32:
                            p.error("stock setup requires a full --peer-key")
                        stock.command(bytes([11]) + struct.pack("<IIBBB", a.frequency_khz, a.bandwidth_hz, a.sf, a.cr, 0))
                        stock.command(bytes([12, a.power_dbm & 255]))
                        stock.command(bytes([8]) + b"Signalman stock peer")
                        clock = int.from_bytes(stock.command(bytes([5]), 9)[1:5], "little")
                        timestamp = max(clock, int(time.time())) + 2
                        stock.command(bytes([6]) + struct.pack("<I", timestamp))
                        if a.public_card:
                            card = bytes.fromhex(json.loads(a.public_card.read_text())["advert_hex"])
                            if len(card) < 102 or card[2:34].hex() != a.peer_key:
                                raise RuntimeError("public business card does not match the expected peer")
                            stock.command(bytes([18]) + card)
                            stock.command(bytes([13]) + bytes.fromhex(a.peer_key))
                            report["public_business_card"] = card.hex()
                        else:
                            contact = bytes.fromhex(a.peer_key) + bytes([1, 0, 255]) + bytes(64) + b"Tucket bench".ljust(32, b"\0") + struct.pack("<IiiI", timestamp, 0, 0, timestamp)
                            stock.command(bytes([9]) + contact)
                        report["after"] = stock.info()
                        actual = report["after"]
                        if (actual["frequency_khz"], actual["bandwidth_hz"], actual["sf"], actual["cr"], actual["power_dbm"]) != (a.frequency_khz, a.bandwidth_hz, a.sf, a.cr, a.power_dbm):
                            raise RuntimeError("stock profile readback mismatch")
                        report["contact_key"] = a.peer_key
                        contacts = stock.contacts()
                        report["contacts_count"] = len(contacts)
                        report["contact_hash_matches"] = sum(key[:2] == a.peer_key[:2] for key in contacts)
                        report["contact_verified"] = a.peer_key in contacts
                        if not report["contact_verified"]:
                            raise RuntimeError("stock contact readback mismatch")
                    else:
                        stock.command(bytes([7, 1]))
                        report["advertisement_accepted"] = True
        report["result"] = "passed"
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        a.output.write_text(json.dumps(report, indent=2))
    print(json.dumps({k: report[k] for k in ("result", "public_key") if k in report}))


if __name__ == "__main__":
    main()
