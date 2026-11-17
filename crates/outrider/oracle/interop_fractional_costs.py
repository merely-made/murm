"""Run existing delivery and propagation exchanges at integer and fractional costs.

All assertions stay in the existing mixed-runtime drivers. This runner only selects
their public cost parameters and already built debug/release example executables.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


HERE = Path(__file__).resolve().parent
GATES = ("interop_direct_receive.py", "interop_direct_send.py",
         "interop_propagation_server.py", "interop_propagation_float_limits.py")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    target = args.target.resolve()
    suffix = ".exe" if os.name == "nt" else ""
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    for variable, profile, example in (
        ("OUTRIDER_DIRECT_SEND", "debug", "stock_direct_send"),
        ("OUTRIDER_DIRECT_RECEIVE", "debug", "stock_direct_receive"),
        ("OUTRIDER_PROPAGATION_SEND", "debug", "stock_propagation_send"),
        ("OUTRIDER_PROPAGATION_SERVER", "release", "stock_propagation_server"),
    ):
        binary = target / profile / "examples" / (example + suffix)
        if not binary.is_file():
            parser.error(f"missing prebuilt example {binary}")
        env[variable] = str(binary)
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for cost in (8, 8.5):
        for gate in GATES:
            name = f"{cost}-{Path(gate).stem}"
            print("START", name, flush=True)
            env["OUTRIDER_STAMP_COST"] = str(cost)
            env["OUTRIDER_PROPAGATION_COST"] = str(cost)
            with (args.output / f"{name}.log").open("xb") as log:
                proc = subprocess.Popen([sys.executable, "-u", str(HERE / gate)], cwd=HERE,
                                        env=env, stdout=log, stderr=subprocess.STDOUT)
                try:
                    results[name] = proc.wait(timeout=240)
                except subprocess.TimeoutExpired:
                    if os.name == "nt":
                        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    else:
                        proc.kill()
                    proc.wait()
                    results[name] = "timeout"
            (args.output / "result.json").write_text(json.dumps(results, indent=2) + "\n")
            print("RESULT", name, results[name], flush=True)
    return int(any(results.values()))


if __name__ == "__main__":
    raise SystemExit(main())
