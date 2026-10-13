#!/usr/bin/env python3
# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

"""One bounded design experiment against the persistent retained peer.

This exercises host process state and actual codec output. It starts no radio,
network service or application store, and is not a conformance campaign.
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time

SCHEMA = "retinue.retained-peer/v1"
LINE_CAP = 65536
TOTAL_CAP = 4 * 1024 * 1024


def require(condition, detail):
    if not condition:
        raise RuntimeError(detail)


class Peer:
    def __init__(self, executable):
        self.process = subprocess.Popen(
            [str(executable), "--retained-peer"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self.started = time.monotonic()
        self.deadline = self.started + 50
        self.request_id = 0
        self.now = 0
        self.input_octets = 0
        self.output_octets = 0
        self.buffer = bytearray()
        self.transcript = []
        os.set_blocking(self.process.stdin.fileno(), False)
        os.set_blocking(self.process.stdout.fileno(), False)
        self.hello = self.read_line()
        require(self.hello.get("kind") == "hello", "missing peer hello")

    def ready(self, file, event):
        timeout = min(12, self.deadline - time.monotonic())
        require(timeout > 0, "experiment lifetime exhausted")
        with selectors.DefaultSelector() as selector:
            selector.register(file, event)
            require(selector.select(timeout), "peer I/O deadline expired")

    def read_line(self):
        while b"\n" not in self.buffer:
            self.ready(self.process.stdout, selectors.EVENT_READ)
            chunk = os.read(self.process.stdout.fileno(), 4096)
            require(chunk, "peer closed before a complete response")
            self.buffer.extend(chunk)
            require(len(self.buffer) <= LINE_CAP, "peer response line too large")
        end = self.buffer.index(b"\n") + 1
        line = bytes(self.buffer[:end])
        del self.buffer[:end]
        self.output_octets += len(line)
        require(self.output_octets <= TOTAL_CAP, "peer output budget exceeded")
        return json.loads(line)

    def command(self, op, *, now=None, expected="ok", **fields):
        if now is None:
            now = self.now
        self.request_id += 1
        request = {"schema": SCHEMA, "request_id": self.request_id,
                   "now_slot": now, "command": {"op": op, **fields}}
        line = (json.dumps(request, separators=(",", ":")) + "\n").encode()
        require(len(line) <= LINE_CAP, "experiment request line too large")
        self.input_octets += len(line)
        require(self.input_octets <= TOTAL_CAP, "experiment input budget exceeded")
        written = 0
        while written < len(line):
            self.ready(self.process.stdin, selectors.EVENT_WRITE)
            written += os.write(self.process.stdin.fileno(), line[written:])
        reply = self.read_line()
        require(reply.get("request_id") == self.request_id, "response ID mismatch")
        require(reply.get("schema") == SCHEMA, "response schema mismatch")
        require(reply.get("implementation") == self.hello["implementation"],
                "implementation identity changed during process")
        require(reply.get("status") == expected, f"unexpected {op} result: {reply}")
        self.now = reply["now_slot"] if reply.get("now_slot") is not None else self.now
        observation = {key: value for key, value in reply.items()
                       if key not in ("schema", "kind", "implementation")}
        self.transcript.append({"op": op, "response": observation})
        return reply

    def finish(self):
        self.process.stdin.close()
        status = self.process.wait(timeout=max(0.1, self.deadline - time.monotonic()))
        detail = self.process.stderr.read(4097)
        require(len(detail) <= 4096, "peer stderr bound exceeded")
        require(status == 0, f"peer exit {status}: {detail.decode(errors='replace')}")
        return {"exit_status": status, "requests": self.request_id,
                "input_octets": self.input_octets, "output_octets": self.output_octets,
                "elapsed_seconds": round(time.monotonic() - self.started, 6)}


def publish(peer, generation, original, expiry=200):
    return peer.command("publish", generation=generation,
                        original_bytes_hex=original.hex(), expires_at=expiry)


def prepared(peer, channel, generation, now=None):
    return peer.command("prepare", now=now, channel=channel,
                        requested_generation=generation)["result"]["prepared"]


def accept(peer, frame):
    return peer.command("settle", token=frame["token"], decision="accept")


def experiment(peer):
    peer.command("init", public_fixture=True, namespace_hex="71" * 32,
                 authority_seed_hex="29" * 32, channels=4,
                 manifest_interval=5, repair_floor=1)
    a = bytes((index * 17 + 3) % 256 for index in range(1015))
    b = bytes((index * 29 + 7) % 256 for index in range(1281))
    metadata_a = publish(peer, 1, a)["result"]["metadata"]
    before = peer.command("snapshot")
    frame = prepared(peer, 0, 1, now=1)
    peer.command("settle", token=frame["token"], decision="defer")
    repeated = prepared(peer, 0, 1)
    require(frame == repeated, "deferral changed prepared bytes, ESI or token")
    held = peer.command("snapshot")
    require(held["producer_stats"]["queued_frames"] ==
            before["producer_stats"]["queued_frames"], "defer advanced queued cursor")
    blocked = peer.command("prepare", channel=1, requested_generation=1,
                           expected="refused")
    require(blocked["refusal"]["code"] == "pending_frame", "missing sink backpressure")
    accepted = accept(peer, frame)
    require(accepted["producer_stats"]["queued_frames"] == 1, "accept accounting mismatch")
    stale = peer.command("settle", token=frame["token"], decision="accept",
                         expected="refused")
    require(stale["refusal"]["code"] == "stale_token", "double settlement accepted")
    tampered = bytearray.fromhex(frame["frame_bytes_hex"])
    tampered[-1] ^= 1
    refused = peer.command("receive", frame_bytes_hex=tampered.hex(), expected="refused")
    require(refused["refusal"]["code"] == "invalid_auth", "tampered signature accepted")
    require(refused["collector_stats"]["decoder_allocations"] == 0 and
            refused["collector_stats"]["decode_work"] == 0, "unauthenticated codec work")
    peer.command("snapshot", now=2)
    metadata_b = publish(peer, 2, b)["result"]["metadata"]
    require(peer.command("lookup_metadata", generation=1)["result"]["metadata"] ==
            metadata_a, "publishing B changed retained A")
    recovered = {}
    deliveries = {1: {"manifest": 0, "fresh": 0, "repair": 0},
                  2: {"manifest": 0, "fresh": 0, "repair": 0}}
    dropped_a_fresh = 0
    # A's late receiver takes only channel 0 from slot 10 onward. B's fast
    # receiver takes channels 1..3. Both are host opportunities, not radios.
    for slot in range(2, 80):
        for channel in range(4):
            generation = 1 if channel == 0 else 2
            item = prepared(peer, channel, generation, now=slot)
            if item is None:
                continue
            accept(peer, item)
            if generation == 1 and item["lane"] == "fresh":
                dropped_a_fresh += 1
            deliver = (generation == 2 and generation not in recovered) or (
                generation == 1 and slot >= 10 and item["lane"] != "fresh"
                and generation not in recovered
            )
            if not deliver:
                continue
            delivered = peer.command("receive", frame_bytes_hex=item["frame_bytes_hex"])
            deliveries[generation][item["lane"]] += 1
            if delivered["result"]["outcome"] == "completed":
                output = delivered["result"]["recovered"]
                require(output["actual_decoder_output"], "non-decoder recovery")
                require(bytes.fromhex(output["original_bytes_hex"]) ==
                        (a if generation == 1 else b), "recovered opaque bytes changed")
                recovered[generation] = {"slot": slot, "channels": [0] if generation == 1
                                         else [1, 2, 3], "metadata": output["metadata"]}
        if len(recovered) == 2:
            break
    require(len(recovered) == 2, "finite workload did not recover both objects")
    require(deliveries[1]["fresh"] == 0 and deliveries[1]["repair"] > 0,
            "late A reconstruction was not repair-only")
    cached_a = peer.command("recovered", generation=1)["result"]
    require(bytes.fromhex(cached_a["original_bytes_hex"]) == a, "retained decoder output changed")
    publish(peer, 3, bytes([0x33]) * 16384)
    full_snapshot = publish(peer, 4, bytes([0x44]) * 16384)
    full = peer.command("publish", generation=5,
        original_bytes_hex="55", expires_at=200, expected="refused")
    require(full["refusal"]["code"] == "full" and
            full["refusal"]["retry_at_slot"] == 200, "capacity refusal/retry absent")
    require(full["producer_stats"]["encoder_allocations"] ==
            full_snapshot["producer_stats"]["encoder_allocations"], "full allocated encoder")
    pending = prepared(peer, 0, 1, now=peer.now + 1)
    require(pending is not None, "missing expirable prepared frame")
    expired = peer.command("snapshot", now=200)
    require(expired["expiry"]["producer"]["invalidated_token"] == pending["token"],
            "expiry did not report invalidated token")
    require(expired["result"]["inventory"] == [] and
            expired["result"]["collector_inventory"] == [], "expiry retained object state")
    expired_token = peer.command("settle", token=pending["token"], decision="accept",
                                 expected="refused")
    require(expired_token["refusal"]["code"] == "stale_token", "expired token accepted")
    c = bytes((index * 31 + 1) % 256 for index in range(513))
    publish(peer, 5, c, expiry=300)
    helper = peer.command("recover", generation=5)["result"]
    require(bytes.fromhex(helper["recovered"]["original_bytes_hex"]) == c and
            helper["recovered"]["actual_decoder_output"], "helper did not recover actual bytes")
    require(helper["delivered_repair_frames"] > 0, "helper did no repair decoding")
    regression = peer.command("snapshot", now=peer.now - 1, expected="refused")
    require(regression["refusal"]["code"] == "clock_regression", "backward time accepted")
    return {"objects": {"A": metadata_a, "B": metadata_b},
            "publication_b_slot": 2, "late_a_join_slot": 10,
            "recovered": recovered, "delivered_frames": deliveries,
            "dropped_a_fresh_frames": dropped_a_fresh,
            "same_prepared_frame_after_defer": True, "single_accept_accounting": True,
            "stale_and_expired_tokens_refused": True,
            "invalid_signature_before_decoder_work": True,
            "fifth_object_refused_before_encoder": True,
            "expiry_releases_capacity": True, "backward_time_refused": True,
            "helper": helper}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), "output already exists")
    executable = args.peer.resolve(strict=True)
    peer = Peer(executable)
    try:
        observations = experiment(peer)
        process = peer.finish()
    finally:
        if peer.process.poll() is None:
            peer.process.kill()
            peer.process.wait(timeout=5)
    report = {"schema": "retinue.retained-live-exploration/v1",
              "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "implementation": peer.hello["implementation"], "limits": peer.hello["limits"],
              "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "process": process, "observations": observations,
              "scope": {"persistent_external_process": True, "actual_raptorq": True,
                        "opaque_exact_bytes": True, "application_admission": False,
                        "sockets": False, "physical_rf": False, "restart_persistence": False,
                        "independent_codec_interoperability": False,
                        "schedule": "owner-selected generation per channel",
                        "collector": "one host collector with separate per-object reception roles",
                        "time": "caller-driven logical slots; channels are host opportunities"},
              "transcript": peer.transcript}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        json.dump(report, output, indent=2, sort_keys=True)
        output.write("\n")
    print(json.dumps({"output": str(args.output), "process": process,
                      "recovered": observations["recovered"]}, sort_keys=True))


if __name__ == "__main__":
    main()
