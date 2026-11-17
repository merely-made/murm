"""Capture stock RNS 1.6 stamped announces, path requests and embedded-key values.

Only an isolated, interface-free Reticulum instance is created. Retain public wire
bytes and public ephemeral keys, never the generated private key material.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import tempfile

import RNS
from RNS import Stamps


def capture(root: Path) -> dict:
    assert RNS.__version__ == "1.6.0", RNS.__version__
    (root / "config").write_text(
        "[reticulum]\nshare_instance=No\nenable_transport=No\n"
        "[logging]\nloglevel=0\n[stamps]\nstamp_destinations=Yes\n"
        "destination_cost=3.5\ndestination_value=3.5\npr_cost=2.5\npr_value=2.5\n"
        "lr_cost=3.5\n[interfaces]\n", encoding="ascii")
    RNS.Reticulum(configdir=str(root))
    identity = RNS.Identity.from_bytes(bytes([0x39]) * 64)
    announces = []
    for ratcheted in (False, True):
        destination = RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE,
                                      "retinue", f"network-vector-{int(ratcheted)}")
        destination.set_link_cost(3.5)
        if ratcheted:
            destination.enable_ratchets(str(root / "vector.ratchets"))
        for app in (None, b"app\xc1inside"):
            packet = destination.announce(app_data=app, send=False)
            packet.pack()
            valid = RNS.Identity.validate_announce(packet, required_value=3.5)
            assert valid
            announces.append(dict(raw=packet.raw.hex(), payload=packet.data.hex(),
                                  destination=destination.hash.hex(),
                                  name_hash=destination.name_hash.hex(),
                                  public_key=identity.get_public_key().hex(),
                                  app=None if app is None else app.hex(), ratcheted=ratcheted,
                                  ratchet=RNS.Identity.get_ratchet(destination.hash).hex()
                                  if ratcheted else None,
                                  link_cost=RNS.Identity.recall_link_cost(destination.hash),
                                  stamp_value=RNS.Identity.recall_stamp_value(destination.hash)))
    target, tag = bytes([0x2A]) * 16, bytes([0x3B]) * 16
    got_tag, extension = RNS.Transport.stamp_path_request(target, tag=tag, return_payload=True)
    assert got_tag == tag
    body = target + RNS.Transport.identity.hash + tag + extension
    parsed = RNS.Transport.parse_path_request(body)
    assert RNS.Transport.validate_pr_stamp(extension[2:], target, tag, required_value=2.5)
    assert parsed == (target, RNS.Transport.identity.hash, tag, extension)
    peer_sig_public = bytes([0x4C]) * 32
    material = target + peer_sig_public
    pair, value = Stamps.generate_stamped_key(material, 3.5)
    assert pair is not None and Stamps.stamp_valid(pair[1], 3.5, material)
    return dict(producer=dict(rns=RNS.__version__), announces=announces,
                path_request=dict(target=target.hex(), tag=tag.hex(), body=body.hex(),
                                  extension=extension.hex(), cost=2.5),
                link_request=dict(destination=target.hex(), sig_public=peer_sig_public.hex(),
                                  public=pair[1].hex(), material=material.hex(), value=value,
                                  required_cost=3.5))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    root = Path(tempfile.mkdtemp(prefix="retinue-network-vectors-"))
    try:
        result = capture(root)
        (args.output / "stock-vectors.json").write_text(json.dumps(result, indent=2) + "\n")
        print(f"Captured {len(result['announces'])} announces, 1 path request, 1 public link stamp")
        return 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
