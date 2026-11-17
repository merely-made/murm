# Qualifying a vendor release

A stock Meshtastic or MeshCore release is offered only once it is on the list: a complete,
tracked manifest in this directory, named by `index.toml`, whose payload is an https `url`
with the file's `byte_length` and `sha256` and whose `[qualification]` table says what was
checked (protocol_parity D5). This page is the bench procedure. The repository keeps no
vendor binary: the pinned sha256 is what vouches for the file, because neither vendor signs
its files and a file replaced upstream must be refused.

## Steps

1. **Pick the release and the exact file.** Use the vendor's recommended channel
   (Meshtastic "Beta", which its apps call Stable; MeshCore's latest `companion-v*` tag).
   - Meshtastic: the per-board file from `https://release.meshtastic.org/<version>/`, as
     named in that directory's `firmware-<board>-<version>.mt.json`. Note the md5 the
     `.mt.json` gives for the file.
   - MeshCore: the asset from
     `https://github.com/meshcore-dev/MeshCore/releases/download/companion-v<version>/<asset>`.
     Note the `sha256:` digest GitHub shows beside the asset.
   - Name the publisher exactly `Meshtastic` or `MeshCore`: the radio's reported version is
     compared with entries of that publisher.
2. **Download it and draft the entry.**
   ```text
   linkboy qualify URL --out DRAFT.toml
   ```
   fetches the file once over https (or pass `--file PATH` for a copy downloaded by hand).
   It prints the length, sha256 and the layout Linkboy detects, keeps the bytes in the
   firmware cache under their sha256, and writes a draft whose human fields say FILL.
3. **Cross-check the hash against the vendor.** MeshCore: the printed sha256 must equal
   GitHub's digest. Meshtastic: the file's md5 (`certutil -hashfile PATH MD5` or `md5sum`)
   must equal the `.mt.json` md5. Record which in `hash_source`, with the date.
4. **Complete the draft.** Fill identity, version (exactly as the radio will report it, for
   example `2.7.26.54e0d8d` or `1.18.0-81dbae5`), source revision and URLs, the target board
   and revision, preserved ranges, the helper, `manual_check`, the recovery text and the
   licence. A UF2's `write_ranges` come from the draft. For a V4 merged image, confirm the
   offset (the draft assumes 0) and any additional images and their offsets from the
   vendor's own flasher, and set `full_erase = true` only if that flasher erases all of
   flash; then `state_impact = "replaced"` and no preserved ranges. Copy no value from
   memory or a third party's notes.
5. **Check the structure.** `linkboy inspect DRAFT.toml` must load it from the cache and
   print it. Add it to `index.toml` (`state = "partial"`, no receipts) and run
   `linkboy catalog firmware/packages/index.toml`. The structure is now qualified:
   `structure_checks` says which checks passed. `companion_checked = false`.
6. **Flash it on the bench**, with Linkboy's plan and the board's own route:
   `linkboy flash-volume VOLUME DRAFT.toml t114@2.x --receipt RECEIPT.json` for a T114,
   `linkboy flash PORT DRAFT.toml v4@REVISION --receipt RECEIPT.json` for a V4. Complete
   the package's `manual_check` with the vendor's own interface.
7. **Check Signalman's stock-companion client against it.** Connect over USB with
   `cargo run -p signalman --features comms --example stock_companion_pair -- probe
   <meshtastic|meshcore> PORT REALM NEW-OUTPUT-DIR` and observe:
   - the handshake completes, and the version notice equals the entry's `version`
     (`Meshtastic firmware ...`, or `MeshCore firmware ..., companion API N` with N at least 3);
   - for Meshtastic, a capture with `apps/signalman/capture/capture_config.py` diffs cleanly
     against `apps/signalman/capture/STRUCTURE.md`; for MeshCore, `DeviceInfo` decodes as
     `apps/signalman/src/radio_comms/companion/meshcore.rs` pins it;
   - one message each way with a second radio (`stock_companion_pair pair ...`).
8. **Record the receipt.** Append it under a dated heading in
   `design_docs/archive_docs/receipts/2026-11-02_protocol_parity_plan_receipts.md` (commands,
   versions, hashes, what was observed), commit it, and only then set
   `companion_checked = true` with `companion_checked_on` and `companion_receipt` (the
   receipt's permalink). The catalog `state` stays `partial` until installer and recovery
   receipts exist, as for every package.

Whether a newer release replaces the older entry or is listed beside it under its own
`package_id` is open in D5; the tool offers the newest listed release either way, and two
entries with one `MAJOR.MINOR.PATCH` are refused as ambiguous.

## Qualified

| Vendor / board | Version | Hash | Structure | Companion client on this board |
|---|---|---|---|---|
| Meshtastic / T114 | 2.7.26.54e0d8d, `https://release.meshtastic.org/2.7.26.54e0d8d/firmware-heltec-mesh-node-t114-2.7.26.54e0d8d.uf2` | sha256 `e038b106…02b9`, 1,467,392 bytes, taken from the copy this repository tracked until 2026-11-14; its md5 equals the vendor's `.mt.json` md5 (read 2026-11-14) | UF2 layout check passes | No. The capture fixture is Meshtastic 2.7.26 on a V4 |

## Not yet qualified

Facts from the 2026-11-14 assessment (`Code/testing/engine-split/assess_stock_firmware_download.md`).
None is listed; each needs the steps above.

| Vendor / board | Candidate | File | Unknown |
|---|---|---|---|
| MeshCore / T114 | companion 1.18.0-81dbae5 (2026-10-15, companion API 14), the ruled build "USB companion, with display" | `Heltec_t114_companion_radio_usb-v1.18.0-81dbae5.uf2`, about 802 KB, GitHub `sha256:e84fc11319df1901685a3a6de9f64d1d5d2b00a35d817ae46ca157a0bb9131df` | Exact length and UF2 span; whether the span clears the T114's preserved ranges; Signalman's client was exercised at API 13, never 14 |
| MeshCore / V4 | companion 1.18.0-81dbae5 | `heltec_v4_companion_radio_usb-v1.18.0-81dbae5-merged.bin`, about 701 KB, GitHub `sha256:d5b0f7a152755b9bc57b1dae1ed3728aea01893468ae9be1ccfaa02d0e4baafc` | The merged image's flash offset (only a third party's docs say 0x0); whether Linkboy's V4 revision is MeshCore's `heltec_v4` or `heltec_v4_r8`; whether its flasher erases all of flash |
| MeshCore / V4 | companion 1.17.1-d929643 (the build the bench used, API 13) | `heltec_v4_companion_radio_usb-v1.17.1-d929643-merged.bin`, about 691 KB, GitHub `sha256:67ef59c7687787255c23659e7525b81c1fc88bf679521f4183099a50580d0a3d` | As above; superseded by 1.18.0 |
| Meshtastic / V4 | 2.7.26.54e0d8d (the vendor's current stable) | From `https://release.meshtastic.org/2.7.26.54e0d8d/`: `firmware-heltec-v4-2.7.26.54e0d8d.factory.bin` (2,182,944 bytes, at 0x0), the OTA image `mt-esp32s3-ota.bin` (636,544 bytes, app1 at 0x650000) and the littlefs image (3,538,944 bytes, at 0xc90000), 16 MB layout, full erase, md5 only | Exact OTA and littlefs file names and md5s from the `.mt.json`; whether Linkboy's V4 revision is `heltec-v4` or `heltec-v4-r8-oled`; the bench ran 2.7.26 on a V4, but from bytes no manifest pinned |
| Meshtastic / T114 | 2.8.2.d5f1619 | Alpha channel (2026-11-03) | The vendor does not recommend alpha for most users |
