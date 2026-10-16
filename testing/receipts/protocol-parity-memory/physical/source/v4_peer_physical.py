"""Single identified COM6 memory campaign, with application-only restoration."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
from serial.tools.list_ports import comports

root = Path.cwd()
work = root / "validation/results/protocol-parity-memory"
tool = root / "validation/results/sennet-tucket-radios/artifacts/espflash.exe"
output = work / "v4-peer-physical.json"
assert not output.exists()
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert digest(tool) == "0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e"
assert digest(work / "v4-worker.bin") == "b7eb9eb1089dce00cfe45702add75e6987b104de0ab2602356840a1d00381a39"
assert digest(work / "v4-peer-original-private-prefix.bin") == "76336208b23f990f35968df6081fd1fb429a752a76d353b0be7bdd28f322c01e"
assert digest(work / "v4-peer-original-private-tail-retry.bin") == "4118655d606257118b01262653dbd4ac49f2efff4c17ddd5c2d99a2ba0f3ab9e"
original = (work / "v4-peer-original-private-prefix.bin").read_bytes()
assert (work / "v4-peer-original-private-app.bin").read_bytes() == original[0x10000:]
assert len(original) == 0x100000
assert (work / "v4-worker.bin").stat().st_size + 0x10000 <= len(original)
report = {"passed": False, "actions": [], "application_only_write_offset": 0x10000,
          "protected_range": "0x3f0000..0x400000", "inputs": {}}
for name in ["v4-worker.bin", "v4-worker.elf", "v4-peer-original-private-prefix.bin",
             "v4-peer-original-private-app.bin", "v4-peer-original-private-tail-retry.bin"]:
    report["inputs"][name] = {"sha256": digest(work/name), "bytes": (work/name).stat().st_size}

def identified():
    p = next(p for p in comports() if p.device == "COM6")
    assert (p.vid, p.pid, p.serial_number) == (0x303a, 0x1001, "44:1B:F6:6A:FB:28")

def run(name, args, timeout=100):
    name = "peer-" + name
    identified()
    start = time.monotonic()
    with (work / (name + ".log")).open("x") as log:
        result = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    report["actions"].append({"name": name, "command": list(map(str,args)),
                              "exit_code": result.returncode, "seconds": round(time.monotonic()-start,3)})
    assert result.returncode == 0, name + " failed"

def flash(name, image):
    run(name, [str(tool), "write-bin", "--port", "COM6", "--chip", "esp32s3",
               "--before", "usb-reset", "--after", "watchdog-reset", "--baud", "921600", "--non-interactive",
               "--skip-update-check", "65536", str(work/image)])
    time.sleep(6)

def read(name, address, length, filename):
    assert not (work/filename).exists()
    run(name, [str(tool), "read-flash", "--port", "COM6", "--chip", "esp32s3",
               "--before", "usb-reset", "--after", "watchdog-reset", "--baud", "921600", "--non-interactive",
               "--skip-update-check", "--block-size", "1024", "--max-in-flight", "1",
               str(address), str(length), str(work/filename)])
    time.sleep(6)

try:
    flash("v4-install", "v4-worker.bin")
    run("v4-campaign", [sys.executable, "-B", "testing/protocol-capacity/board_probe.py", "v4",
                         "--port", "COM6", "--usb-serial", "44:1B:F6:6A:FB:28", "--rounds", "16", "--output", str(work/"v4-peer-campaign.json")], 300)
    report["campaign_passed"] = True
    read("v4-installed-read", 0x10000, (work/"v4-worker.bin").stat().st_size, "v4-peer-installed-private-app.bin")
    assert (work/"v4-peer-installed-private-app.bin").read_bytes() == (work/"v4-worker.bin").read_bytes()
    report["installed_application_exact"] = True
except Exception as error:
    report["error"] = repr(error)
finally:
    try:
        flash("v4-restore", "v4-peer-original-private-app.bin")
        read("v4-restored-prefix-read", 0, len(original), "v4-peer-restored-private-prefix.bin")
        read("v4-restored-tail-read", 0x3f0000, 0x10000, "v4-peer-restored-private-tail.bin")
        report["original_prefix_exact"] = (work/"v4-peer-restored-private-prefix.bin").read_bytes() == original
        report["protected_tail_exact"] = (work/"v4-peer-restored-private-tail.bin").read_bytes() == (work/"v4-peer-original-private-tail-retry.bin").read_bytes()
        run("v4-restored-status", [sys.executable, "-B", "testing/protocol-capacity/board_probe.py", "v4",
                                  "--port", "COM6", "--usb-serial", "44:1B:F6:6A:FB:28", "--output", str(work/"v4-peer-restored.json")])
        before = json.loads((work/"v4-peer-probe-before.json").read_text())
        after = json.loads((work/"v4-peer-restored.json").read_text())
        report["original_status_profile_exact"] = before["responses"][:4] != [] and all(
            b["response"] == a["response"] for b,a in zip(before["responses"][:4], after["responses"][:4]))
        report["passed"] = all(report.get(key,False) for key in ["campaign_passed", "installed_application_exact", "original_prefix_exact", "protected_tail_exact", "original_status_profile_exact"])
    except Exception as error:
        report["restoration_error"] = repr(error)
    output.write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps(report, indent=2))
raise SystemExit(0 if report["passed"] else 1)
