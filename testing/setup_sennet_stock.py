"""Configure the identified disposable stock bench peer through its untouched CLI.

The GPL stock client runs as a black-box tool/oracle. No implementation or
generated schema source is read/copied into Sennet. Only selected public
metadata and the intentionally public LongFast key are admitted to receipts.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time
from serial.tools.list_ports import comports


def identified(port, usb):
    found = next((p for p in comports() if p.device.casefold() == port.casefold()), None)
    if found is None or (found.vid, found.pid, found.serial_number) != (0x303A, 0x1001, usb):
        raise RuntimeError("stock peer USB identity mismatch")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--port", required=True)
    p.add_argument("--usb-serial", required=True)
    p.add_argument("--expected-node", required=True, type=lambda s: int(s, 16))
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--configure", action="store_true")
    a = p.parse_args()
    if a.output.exists():
        p.error("preserve earlier receipt")
    report = {"result": "not_completed", "port": a.port, "usb_serial": a.usb_serial,
              "public_channel": "LongFast", "configured": a.configure}
    try:
        identified(a.port, a.usb_serial)
        if a.configure:
            command = [sys.executable, "-B", "-m", "meshtastic", "--port", a.port,
                       "--set", "lora.region", "US", "--set", "lora.modem_preset", "LONG_FAST",
                       "--set", "lora.use_preset", "true",
                       "--set", "lora.override_frequency", "906.875", "--set", "lora.tx_power", "0",
                       "--ch-index", "0", "--ch-set", "psk", "default", "--ch-set", "name", "LongFast"]
            result = subprocess.run(command, capture_output=True, text=True, timeout=60)
            report["configure_exit"] = result.returncode
            # Never retain unrestricted client config dumps or credential fields.
            if result.returncode:
                raise RuntimeError("stock public-channel configuration refused")
            time.sleep(4)
        identified(a.port, a.usb_serial)
        from meshtastic.serial_interface import SerialInterface
        interface = SerialInterface(devPath=a.port)
        try:
            node = interface.localNode
            lora = node.localConfig.lora
            channel = node.channels[0]
            public = bytes(channel.settings.psk) in (b"\x01", bytes.fromhex("d4f1bb3a20290759f0bcffabcf4e6901"))
            report.update(node_number=interface.myInfo.my_node_num,
                          firmware=interface.metadata.firmware_version,
                          region=int(lora.region), modem_preset=int(lora.modem_preset),
                          override_frequency=float(lora.override_frequency), tx_power=int(lora.tx_power),
                          use_preset=bool(lora.use_preset), channel_role=int(channel.role),
                          channel_name=channel.settings.name, public_default_key_match=public)
            if report["node_number"] != a.expected_node or not report["firmware"].startswith("2.7.26."):
                raise RuntimeError("stock node or firmware differs from the pinned bench")
            if not public or not report["use_preset"] or report["channel_role"] != 1 or report["region"] != 1 or report["modem_preset"] != 0 or report["override_frequency"] != 906.875 or report["tx_power"] != 0 or report["channel_name"] not in ("", "LongFast"):
                raise RuntimeError("stock public LongFast profile readback mismatch")
            report["result"] = "passed"
        finally:
            interface.close()
    except Exception as error:
        # Errors contain our own scope assertions, not arbitrary config data.
        report["error_type"] = type(error).__name__
        raise
    finally:
        a.output.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
