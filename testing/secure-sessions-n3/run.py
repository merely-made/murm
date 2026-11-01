#!/usr/bin/env python3
"""Run independent native server/client processes and check every outcome."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


SCENARIOS = (
    "reconnect",
    "wrong_server",
    "wrong_client",
    "tamper_hello",
    "tamper_ciphertext",
    "cancel",
    "setup_stall",
    "slow_consumer",
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def validate(carrier: str, scenario: str, server: dict, client: dict) -> dict:
    server_steps = server["steps"]
    client_steps = client["steps"]
    expected_count = 3 if scenario == "reconnect" else 1
    require(len(server_steps) == expected_count, "server attempt count")
    require(len(client_steps) == expected_count, "client attempt count")
    require(server["pending_after"] == client["pending_after"] == 0, "setup lease leak")
    require(server["pid"] != client["pid"] and min(server["pid"], client["pid"]) > 0, "independent native process identity")
    require(all(step["pending_after"] == 0 for step in server_steps + client_steps), "per-attempt lease leak")
    for role, report in (("server", server), ("client", client)):
        require(report["role"] == role, "process role attribution")
        require(report["carrier"] == carrier and report["scenario"] == scenario, "report attribution")
        require(report["security_mode"] == ("native" if carrier == "iroh-native" else "noise"), "security mode differs from configured profile")
        require(report["peak_pending"] <= 1, "pending setup cap exceeded")
        for step in report["steps"]:
            if step["outcome"] == "no_stream":
                require((carrier, scenario, role) == ("iroh-native", "wrong_server", "server"), "unexpected undisclosed stream control")
                require(not step["carrier_peer_present"] and not step["shared_link_present"], "facts invented for an undisclosed stream")
                continue
            require(step["carrier_peer_present"] == (carrier in ("iroh", "iroh-native")), "carrier identity relabelled")
            require(step["shared_link_present"] == (carrier == "reticulum"), "shared link missing or invented")
    outcomes = [step["outcome"] for step in server_steps]
    peer_outcomes = [step["outcome"] for step in client_steps]
    if scenario == "reconnect":
        expected = ["responded", "admission_refused", "responded"]
        require(outcomes == peer_outcomes == expected, "reconnect/replay decisions differ")
        hashes = [step["channel_binding"] for step in server_steps]
        require(all(hashes) and len(set(hashes)) == 3, "secure channel reused a session binding")
        require(hashes == [step["channel_binding"] for step in client_steps], "peer channel bindings differ")
        epochs = [server_steps[index]["session_id"] for index in (0, 2)]
        require(all(epochs) and epochs[0] != epochs[1], "admission epoch reused")
        require(epochs == [client_steps[index]["session_id"] for index in (0, 2)], "peer epochs differ")
        require(server["handled_requests"] == server["completed_requests"] == client["completed_requests"] == [1, 3], "request replayed or omitted")
    elif scenario == "slow_consumer":
        require(outcomes == ["application_timeout"], "slow reader was not bounded")
        require(peer_outcomes == ["consumer_stalled"], "slow reader control did not run")
        require(server["handled_requests"] == [1] and server["completed_requests"] == [], "slow service receipt is incorrect")
    else:
        require(not server["handled_requests"] and not server["completed_requests"], "refused peer reached application handler")
        require(not client["completed_requests"], "refused client received application response")
        require("responded" not in outcomes + peer_outcomes, "negative control succeeded")
        if scenario == "tamper_hello":
            require(outcomes == peer_outcomes == ["admission_refused"], "signed transcript tamper was not refused")
            require(client_steps[0]["mutation"] == "signed_hello", "signed transcript mutator did not run")
            require(server_steps[0]["detail"] == "session proof invalid", "hello refused for the wrong reason")
        elif scenario == "tamper_ciphertext":
            require(outcomes == ["admission_io"] and client_steps[0]["mutation"] == "ciphertext", "ciphertext mutation did not fail channel integrity")
        elif scenario == "wrong_client":
            require(outcomes == ["setup_refused"], "wrong secure client key was not refused")
            require(server_steps[0]["detail"] == "PeerOrProtocolMismatch", "client key refused for the wrong reason")
        elif scenario == "wrong_server":
            require(peer_outcomes == ["setup_refused"], "unexpected server key was not refused before admission")
            require(client_steps[0]["detail"] == "PeerOrProtocolMismatch", "publisher refused for the wrong reason")
        elif scenario == "setup_stall":
            expected = "admission_bounded" if carrier == "iroh-native" else "setup_refused"
            require(outcomes == [expected] and server["timed_out"] >= 1, "stalled setup escaped deadline")
        elif scenario == "cancel":
            expected = "admission_io" if carrier == "iroh-native" else "setup_refused"
            require(outcomes == [expected] and peer_outcomes == ["peer_closed"], "cancelled peer did not release setup")
    return {
        "carrier": carrier,
        "scenario": scenario,
        "passed": True,
        "server_outcomes": outcomes,
        "client_outcomes": peer_outcomes,
        "handled_requests": server["handled_requests"],
        "completed_requests": server["completed_requests"],
        "pending_after": 0,
        "server_pid": server["pid"],
        "client_pid": client["pid"],
    }


def run_case(binary: Path, output: Path, carrier: str, scenario: str) -> dict:
    case = output / f"{carrier}-{scenario}"
    case.mkdir(parents=True, exist_ok=False)
    contact = case / "contact.json"
    server_result = case / "server.json"
    client_result = case / "client.json"
    started = time.monotonic()
    with (case / "server.log").open("w") as server_log:
        server = subprocess.Popen(
            [str(binary), "server", carrier, scenario, str(contact), str(server_result)],
            stdout=server_log,
            stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + 15
            while True:
                require(server.poll() is None, f"server exited before contact: {case}")
                if contact.exists():
                    try:
                        json.loads(contact.read_text())
                        break
                    except json.JSONDecodeError:
                        pass
                require(time.monotonic() < deadline, f"contact deadline: {case}")
                time.sleep(0.02)
            client = subprocess.run(
                [str(binary), "client", scenario, str(contact), str(client_result)],
                capture_output=True,
                text=True,
                timeout=45,
            )
            (case / "client.log").write_text(client.stdout + client.stderr)
            require(client.returncode == 0, f"client failed: {case}")
            require(server.wait(timeout=15) == 0, f"server failed: {case}")
            result = validate(carrier, scenario, json.loads(server_result.read_text()), json.loads(client_result.read_text()))
            result["elapsed_seconds"] = round(time.monotonic() - started, 3)
            return result
        finally:
            if server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=3)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--carriers", nargs="+", choices=("iroh", "tcp", "reticulum", "iroh-native"), default=("iroh", "tcp", "reticulum", "iroh-native"))
    parser.add_argument("--scenarios", nargs="+", choices=SCENARIOS, default=SCENARIOS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixture = Path(__file__).resolve().parent
    inputs = [fixture / name for name in ("Cargo.toml", "Cargo.lock", "src/main.rs", "src/carriers.rs", "run.py")]
    transport = fixture.parent.parent / "native/crates/transport"
    transport_inputs = sorted([transport / "Cargo.toml", *transport.rglob("*.rs")])
    digest = hashlib.sha256()
    for path in transport_inputs:
        digest.update(str(path.relative_to(transport)).encode() + b"\0" + hashlib.sha256(path.read_bytes()).digest())
    for prefix, base, paths in (("fixture", fixture, inputs), ("transport", transport, transport_inputs)):
        for path in paths:
            saved = output / "source" / prefix / path.relative_to(base)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, saved)
    workspace_manifest = transport.parent.parent / "Cargo.toml"
    shutil.copyfile(workspace_manifest, output / "source/native-workspace-Cargo.toml")
    applicable = [(carrier, scenario) for carrier in args.carriers for scenario in args.scenarios if (carrier, scenario) != ("iroh-native", "tamper_ciphertext")]
    summary = {
        "evidence_class": "native-two-process-local-network",
        "overall_passed": False,
        "expected_cases": len(applicable),
        "excluded_cases": [{"carrier": "iroh-native", "scenario": "tamper_ciphertext", "reason": "Native iroh exposes application streams above TLS; this fixture cannot alter encrypted QUIC packets."}] if "iroh-native" in args.carriers and "tamper_ciphertext" in args.scenarios else [],
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "fixture_input_sha256": {str(path.relative_to(fixture)): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs},
        "transport_source_sha256": digest.hexdigest(),
        "native_workspace_manifest_sha256": hashlib.sha256(workspace_manifest.read_bytes()).hexdigest(),
        "repository_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=fixture.parent.parent, text=True).strip(),
        "cases": [],
    }
    try:
        require(bool(applicable), "selection contains no applicable process cases")
        for carrier, scenario in applicable:
            result = run_case(binary, output, carrier, scenario)
            summary["cases"].append(result)
            print(f"PASS {carrier} {scenario}", flush=True)
        summary["overall_passed"] = True
    except BaseException as error:
        summary["failure"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS {len(summary['cases'])} native process cases", flush=True)


if __name__ == "__main__":
    main()
