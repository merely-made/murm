"""Identified T114 single-bank application-only diagnostic and restoration."""
import hashlib
import json
import re
from pathlib import Path
import subprocess
import sys
import time
from zipfile import ZipFile
import serial
from serial.tools.list_ports import comports

root = Path.cwd()
work = root / "validation/results/protocol-parity-memory"
tool = Path(r"C:\Users\mark_\AppData\Local\hermes\hermes-agent\venv\Scripts\adafruit-nrfutil.exe")
output = work / "t114-direct-physical.json"
assert not output.exists()
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert digest(tool) == "458cd93f99c0ac4aa85c5e9a8a5cd9ffde4144c211d499a4ab5cf2cc9704fd9d"
expected = {"t114-direct.zip": "bae64ab21874ea22e6f3417ef76326e0b4fd75480c32a9c65420f4bec951f69c",
            "t114-restore.zip": "c1e0dc9e62cc6f5d100a1e6d7b8581f2c639a7ca65f35ceaecc7ac8e3b94355f",
            "t114-handoff.zip": "7ef7e6c2d50e8aa5e5536cdcda124cb2dda9636dcb40fa6dd9efb32feae41740"}
report = {"passed": False, "actions": [], "inputs": {},
          "state_scope": "application-only DFU, live identity/sequence/profile; no protected-flash readback"}
for name, sha in expected.items():
    assert digest(work/name) == sha
    with ZipFile(work/name) as archive:
        manifest = json.loads(archive.read("manifest.json"))["manifest"]
        assert set(manifest) == {"application", "dfu_version"}
        payload = archive.read(manifest["application"]["bin_file"])
        assert len(payload) % 4 == 0 and 0x26000 + ((len(payload)+4095)//4096)*4096 <= 0xe6000
        report["inputs"][name] = {"sha256": sha, "payload_bytes": len(payload),
                                  "payload_sha256": hashlib.sha256(payload).hexdigest(),
                                  "start": 0x26000, "erase_end": 0x26000+((len(payload)+4095)//4096)*4096}

def loader():
    found = next((p for p in comports() if p.device == "COM4"), None)
    return found is not None and (found.vid, found.pid, found.serial_number) == (0x239a,0x0071,"AA6D3CD5E3BD5626")

def app():
    found = next((p for p in comports() if p.device == "COM10"), None)
    return found is not None and (found.vid, found.pid, found.serial_number) == (0x1915,0x521f,"TULLE-T114-01")

def wait_for(check):
    until = time.monotonic()+15
    while time.monotonic()<until:
        if check(): return
        time.sleep(.25)
    raise TimeoutError("USB identity did not return")

def enter_loader():
    wait_for(lambda: loader() or app())
    if loader(): return
    assert app(), "application identity absent"
    link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
    link.port,link.rts,link.dtr = "COM10",False,True
    with link:
        time.sleep(.3)
        link.read(4096)
        link.write(b"bootloader\n")
        link.flush()
    wait_for(loader)

def run(name, args, timeout=180):
    name = "direct-" + name
    start = time.monotonic()
    with (work/(name+".log")).open("x") as log:
        result = subprocess.run(args,stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
    report["actions"].append({"name":name,"command":list(map(str,args)),
                              "exit_code":result.returncode,"seconds":round(time.monotonic()-start,3)})
    assert result.returncode == 0, name + " failed"

def flash(name, package, expect_application=True):
    assert loader(), "serial loader identity mismatch"
    run(name,[str(tool),"dfu","serial","--package",str(work/package),"--port","COM4",
              "--baudrate","115200","--singlebank"])
    assert "Device programmed." in (work/("direct-"+name+".log")).read_text(), "DFU completion absent"
    if expect_application:
        wait_for(app)
    else:
        time.sleep(12)
        wait_for(loader)

try:
    enter_loader()
    flash("t114-stage", "t114-handoff.zip", False)
    flash("t114-install","t114-direct.zip")
    run("t114-campaign",[sys.executable,"-B","testing/protocol-capacity/board_probe.py","t114",
                          "--port","COM10","--rounds","16","--output",str(work/"t114-direct-campaign.json")],300)
    report["campaign_passed"] = True
except Exception as error:
    report["error"] = repr(error)
finally:
    try:
        enter_loader()
        flash("t114-restore-stage", "t114-handoff.zip", False)
        flash("t114-restore","t114-restore.zip")
        run("t114-restored-status",[sys.executable,"-B","testing/protocol-capacity/board_probe.py","t114",
                                   "--port","COM10","--output",str(work/"t114-direct-restored.json")])
        before = json.loads((work/"t114-before.json").read_text())
        after = json.loads((work/"t114-direct-restored.json").read_text())
        normalize = lambda s: re.sub(r"reset=\w+ crash=\d+", "reset=HANDOFF crash=HANDOFF", s)
        report["original_status_profile_exact"] = all(normalize(b["response"])==normalize(a["response"]) for b,a in zip(before["responses"][:4],after["responses"][:4]))
        report["reset_comparison_scope"] = "Intentional watchdog bridge changes boot cause/crash residue; only those two fields normalized. Campaign itself requires identical before/after status."
        report["original_identity_sequence_node"] = all(s in after["attached"] for s in ["identity=loaded slot=B seq=93", "node=599997c8"])
        report["passed"] = all(report.get(k,False) for k in ["campaign_passed","original_status_profile_exact","original_identity_sequence_node"])
    except Exception as error:
        report["restoration_error"] = repr(error)
    output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(report,indent=2))
raise SystemExit(0 if report["passed"] else 1)
