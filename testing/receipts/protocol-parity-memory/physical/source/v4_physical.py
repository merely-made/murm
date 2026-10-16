"""Single identified COM7 memory campaign, with application-only restoration."""
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
output = work / "v4-physical.json"
assert not output.exists()
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert digest(tool) == "0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e"
assert digest(work / "v4-physical.bin") == "5f8bb53fea09941e906612d8f8c3a906c74bcdf9df6de57a4b53c55f4412b961"
assert digest(work / "v4-original-private-prefix.bin") == "37285b729fac58354103d50e9faf9b77f78c90f769f6cdd404f973f4ab4230a3"
assert digest(work / "v4-original-private-tail.bin") == "acba765c11cc2bdef36e5a39064ecb401f5be9ca96f371947d47b2b6d49fb0a0"
original = (work / "v4-original-private-prefix.bin").read_bytes()
assert (work / "v4-original-private-app.bin").read_bytes() == original[0x10000:]
assert len(original) == 0x100000
assert (work / "v4-physical.bin").stat().st_size + 0x10000 <= len(original)
report = {"passed": False, "actions": [], "application_only_write_offset": 0x10000,
          "protected_range": "0x3f0000..0x400000", "inputs": {}}
for name in ["v4-physical.bin", "v4-physical.elf", "v4-original-private-prefix.bin",
             "v4-original-private-app.bin", "v4-original-private-tail.bin"]:
    report["inputs"][name] = {"sha256": digest(work/name), "bytes": (work/name).stat().st_size}

def identified():
    p = next(p for p in comports() if p.device == "COM7")
    assert (p.vid, p.pid, p.serial_number) == (0x303a, 0x1001, "44:1B:F6:6A:FA:64")

def run(name, args, timeout=100):
    identified()
    start = time.monotonic()
    with (work / (name + ".log")).open("x") as log:
        result = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    report["actions"].append({"name": name, "command": list(map(str,args)),
                              "exit_code": result.returncode, "seconds": round(time.monotonic()-start,3)})
    assert result.returncode == 0, name + " failed"

def flash(name, image):
    run(name, [str(tool), "write-bin", "--port", "COM7", "--chip", "esp32s3",
               "--before", "usb-reset", "--baud", "921600", "--non-interactive",
               "--skip-update-check", "65536", str(work/image)])
    time.sleep(2)

def read(name, address, length, filename):
    assert not (work/filename).exists()
    run(name, [str(tool), "read-flash", "--port", "COM7", "--chip", "esp32s3",
               "--before", "usb-reset", "--baud", "921600", "--non-interactive",
               "--skip-update-check", "--block-size", "1024", "--max-in-flight", "1",
               str(address), str(length), str(work/filename)])
    time.sleep(2)

try:
    flash("v4-install", "v4-physical.bin")
    read("v4-installed-read", 0x10000, (work/"v4-physical.bin").stat().st_size, "v4-installed-private-app.bin")
    assert (work/"v4-installed-private-app.bin").read_bytes() == (work/"v4-physical.bin").read_bytes()
    report["installed_application_exact"] = True
    run("v4-campaign", [sys.executable, "-B", "testing/protocol-capacity/board_probe.py", "v4",
                         "--port", "COM7", "--rounds", "16", "--output", str(work/"v4-campaign.json")], 300)
    report["campaign_passed"] = True
except Exception as error:
    report["error"] = repr(error)
finally:
    try:
        flash("v4-restore", "v4-original-private-app.bin")
        read("v4-restored-prefix-read", 0, len(original), "v4-restored-private-prefix.bin")
        read("v4-restored-tail-read", 0x3f0000, 0x10000, "v4-restored-private-tail.bin")
        report["original_prefix_exact"] = (work/"v4-restored-private-prefix.bin").read_bytes() == original
        report["protected_tail_exact"] = (work/"v4-restored-private-tail.bin").read_bytes() == (work/"v4-original-private-tail.bin").read_bytes()
        run("v4-restored-status", [sys.executable, "-B", "testing/protocol-capacity/board_probe.py", "v4",
                                  "--port", "COM7", "--output", str(work/"v4-restored.json")])
        before = json.loads((work/"v4-before.json").read_text())
        after = json.loads((work/"v4-restored.json").read_text())
        report["original_status_profile_exact"] = before["responses"][:4] != [] and all(
            b["response"] == a["response"] for b,a in zip(before["responses"][:4], after["responses"][:4]))
        report["passed"] = all(report.get(key,False) for key in ["campaign_passed", "original_prefix_exact", "protected_tail_exact", "original_status_profile_exact"])
    except Exception as error:
        report["restoration_error"] = repr(error)
    output.write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps(report, indent=2))
raise SystemExit(0 if report["passed"] else 1)
