"""Measure separately enabled upstream graphs; never retain full Cargo metadata."""

import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess


def run(*args):
    result = subprocess.run(args, check=True, text=True, capture_output=True)
    return result.stdout


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def version_key(version):
    return tuple(int(part) for part in version.split(".")[:3])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    manifest = root / "Cargo.toml"
    rustc = run("rustc", "-vV").strip()
    target = next(line[6:] for line in rustc.splitlines() if line.startswith("host: "))
    report = {
        "schema": 1,
        "generated_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "scope": "Separately enabled normal/build dependency graphs for the host target; excludes the probe package. Not binary size, performance, MSRV execution or shipped consumer lockfiles.",
        "manifest_sha256": digest(manifest),
        "lock_sha256": digest(root / "Cargo.lock"),
        "cargo": run("cargo", "--version").strip(),
        "rustc": rustc,
        "target": target,
        "cases": {},
    }
    forbidden = {
        "murm", "murm-webrtc", "mere-transport", "personae", "notochord",
        "luggage", "muniment", "stickleback", "genet-runtime", "netfetcher",
    }
    for feature in ("str0m-core", "rtc-core", "async-api"):
        metadata = json.loads(run(
            "cargo", "metadata", "--format-version", "1", "--locked",
            "--manifest-path", str(manifest), "--no-default-features",
            "--features", feature, "--filter-platform", target,
        ))
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        packages = {package["id"]: package for package in metadata["packages"]}
        seen = set()
        pending = [metadata["resolve"]["root"]]
        while pending:
            ident = pending.pop()
            if ident in seen:
                continue
            seen.add(ident)
            for dep in nodes[ident]["deps"]:
                if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                    pending.append(dep["pkg"])
        seen.discard(metadata["resolve"]["root"])
        selected = [packages[ident] for ident in seen]
        names = {package["name"] for package in selected}
        known_floors = [package["rust_version"] for package in selected if package["rust_version"]]
        report["cases"][feature] = {
            "transitive_package_versions": len(selected),
            "build_script_packages": sorted(
                f'{p["name"]}@{p["version"]}' for p in selected
                if any(t["kind"] == ["custom-build"] for t in p["targets"])
            ),
            "highest_declared_rust_version": max(known_floors, key=version_key),
            "crypto_and_runtime_packages": sorted(
                f'{p["name"]}@{p["version"]}' for p in selected
                if p["name"] in {"ring", "aws-lc-rs", "aws-lc-sys", "str0m-rust-crypto", "dimpl", "tokio", "smol", "cc"}
            ),
            "forbidden_upper_layer_packages": sorted(names & forbidden),
            "packages": sorted(f'{p["name"]}@{p["version"]}' for p in selected),
        }
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        # Cited receipts are immutable: reruns print by default or need a new path.
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as output:
            output.write(encoded)
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
