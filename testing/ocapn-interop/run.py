#!/usr/bin/env python3
"""Run a declared subset of the immutable, independent OCapN test suite."""

from __future__ import annotations

import argparse
import hashlib
import importlib
import importlib.metadata
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
import unittest


SUITE_COMMIT = "31f0b806108a55d76a6ede5af7e413818dfb9221"
SELECTED = (
    ("op_start_session", "OpStartSessionTest", "test_captp_remote_version"),
    ("op_start_session", "OpStartSessionTest", "test_start_session_with_invalid_version"),
    ("op_start_session", "OpStartSessionTest", "test_start_session_with_invalid_signature"),
    ("op_abort", "OpAbortTest", "test_abort_before_setup"),
    ("op_deliver", "OpDeliverTest", "test_send_deliver_no_answer_or_response"),
    ("op_deliver", "OpDeliverTest", "test_deliver_with_resolver"),
    ("op_deliver", "OpDeliverTest", "test_deliver_promise_pipeline"),
    ("op_deliver", "OpDeliverTest", "test_promise_pipeline_with_break"),
    ("op_listen", "OpListenTest", "test_op_listen_to_promise_and_fulfill"),
    ("op_listen", "OpListenTest", "test_op_listen_to_promise_and_break"),
    ("op_listen", "OpListenTest", "test_op_listen_already_has_answer"),
    ("op_gc", "OpGcExportsTest", "test_gc_export_emitted_single_object"),
    ("op_gc", "OpGcExportsTest", "test_gc_export_with_multiple_refrences"),
    ("op_gc", "OpGcExportsTest", "test_gc_export_with_multiple_refrences_in_different_messages"),
    ("op_gc", "OpGcAnswersTest", "test_gc_answer"),
)
EXCLUDED = {
    "crossed_hellos": [
        "tests.op_start_session.OpStartSessionTest.test_crossed_hellos_mitigation_aborts_inbound",
        "tests.op_start_session.OpStartSessionTest.test_crossed_hellos_mitigation_outbound_aborts",
    ],
    "third_party_handoffs": "All tests in tests.third_party_handoffs (unsupported by this bounded profile).",
    "other_netlayers": "Tor, native iroh and Noise netlayer interoperability are not exercised by this testing-only TCP suite run.",
    "current_draft": "The pinned suite's CapTP 1.0 wire subset is exercised; no claim of conformance to every later draft revision.",
}


def require(value: bool, message: str) -> None:
    if not value:
        raise AssertionError(message)


def selected_ids() -> list[str]:
    return [f"tests.{module}.{case}.{method}" for module, case, method in SELECTED]


def supplier_state(path: Path) -> dict:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path, text=True).strip()
    require(head == SUITE_COMMIT, f"supplier HEAD must be {SUITE_COMMIT}")
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=path, text=True)
    require(not status.strip(), "supplier source has changes")
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=path).decode().split("\0")
    hashes = {name: hashlib.sha256((path / name).read_bytes()).hexdigest() for name in names if name}
    return {"commit": head, "tracked_sha256": hashes}


def runtime_state(path: Path) -> dict[str, str]:
    inputs = sorted([path / "Cargo.toml", *path.rglob("*.rs")])
    return {
        str(source.relative_to(path)): hashlib.sha256(source.read_bytes()).hexdigest()
        for source in inputs
    }


class RecordedResult(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.case_results = []

    def addSuccess(self, test):
        self.case_results.append({"id": test.id(), "outcome": "passed"})
        super().addSuccess(test)

    def addFailure(self, test, error):
        self.case_results.append({"id": test.id(), "outcome": "failed"})
        super().addFailure(test, error)

    def addError(self, test, error):
        self.case_results.append({"id": test.id(), "outcome": "error"})
        super().addError(test, error)

    def addSkip(self, test, reason):
        self.case_results.append({"id": test.id(), "outcome": "skipped", "reason": reason})
        super().addSkip(test, reason)


def run_original_tests(supplier: Path, locator: str, output: Path) -> None:
    # Do not add generated caches or alter any supplier source/tooling.
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(supplier))
    from netlayers.testing_only_tcp import TestingOnlyTCPNetlayer
    from utils.ocapn_uris import OCapNPeer

    peer = OCapNPeer.from_uri(locator)
    require(str(peer.transport) == "tcp-testing-only", "only the declared testing TCP profile is allowed")
    netlayer = TestingOnlyTCPNetlayer("127.0.0.1")
    cases = []
    for module, case, method in SELECTED:
        original = getattr(importlib.import_module(f"tests.{module}"), case)
        cases.append(original(netlayer, peer, "1.0", method))
    try:
        result = unittest.TextTestRunner(verbosity=2, resultclass=RecordedResult).run(unittest.TestSuite(cases))
        passed = (
            result.wasSuccessful()
            and result.testsRun == len(SELECTED)
            and not result.skipped
            and [case["id"] for case in result.case_results] == selected_ids()
            and all(case["outcome"] == "passed" for case in result.case_results)
        )
        report = {
            "overall_passed": passed,
            "tests_run": result.testsRun,
            "case_results": result.case_results,
            "failures": [{"id": test.id(), "traceback": detail} for test, detail in result.failures],
            "errors": [{"id": test.id(), "traceback": detail} for test, detail in result.errors],
            "skipped": [{"id": test.id(), "reason": reason} for test, reason in result.skipped],
        }
        output.write_text(json.dumps(report, indent=2) + "\n")
        require(passed, "the complete declared supplier subset did not pass")
    finally:
        netlayer.shutdown()


def run(args) -> None:
    supplier = args.supplier.resolve(strict=True)
    binary = args.server_binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    summary = {
        "overall_passed": False,
        "evidence_class": "independent-upstream-wire-subset-over-testing-only-tcp",
        "expected_tests": len(SELECTED),
        "selected_ids": selected_ids(),
        "excluded": EXCLUDED,
        "upstream_setup_retry_policy": "Unmodified supplier decorator may retry network-related setUp failures up to three times; this runner adds no retries.",
        "python": sys.version,
        "python_dependencies": {
            name: importlib.metadata.version(name)
            for name in ("cryptography", "stem", "cffi")
        },
        "server_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }
    server = None
    try:
        summary["supplier_before"] = supplier_state(supplier)
        repository = Path(__file__).resolve().parents[2]
        runtime = repository / "native/crates/ocapn"
        runtime_inputs = sorted([runtime / "Cargo.toml", *runtime.rglob("*.rs")])
        summary["runtime_source_sha256"] = runtime_state(runtime)
        summary["native_workspace_lock_sha256"] = hashlib.sha256((repository / "native/Cargo.lock").read_bytes()).hexdigest()
        summary["native_workspace_manifest_sha256"] = hashlib.sha256((repository / "native/Cargo.toml").read_bytes()).hexdigest()
        summary["repository_head"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
        for path in runtime_inputs:
            saved = output / "source/runtime" / path.relative_to(runtime)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, saved)
        shutil.copyfile(Path(__file__), output / "source/run.py")
        shutil.copyfile(repository / "native/Cargo.lock", output / "source/native-Cargo.lock")
        shutil.copyfile(repository / "native/Cargo.toml", output / "source/native-Cargo.toml")
        locator_file = output / "locator.txt"
        with (output / "server.log").open("w") as server_log:
            server = subprocess.Popen([str(binary), str(locator_file)], stdout=server_log, stderr=subprocess.STDOUT)
            summary["server_pid"] = server.pid
            deadline = time.monotonic() + 15
            while not locator_file.exists() or not locator_file.read_text().strip():
                require(server.poll() is None, "suite server exited before publishing its locator")
                require(time.monotonic() < deadline, "suite server locator deadline")
                time.sleep(0.02)
            locator = locator_file.read_text().strip()
            summary["locator"] = locator
            with (output / "supplier-tests.log").open("w") as test_log:
                child = subprocess.run(
                    [sys.executable, str(Path(__file__).resolve()), "--child", "--supplier", str(supplier), "--locator", locator, "--result", str(output / "supplier-result.json")],
                    cwd=supplier,
                    stdout=test_log,
                    stderr=subprocess.STDOUT,
                    timeout=180,
                )
            summary["supplier_exit_code"] = child.returncode
            require(child.returncode == 0, "independent supplier test process failed")
            result = json.loads((output / "supplier-result.json").read_text())
            require(result["overall_passed"] and result["tests_run"] == len(SELECTED), "supplier result is incomplete")
            require(server.poll() is None, "suite server exited unexpectedly during tests")
            summary["supplier_after"] = supplier_state(supplier)
            require(summary["supplier_before"] == summary["supplier_after"], "supplier source changed during execution")
            summary["runtime_source_sha256_after"] = runtime_state(runtime)
            require(summary["runtime_source_sha256"] == summary["runtime_source_sha256_after"], "runtime source changed during execution")
            require(summary["native_workspace_lock_sha256"] == hashlib.sha256((repository / "native/Cargo.lock").read_bytes()).hexdigest(), "workspace lock changed during execution")
            require(summary["native_workspace_manifest_sha256"] == hashlib.sha256((repository / "native/Cargo.toml").read_bytes()).hexdigest(), "workspace manifest changed during execution")
            require(summary["server_binary_sha256"] == hashlib.sha256(binary.read_bytes()).hexdigest(), "server binary changed during execution")
            summary["result"] = result
            summary["overall_passed"] = True
    except BaseException as error:
        summary["failure"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        if server is not None and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=3)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=3)
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS {len(SELECTED)} immutable upstream tests; declared exclusions retained", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--supplier", type=Path, required=True)
    parser.add_argument("--server-binary", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--child", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--locator", help=argparse.SUPPRESS)
    parser.add_argument("--result", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.child:
        run_original_tests(args.supplier.resolve(strict=True), args.locator, args.result)
    else:
        require(args.server_binary is not None and args.output is not None, "server binary and output directory are required")
        run(args)


if __name__ == "__main__":
    main()
