#!/usr/bin/env python3
# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0
"""Regenerate independent OpenRQ vectors and compare two process peers.

The default checks the committed corpus without modifying it. External source,
JDK archives and build output stay in the explicitly supplied tools directory.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import tarfile
import urllib.request

HERE = Path(__file__).resolve().parent
SUITE = HERE.parent
COMMITTED = SUITE / "vectors" / "openrq-interop.json"
REVISION = "45f4437ba96d7e69a6ee05a2e51cf79cda90448e"
SOURCE_URL = "https://codeload.github.com/openrq-team/OpenRQ/tar.gz/" + REVISION
SOURCE_SHA256 = "018fbf8f77bdce497596c52cd57bdd2c7b4389cb2b1393f08ed21f68e3883e56"
JDK_RELEASE = "jdk-21.0.8+9"
JDK_NAME = "OpenJDK21U-jdk_aarch64_mac_hotspot_21.0.8_9.tar.gz"
JDK_URL = ("https://github.com/adoptium/temurin21-binaries/releases/download/"
           "jdk-21.0.8%2B9/" + JDK_NAME)
JDK_SHA256 = "59422c2292ae4e76b87e00d8808dbe49cffa39af731e08bb0292ddb0af4e0261"
REQUEST_LIMIT = 512 * 1024
PACKET_LIMIT = 1024


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def canonical(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def verify_archive(path, expected):
    require(path.is_file(), "infrastructure unavailable: missing " + str(path)
            + "; run with --bootstrap on macOS arm64 to retrieve pinned tools")
    require(digest(path) == expected, "archive SHA256 mismatch: " + str(path))


def download(path, url, expected):
    if path.exists():
        verify_archive(path, expected)
        return
    temporary = path.with_suffix(path.suffix + ".download")
    try:
        with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as output:
            shutil.copyfileobj(response, output)
        require(digest(temporary) == expected, "download SHA256 mismatch: " + url)
        temporary.replace(path)
    finally:
        if temporary.exists():
            temporary.unlink()


def extract(archive, destination, omit=()):
    # Validate paths and links before extraction; neither external archive is a git input.
    with tarfile.open(archive, "r:gz") as contents:
        root = destination.resolve()
        members = [member for member in contents.getmembers() if member.name not in omit]
        for member in members:
            target = (root / member.name).resolve()
            require(target == root or root in target.parents, "unsafe archive member path")
            if member.issym() or member.islnk():
                base = target.parent if member.issym() else root
                link = (base / member.linkname).resolve()
                require(link == root or root in link.parents, "unsafe archive link")
            require(not (member.ischr() or member.isblk() or member.isfifo()),
                    "unsafe archive special file")
        contents.extractall(destination, members=members)


def tree_digest(root):
    hasher = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if path.is_file():
            name = path.relative_to(root).as_posix().encode("utf-8")
            data = path.read_bytes()
            hasher.update(struct.pack(">I", len(name)))
            hasher.update(name)
            hasher.update(struct.pack(">Q", len(data)))
            hasher.update(data)
    return hasher.hexdigest()


def prepare_peer(cache, bootstrap):
    require(platform.system() == "Darwin" and platform.machine() == "arm64",
            "infrastructure unavailable: pinned portable JDK profile is macOS arm64")
    # Permit an external cache, or an explicitly git-ignored in-checkout cache.
    cache = cache.resolve()
    repository = subprocess.run(["git", "-C", str(SUITE), "rev-parse", "--show-toplevel"],
                                check=True, capture_output=True, text=True, timeout=60).stdout.strip()
    root = Path(repository).resolve()
    if cache == root or root in cache.parents:
        ignored = subprocess.run(["git", "-C", str(root), "check-ignore", "--quiet", str(cache)],
                                 capture_output=True, timeout=60).returncode == 0
        require(ignored, "tools directory inside git checkout must be explicitly git ignored")
    cache.mkdir(parents=True, exist_ok=True)
    source_archive = cache / "openrq-45f4437.tar.gz"
    jdk_archive = cache / "temurin-21.0.8.tar.gz"
    if bootstrap:
        download(source_archive, SOURCE_URL, SOURCE_SHA256)
        download(jdk_archive, JDK_URL, JDK_SHA256)
    verify_archive(source_archive, SOURCE_SHA256)
    verify_archive(jdk_archive, JDK_SHA256)
    # Re-extract verified archives on every run so an edited external checkout is not an oracle.
    source = cache / ("OpenRQ-" + REVISION)
    jdk = cache / "jdk-21.0.8+9"
    for directory in (source, jdk):
        if directory.exists():
            shutil.rmtree(directory)
    # The pinned upstream archive contains an unused src/opt absolute symlink into
    # the author's home directory. Never create it; main sources/resources suffice.
    extract(source_archive, cache, omit=("OpenRQ-" + REVISION + "/src/opt",))
    extract(jdk_archive, cache)
    java_home = jdk / "Contents" / "Home"
    classes = cache / "openrq-peer-classes"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir()
    sources = sorted(str(path) for path in (source / "src" / "main").rglob("*.java"))
    require(bool(sources), "OpenRQ source archive has no main sources")
    command = [str(java_home / "bin" / "javac"), "--release", "8", "-g:none", "-d",
               str(classes)] + sources + [str(HERE / "OpenRqPeer.java")]
    subprocess.run(command, check=True, capture_output=True, timeout=60)
    shutil.copytree(source / "src" / "resources", classes, dirs_exist_ok=True)
    require(digest(source / "LICENSE") == digest(HERE / "OpenRQ-LICENSE.txt"),
            "committed OpenRQ license differs from pinned source")
    java_version = subprocess.run([str(java_home / "bin" / "java"), "-version"],
                                  check=True, capture_output=True, text=True, timeout=60).stderr.strip()
    require("Temurin-21.0.8+9" in java_version, "unexpected pinned JDK version")
    provenance = {
        "implementation": "OpenRQ",
        "repository": "https://github.com/openrq-team/OpenRQ",
        "revision": REVISION,
        "source_archive_url": SOURCE_URL,
        "source_archive_sha256": SOURCE_SHA256,
        "source_main_tree_sha256": tree_digest(source / "src" / "main"),
        "source_resources_tree_sha256": tree_digest(source / "src" / "resources"),
        "unused_source_archive_member_omitted": "src/opt (absolute symlink; unused by main build)",
        "upstream_license": "Apache-2.0",
        "upstream_license_url": "https://github.com/openrq-team/OpenRQ/blob/" + REVISION + "/LICENSE",
        "license_path": "interop/OpenRQ-LICENSE.txt",
        "license_sha256": digest(HERE / "OpenRQ-LICENSE.txt"),
        "toolchain": {"vendor": "Eclipse Temurin", "release": JDK_RELEASE,
                      "platform": "macOS aarch64", "archive_url": JDK_URL,
                      "archive_sha256": JDK_SHA256,
                      "checksum_url": JDK_URL + ".sha256.txt",
                      "license": "GPL-2.0 WITH Classpath-exception-2.0",
                      "javac_options": ["--release", "8", "-g:none"],
                      "java_version": java_version},
        "java_peer_sha256": digest(HERE / "OpenRqPeer.java"),
        "generator_sha256": digest(Path(__file__).resolve()),
        "compiled_classes_tree_sha256": tree_digest(classes),
        "normalization": "OpenRQ omits final-source padding; wrapper restores zero bytes to T. "
                         "OTI and payload IDs are passed exactly; repair symbols are unchanged.",
    }
    return [str(java_home / "bin" / "java"), "-Xmx256m", "-cp", str(classes), "OpenRqPeer"], provenance


def payload_id(sbn, esi):
    require(0 <= sbn < 256 and 0 <= esi <= 4095, "bounded payload ID")
    return bytes([sbn]) + esi.to_bytes(3, "big")


def request_encode(oti, data, ids):
    require(len(ids) <= PACKET_LIMIT, "packet count")
    request = b"RQI1E" + oti + struct.pack(">I", len(data)) + data + struct.pack(">H", len(ids))
    return request + b"".join(payload_id(sbn, esi) for sbn, esi in ids)


def request_decode(oti, packets):
    require(len(packets) <= PACKET_LIMIT, "packet count")
    return (b"RQI1D" + oti + struct.pack(">H", len(packets))
            + b"".join(struct.pack(">H", len(packet)) + packet for packet in packets))


def call(command, request):
    require(len(request) <= REQUEST_LIMIT, "request count")
    result = subprocess.run(command, input=request, capture_output=True, timeout=60)
    require(result.returncode == 0, "peer failed: " + result.stderr.decode("utf-8", errors="replace").strip())
    require(len(result.stdout) <= REQUEST_LIMIT, "peer response exceeds limit")
    return result.stdout


def parse_encoded(response, ids, symbol_size):
    require(response[:5] == b"RQO1E", "encoder response magic")
    require(len(response) >= 7, "truncated encoder response")
    count = struct.unpack(">H", response[5:7])[0]
    require(count == len(ids), "encoder response packet count")
    offset = 7
    packets = []
    for sbn, esi in ids:
        require(offset + 2 <= len(response), "truncated encoder response")
        length = struct.unpack(">H", response[offset:offset + 2])[0]
        offset += 2
        require(length == 4 + symbol_size and offset + length <= len(response), "encoder packet length")
        packet = response[offset:offset + length]
        require(packet[:4] == payload_id(sbn, esi), "encoder response payload ID")
        packets.append(packet)
        offset += length
    require(offset == len(response), "trailing encoder response")
    return packets


def parse_decoded(response, expected):
    require(response[:5] == b"RQO1D" and len(response) >= 6, "decoder response magic")
    require(response[5] == 1, "decoder did not complete selected delivery scenario")
    require(len(response) >= 10, "truncated decoder response")
    length = struct.unpack(">I", response[6:10])[0]
    require(length == len(expected) and response[10:] == expected, "decoder object mismatch")
    return response[10:]


def make_cases(java, rust):
    cases = []
    for length, blocks in [(1280, 1), (1281, 1), (1536, 1), (1537, 1), (2561, 2), (4097, 2)]:
        size = 128
        data = bytes((index * 73 + (index >> 3) * 19 + length) % 256 for index in range(length))
        oti = length.to_bytes(5, "big") + b"\0" + struct.pack(">HBHB", size, blocks, 1, 1)
        total = (length + size - 1) // size
        small, large_count = divmod(total, blocks)
        counts = [small + (1 if sbn < large_count else 0) for sbn in range(blocks)]
        source_ids = [(sbn, esi) for sbn, count in enumerate(counts) for esi in range(count)]
        repair_ids = [(sbn, esi) for sbn, count in enumerate(counts) for esi in range(count, 2 * count + 8)]
        ids = source_ids + repair_ids
        packets = parse_encoded(call(java, request_encode(oti, data, ids)), ids, size)
        rust_packets = parse_encoded(call(rust, request_encode(oti, data, ids)), ids, size)
        require(rust_packets == packets, "exact independent encoder symbol mismatch: F%d/Z%d" % (length, blocks))
        delivery = [
            ("all-source", source_ids),
            ("mixed-loss", [item for item in source_ids if item[1] % 3 == 0] + repair_ids),
            ("repair-only", repair_ids),
            ("repair-reversed-duplicates", list(reversed(repair_ids)) + repair_ids[::3]),
        ]
        packet_map = {item: packet for item, packet in zip(ids, packets)}
        rust_map = {item: packet for item, packet in zip(ids, rust_packets)}
        scenarios = []
        for scenario_id, ordered_ids in delivery:
            parse_decoded(call(java, request_decode(oti, [packet_map[item] for item in ordered_ids])), data)
            frames = [rust_map[item] for item in ordered_ids]
            decoded = parse_decoded(call(java, request_decode(oti, frames)), data)
            parse_decoded(call(rust, request_decode(oti, [packet_map[item] for item in ordered_ids])), data)
            scenarios.append({"id": scenario_id,
                              "delivery": [{"sbn": sbn, "esi": esi} for sbn, esi in ordered_ids],
                              "expected": "complete", "java_decoded_object_hex": decoded.hex(),
                              "rust_packets_sha256": hashlib.sha256(b"".join(frames)).hexdigest()})
        cases.append({
            "id": "f%d-t%d-z%d-n1-al1" % (length, size, blocks),
            "object_hex": data.hex(),
            "oti_hex": oti.hex(),
            "blocks": [{"sbn": sbn, "k": count,
                        "k_prime": next(prime for prime in (10, 12, 18) if prime >= count)}
                       for sbn, count in enumerate(counts)],
            "packets": [{"sbn": sbn, "esi": esi, "symbol_hex": packet[4:].hex()}
                        for (sbn, esi), packet in zip(ids, packets)],
            "scenarios": scenarios,
        })
    return cases


def cross_check(java, rust, cases):
    comparisons = 0
    decodes = 0
    observations = []
    for case in cases:
        oti = bytes.fromhex(case["oti_hex"])
        data = bytes.fromhex(case["object_hex"])
        ids = [(packet["sbn"], packet["esi"]) for packet in case["packets"]]
        expected = [payload_id(sbn, esi) + bytes.fromhex(packet["symbol_hex"])
                    for (sbn, esi), packet in zip(ids, case["packets"])]
        rust_packets = parse_encoded(call(rust, request_encode(oti, data, ids)), ids, 128)
        require(rust_packets == expected, "exact encoder symbol mismatch: " + case["id"])
        comparisons += len(ids)
        java_map = dict(zip(ids, expected))
        rust_map = dict(zip(ids, rust_packets))
        for scenario in case["scenarios"]:
            delivery = [(item["sbn"], item["esi"]) for item in scenario["delivery"]]
            parse_decoded(call(rust, request_decode(oti, [java_map[item] for item in delivery])), data)
            parse_decoded(call(java, request_decode(oti, [rust_map[item] for item in delivery])), data)
            decodes += 2
        observations.append({"id": case["id"], "exact_symbol_comparisons": len(ids),
                             "openrq_to_rust_decode_scenarios": len(case["scenarios"]),
                             "rust_to_openrq_decode_scenarios": len(case["scenarios"])})
    return {"status": "pass", "exact_symbol_comparisons": comparisons,
            "bidirectional_decode_scenarios": decodes, "cases": observations}


def negative_checks(java, rust):
    def oti(length=1280, size=128, blocks=1, alignment=1):
        return length.to_bytes(5, "big") + b"\0" + struct.pack(">HBHB", size, blocks, 1, alignment)

    requests = [
        ("zero-T", b"RQI1D" + oti(size=0) + b"\0\0",
         "bounded T", "T, Z, N and Al must be positive"),
        ("Al4-outside-Java-profile", b"RQI1D" + oti(alignment=4) + b"\0\0",
         "shared profile requires N=1, Al=1", None),
        ("K-over-cap", b"RQI1D" + oti(length=65536, size=4) + b"\0\0",
         "bounded block K", "profile source-block work budget exceeded"),
        ("SBN-outside-object", request_decode(oti(), [bytes([1, 0, 0, 0]) + bytes(128)]),
         "SBN outside object", "SBN outside configured source blocks"),
        ("ESI-over-cap", request_decode(oti(), [bytes([0, 0, 16, 0]) + bytes(128)]),
         "bounded ESI", "profile ESI budget exceeded"),
        ("short-symbol", request_decode(oti(), [bytes(4 + 127)]),
         "profile requires one full symbol per packet", "profile packet must contain one full symbol"),
        ("trailing-bytes", b"RQI1D" + oti() + b"\0\0X",
         "trailing request bytes", "trailing interop request bytes"),
        ("packet-count-over-cap", b"RQI1D" + oti() + struct.pack(">H", 1025),
         "too many delivered packets", "interop packet count budget exceeded"),
        ("request-over-512KiB", b"X" * (REQUEST_LIMIT + 1),
         "request exceeds 512 KiB", "interop request budget exceeded"),
    ]
    observations = []
    for name, request, java_reason, rust_reason in requests:
        peers = [(java, 1, "OpenRqPeer: ", java_reason)]
        if rust_reason is not None:
            peers.append((rust, 2, "transport-conformance: ", rust_reason))
        for command, exit_code, prefix, reason in peers:
            result = subprocess.run(command, input=request, capture_output=True, timeout=60)
            require(result.returncode == exit_code and not result.stdout
                    and result.stderr.decode("utf-8") == prefix + reason + "\n",
                    "expected bounded rejection did not occur (crash/infrastructure errors fail): " + name)
        observations.append({"id": name, "java_exit_code": 1, "java_reason": java_reason,
                             "rust_exit_code": 2 if rust_reason is not None else None,
                             "rust_reason": rust_reason})
    return {"java_rejections": len(requests), "rust_shared_profile_rejections": 8,
            "requests": observations,
            "scope": "Process envelope and bounded profile rejections; not exhaustive malformed RFC6330 tests"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-dir", type=Path,
                        default=Path.home() / ".cache" / "retinue-transport-conformance" / "openrq")
    parser.add_argument("--bootstrap", action="store_true", help="retrieve hash-pinned tools outside git")
    parser.add_argument("--write-fixture", type=Path, help="explicit corpus output; default checks committed corpus")
    parser.add_argument("--rust-peer", type=Path, default=SUITE / "target" / "debug" / "transport-conformance",
                        help="built Rust binary to run with --interop-peer (default: suite target/debug)")
    parser.add_argument("--build-rust-peer", action="store_true",
                        help="first cargo build current suite with --locked --offline")
    parser.add_argument("--receipt", type=Path, help="write live qualification evidence")
    arguments = parser.parse_args()
    if arguments.build_rust_peer:
        require(arguments.rust_peer == SUITE / "target" / "debug" / "transport-conformance",
                "--build-rust-peer uses the suite default binary; omit --rust-peer override")
        build = subprocess.run(["cargo", "build", "--locked", "--offline", "--manifest-path",
                                str(SUITE / "Cargo.toml"), "--target-dir", str(SUITE / "target"),
                                "--message-format=json"], check=True, capture_output=True,
                               text=True, timeout=60)
        executables = []
        for line in build.stdout.splitlines():
            message = json.loads(line)
            target = message.get("target", {})
            if (message.get("reason") == "compiler-artifact" and target.get("name") == "transport-conformance"
                    and "bin" in target.get("kind", []) and message.get("executable")
                    and Path(target.get("src_path", "")).resolve() == SUITE / "src" / "main.rs"):
                executables.append(Path(message["executable"]).resolve())
        require(len(executables) == 1, "Cargo did not identify exactly one current process-peer executable")
        # Cargo JSON handles CARGO_BUILD_TARGET/config targets as well as native builds.
        arguments.rust_peer = executables[0]
    require(arguments.rust_peer.is_file(), "infrastructure unavailable: Rust peer binary missing; "
            "run cargo build --locked --manifest-path " + str(SUITE / "Cargo.toml"))
    rust = [str(arguments.rust_peer.resolve()), "--interop-peer"]
    java, generator = prepare_peer(arguments.tools_dir, arguments.bootstrap)
    generator["captured_decoder_direction"] = "OpenRQ decoded actual Rust raptorq 2.0.1 packets"
    generator["rust_encoder_package"] = "raptorq 2.0.1; locked Cargo.lock"
    cases = make_cases(java, rust)
    fixture = {"schema": "transport-conformance.raptorq-interop/v1", "generator": generator,
               "corpus_edition": "2026-10-11",
               "profile": {"name": "openrq-shared-al1-single-symbol", "F_max": 65536,
                           "T_max": 256, "Z_max": 16, "K_per_block_max": 256,
                           "N": 1, "Al": 1, "ESI_max": 4095,
                           "packet_representation": "4-byte RFC6330 payload ID and one complete T-byte symbol",
                           "case_symbol_size": 128},
               "object_recipe": "byte[i]=(i*73+(i>>3)*19+F)%256; authored deterministic data",
               "scope": "Pinned independent implementation vectors for selected objects, ESIs and delivery schedules; "
                        "not full RFC6330 conformance or a probabilistic recovery-rate measurement.",
               "cases": cases}
    corpus = canonical(fixture)
    if arguments.write_fixture:
        arguments.write_fixture.parent.mkdir(parents=True, exist_ok=True)
        arguments.write_fixture.write_bytes(corpus)
    else:
        require(COMMITTED.is_file(), "committed OpenRQ corpus is missing; use explicit --write-fixture")
        require(COMMITTED.read_bytes() == corpus, "regenerated OpenRQ corpus differs from committed fixture")
    report = {"schema": "transport-conformance.raptorq-live-interop/v1",
              "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
              "generator": generator, "corpus_sha256": hashlib.sha256(corpus).hexdigest(),
              "corpus_regeneration": "pass", "profile": fixture["profile"],
              "coverage": {"full_rfc6330_conformance": False, "performance_benchmark": False,
                           "radio_hardware_qualification": False}}
    report["independent_interoperability"] = cross_check(java, rust, cases)
    report["bounded_process_preflight"] = negative_checks(java, rust)
    report["rust_peer_binary_sha256"] = digest(arguments.rust_peer)
    report["rust_peer_build_mode"] = ("rebuilt_current_suite_locked_offline" if arguments.build_rust_peer
                                      else "prebuilt_binary_identity_only")
    if arguments.build_rust_peer:
        report["rust_peer_source_tree_sha256"] = tree_digest(SUITE / "src")
        report["rust_peer_cargo_manifest_sha256"] = digest(SUITE / "Cargo.toml")
        report["rust_peer_cargo_lock_sha256"] = digest(SUITE / "Cargo.lock")
    if arguments.receipt:
        arguments.receipt.parent.mkdir(parents=True, exist_ok=True)
        arguments.receipt.write_bytes(canonical(report))
    print(json.dumps({"corpus_regeneration": "pass", "cases": len(cases),
                      "corpus_sha256": report["corpus_sha256"],
                      "independent_interoperability": report["independent_interoperability"]}, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("OpenRQ qualification failed: " + str(error), file=sys.stderr)
        sys.exit(1)
