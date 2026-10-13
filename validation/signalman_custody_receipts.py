#!/usr/bin/env python3
# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

"""Qualify the desktop station against an absent and real Locked Djinn.

Run after the desktop suite and keeper build. All processes and state belong
to this runner. A real forbidden local-seed fallback is planted, observed to
fail the same receipts, and restored byte-for-byte before the final green run.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    repo = Path(__file__).resolve().parents[1]
    target = Path("C:/t/cargo-targets/retinue")
    keeper = Path("C:/t/cargo-targets/mere/debug/dr-c-receipt-host.exe")
    runtime = target / "signalman-custody-runtime"
    output = repo / "validation" / "signalman-custody"
    output.mkdir(exist_ok=True)
    if runtime.exists():
        raise RuntimeError("runtime exists; inspect its owner before cleanup")
    runtime.mkdir()
    (runtime / ".signalman-generated").write_text("synthetic custody receipt data\n")
    source = repo / "apps/signalman-desktop/src/station.rs"
    clean = source.read_bytes()
    records = []
    env = os.environ.copy()
    env["CARGO_PROFILE_TEST_DEBUG"] = "0"
    base = ["cargo", "+1.99.0", "test", "--locked", "--offline",
            "--manifest-path", "apps/signalman-desktop/Cargo.toml",
            "--target-dir", str(target), "-j4", "--lib"]

    def run(label, command, expected=0, child_env=None):
        log = output / (label + ".log")
        print(label, flush=True)
        start = time.monotonic()
        with log.open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, cwd=repo, env=child_env or env,
                                    stdout=stream, stderr=subprocess.STDOUT)
        records.append({"label": label, "command": command,
                        "exit": result.returncode, "expected": expected,
                        "seconds": round(time.monotonic() - start, 2),
                        "log": log.name, "sha256": digest(log)})
        if result.returncode != expected:
            raise RuntimeError(f"{label} exited {result.returncode}; see {log}")
        return log

    def build(label):
        log = run(label, base + ["--no-run", "--message-format=json"])
        artifacts = []
        for line in log.read_text(encoding="utf-8").splitlines():
            if not line.startswith("{"):
                continue
            value = json.loads(line)
            if value.get("reason") == "compiler-artifact" and value.get("executable"):
                artifacts.append(Path(value["executable"]))
        if len(artifacts) != 1:
            raise RuntimeError(f"expected one library test binary, got {artifacts}")
        binary = artifacts[0]
        records[-1]["executable_sha256"] = digest(binary)
        return binary

    def receipts(binary, phase, expected=0):
        for mode in ("absent", "locked"):
            endpoint = "\\\\.\\pipe\\signalman-refresh-" + str(uuid.uuid4())
            child_env = {**env, "GRAPHSHELL_APP_ENDPOINT": endpoint,
                         "DR_C_RECEIPT_MODE": mode}
            host = None
            stream = None
            root = runtime / (phase + "-" + mode)
            try:
                if mode == "locked":
                    stream = (output / (phase + "-keeper.log")).open("w")
                    host = subprocess.Popen([str(keeper), endpoint, str(root), "locked"],
                                            stdout=stream, stderr=subprocess.STDOUT)
                    deadline = time.monotonic() + 30
                    while not (root / "ready").exists():
                        if host.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError("isolated keeper did not become ready")
                        time.sleep(0.1)
                    before = {str(p.relative_to(root)): digest(p)
                              for p in root.rglob("*") if p.is_file()}
                log = run(phase + "-" + mode,
                          [str(binary), "desktop_station_pending_leaves_local_state_untouched",
                           "--ignored", "--nocapture", "--test-threads=1"],
                          expected, child_env)
                records[-1]["executable_sha256"] = digest(binary)
                text = log.read_text(encoding="utf-8")
                tripwire = "pending identity must not create local state"
                if expected == 101 and tripwire not in text:
                    raise RuntimeError("negative control failed at the wrong assertion")
                if expected == 0 and "1 passed; 0 failed" not in text:
                    raise RuntimeError("receipt did not execute one passing test")
                if mode == "locked":
                    after = {str(p.relative_to(root)): digest(p)
                             for p in root.rglob("*") if p.is_file()}
                    if before != after:
                        raise RuntimeError("Locked keeper state changed")
                    records[-1]["keeper_state_unchanged"] = True
            finally:
                if host is not None:
                    host.terminate()
                    try:
                        host.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        host.kill()
                        host.wait(timeout=5)
                if stream is not None:
                    stream.close()

    completed = False
    control_sha = None
    try:
        receipts(build("build-green"), "green")
        text = clean.decode("utf-8")
        needle = "    // Request custody before touching local state or the radio."
        fallback = """    // NEGATIVE CONTROL: deliberately restore the forbidden local seed path.
    if authority.status().map(|s| s.wallet_public_key.is_none()).unwrap_or(true) {
        use personae::IdentityProvider;
        let provider = personae::InMemoryProvider::random();
        std::fs::create_dir_all(&settings.data_root).unwrap();
        std::fs::write(settings.data_root.join("master.seed"), provider.master_keypair().to_seed()).unwrap();
        return Err("station identity is pending: forbidden local fallback restored".into());
    }
"""
        if text.count(needle) != 1:
            raise RuntimeError("control insertion point changed")
        source.write_bytes(text.replace(needle, fallback + needle).encode("utf-8"))
        control_sha = digest(source)
        receipts(build("build-negative"), "negative", 101)
        source.write_bytes(clean)
        if digest(source) != hashlib.sha256(clean).hexdigest():
            raise RuntimeError("source was not restored byte-exact")
        receipts(build("build-restored"), "restored")
        completed = True
    finally:
        source.write_bytes(clean)
        # Every host has been waited above. Verify the marker and absolute
        # containment before removing synthetic state, never build/source data.
        if runtime.resolve().parent != target.resolve() or not (runtime / ".signalman-generated").is_file():
            raise RuntimeError("synthetic runtime cleanup boundary changed")
        shutil.rmtree(runtime)
        manifest = {"completed": completed, "toolchain": "1.99.0",
                    "keeper_sha256": digest(keeper),
                    "source_clean_sha256": hashlib.sha256(clean).hexdigest(),
                    "source_control_sha256": control_sha,
                    "source_restored_sha256": digest(source),
                    "runtime_removed": not runtime.exists(), "commands": records}
        (output / "receipt.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
