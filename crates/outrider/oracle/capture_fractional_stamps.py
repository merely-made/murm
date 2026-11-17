"""Compare fractional stamp math and delivery announces with stock RNS 1.6/LXMF 1.3.

The digest cases inject an exact SHA-256 result into stock's public stamp functions
to exercise both sides of every target boundary. Other cases use actual hashes.
No Reticulum instance, network interface or radio is opened. Output must be new.
"""

from __future__ import annotations

import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import subprocess
from types import SimpleNamespace

import LXMF
import RNS
from RNS import Stamps
from RNS.vendor import umsgpack


COSTS = (-1.0, 0.0, 0.001, 0.5, 1.0, 1.001953124, 1.001953125,
         1.001953126, 1 + 1 / 256, 7.99609375, 8.0, 8.001953124,
         8.001953125, 8.00390625, 8.5, 99.99609375, 100.0, 101.0)
MAX_DIGEST = (1 << 256) - 1


def stock_vectors() -> dict:
    assert (RNS.__version__, LXMF.__version__) == ("1.6.0", "1.3.0")
    costs, digests, announces, propagation = [], {}, [], []
    original_hash = RNS.Identity.full_hash
    for cost in COSTS:
        fp = Stamps.cost_to_fp(cost)
        target = Stamps.target_from_fp(fp)
        costs.append(dict(input=cost, fp=fp, wire=Stamps.cost_to_bytes(cost).hex(),
                          quantized=Stamps.quantize_cost(cost), target=hex(target)))
        for number in (target - 1, target, target + 1):
            if 0 <= number <= MAX_DIGEST:
                blob = number.to_bytes(32, "big")
                try:
                    RNS.Identity.full_hash = staticmethod(lambda data, digest=blob: digest)
                    value = Stamps.stamp_value(b"", b"")
                    valid = Stamps.stamp_valid(b"", cost, b"")
                finally:
                    RNS.Identity.full_hash = original_hash
                key = (fp, blob.hex())
                digests[key] = dict(fp=fp, digest=blob.hex(), valid=valid, value=value)

    for cost in (None, 1, 1.00390625, 7.99609375, 8, 8.5, 99.99609375):
        destination = SimpleNamespace(display_name="Fractional stock", stamp_cost=cost)
        router = SimpleNamespace(delivery_destinations={b"key": destination})
        raw = LXMF.LXMRouter.get_announce_app_data(router, b"key")
        decoded = LXMF.stamp_cost_from_app_data(raw)
        announces.append(dict(input=cost, raw=raw.hex(),
                              fp=None if decoded is None else Stamps.cost_to_fp(decoded)))
    # A numeric fourth field takes precedence; an invalid field falls back to legacy.
    for fractional in (None, "bad", True, 8.00390625):
        raw = umsgpack.packb([b"Fractional stock", 12, [], fractional])
        decoded = LXMF.stamp_cost_from_app_data(raw)
        announces.append(dict(input=fractional, raw=raw.hex(),
                              fp=None if decoded is None else Stamps.cost_to_fp(decoded)))

    for requested in ((8, 3, 12), (8.5, 1.25, 12.75),
                      (8.00390625, 3.99609375, 9.5), (8.001, 0.002, 12),
                      (8.002, 0.001, 12)):
        p, f, peer = requested
        router = SimpleNamespace(propagation_stamp_cost=p,
                                 propagation_stamp_cost_flexibility=f, peering_cost=peer,
                                 propagation_node=True, from_static_only=False,
                                 propagation_per_transfer_limit=256.5,
                                 propagation_per_sync_limit=10240.25,
                                 get_propagation_node_announce_metadata=lambda: {})
        raw = LXMF.LXMRouter.get_propagation_node_app_data(router)
        assert LXMF.pn_announce_data_is_valid(raw)
        propagation.append(dict(input=requested, raw=raw.hex(), valid=True,
                                costs_fp=[Stamps.cost_to_fp(cost) for cost in requested],
                                floor_fp=Stamps.cost_to_fp(max(0, p - f))))
    for legacy in ([], ["bad", 1, 2]):
        raw = umsgpack.packb([False, 1, True, 256.5, 10240.25, legacy, {}, [8.5, 1.25, 12.75]])
        assert not LXMF.pn_announce_data_is_valid(raw)
        propagation.append(dict(raw=raw.hex(), valid=False))

    stamp_checks = []
    material = b"retinue-fractional-stamp-oracle"
    for nonce in range(16):
        stamp = nonce.to_bytes(32, "big")
        digest = sha256(material + stamp).digest()
        value = Stamps.stamp_value(material, stamp)
        for cost in (0.0, 0.5, 1.0, 1.00390625, 8.0, 8.5):
            stamp_checks.append(dict(fp=Stamps.cost_to_fp(cost), material=material.hex(),
                                     stamp=stamp.hex(), digest=digest.hex(), value=value,
                                     valid=Stamps.stamp_valid(stamp, cost, material)))
    rejected = []
    for label, cost in (("nan", math.nan), ("inf", math.inf), ("-inf", -math.inf)):
        try:
            Stamps.cost_to_fp(cost)
        except (ValueError, OverflowError) as error:
            rejected.append(dict(input=label, exception=type(error).__name__))
        else:
            raise AssertionError(f"stock accepted {label}")
    return dict(producer=dict(rns=RNS.__version__, lxmf=LXMF.__version__), costs=costs,
                digests=list(digests.values()), stamp_checks=stamp_checks,
                delivery_announces=announces, propagation_announces=propagation,
                nonfinite=rejected)


def compare(example: Path, vectors: dict) -> int:
    commands, expected = [], []
    for entry in vectors["costs"]:
        commands.append(f"COST {entry['input']}")
        expected.append(f"COST {entry['fp']}")
    for entry in vectors["nonfinite"]:
        commands.append(f"COST {entry['input']}")
        expected.append("INVALID")
    for entry in vectors["digests"] + vectors["stamp_checks"]:
        commands.append(f"DIGEST {entry['fp']} {entry['digest']}")
        expected.append((entry["valid"], entry["value"]))
    for entry in vectors["delivery_announces"]:
        commands.append(f"ANNOUNCE {entry['raw']}")
        expected.append("NONE" if entry["fp"] is None else f"COST {entry['fp']}")
    for entry in vectors["propagation_announces"]:
        commands.append(f"PROPAGATION {entry['raw']}")
        expected.append("COSTS " + " ".join(map(str, entry["costs_fp"]))
                        + f" FLOOR {entry['floor_fp']}" if entry["valid"] else "INVALID")
    result = subprocess.run([str(example.resolve())], input="\n".join(commands) + "\n",
                            text=True, capture_output=True, timeout=30, check=True)
    rows = result.stdout.splitlines()
    assert len(rows) == len(expected), (len(rows), len(expected), result.stderr)
    for command, wanted, got in zip(commands, expected, rows):
        if isinstance(wanted, tuple):
            fields = got.split()
            actual = (fields[1] == "true", float(fields[3]))
            assert actual == wanted, (command, wanted, got)
        else:
            assert got == wanted, (command, wanted, got)
    return len(commands)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--example", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    vectors = stock_vectors()
    (args.output / "stock-vectors.json").write_text(json.dumps(vectors, indent=2) + "\n")
    count = compare(args.example, vectors) if args.example else 0
    result = dict(compared=count, rns=RNS.__version__, lxmf=LXMF.__version__)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
