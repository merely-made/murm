# Retinue firmware packages

This directory contains the public package index, recovery instructions, and package
templates. Built images are not kept in the repository (protocol_parity D4): a template
(`*.template.toml`) carries a package's identity, devices, helpers, preserved ranges and
recovery instructions, and the staging step below builds the image and writes the full
manifest beside it. Linkboy refuses a template as a package. The exception is the
serial-DFU recovery package `t114-v47.toml` with
`../t114-phy/tulle-t114-phy-v47.zip`, kept because a recovery image must exist before
anything can be built. The index is the publishable installer evidence artifact:
`retinue.heltec-v4` is the September 30 modem build, with fresh Windows installation
and ROM-loader retry evidence. The partial `retinue.heltec-v4-resident` commissioning
image was withdrawn on 2026-11-14: foreign LoRa meshes are reached through a stock
companion radio instead. T114 offered the installed native `711816a` image as
`partial`, pending a recovery retry of those same bytes. The `711816a` images and the
historical v51 artifacts left the tree on 2026-11-14 and remain in git history; receipts
for them qualify those bytes only, not a later staged build. The retained
Meshtastic T114 package remains `partial` until its own interface check is recorded.
Vendor releases are listed as [qualified downloads](#qualified-vendor-releases), not tracked
files (protocol_parity D5): the Meshtastic T114 entry pins the vendor's UF2 by URL, length
and sha256, and its file left the tree on 2026-11-14.

The V4 images identify source `711816a8124bd58cd8cc314ec4de5009be7df166` and their
image type in live status. Both installed V4s retain their original settings.
The resident image starts in the persisted modem personality; one-shot resident
setup uses volatile test channel material. Three Sennet and Tucket visits passed,
but a final home link proof after cancellation timed out. It does not close the
resident acceptance gate. See the [firmware receipt](https://github.com/merely-made/murm/blob/b0e31de8b581940dcd347968585c401e3ad786ca/testing/receipts/firmware-refresh/README.md).

The [follow-up receipt](https://github.com/merely-made/murm/blob/b0e31de8b581940dcd347968585c401e3ad786ca/testing/receipts/firmware-refresh/follow-up/README.md)
records T114 installation, original settings sequence and identity, native resource
RF checks, guarded package admission and two signed announces advancing across
soft resets. The loader backup omitted the settings pair; unchanged live sequence
and identity are not a byte-identical readback. The isolated resident run passed
one cancellation, then reported Home and LinkUp on a second but timed out waiting
for its expected RF proof. Both current T114 and resident packages remain partial.

## Before installing

Use Linkboy's package path so the manifest, every artifact hash, board facts, flash route, and
recovery instructions are checked together. Installed builds resolve an admitted helper from their
own platform directory and keep that verified executable for the write:

- Heltec WiFi LoRa 32 V4: an official `espflash` 4.5.0 platform release through the ESP32-S3
  ROM loader. The manifest records both upstream archive and extracted executable SHA-256.
- Heltec T114: Linkboy's built-in application-only UF2 writer through the stock `HT-n5262`
  mass-storage bootloader. The retained serial-DFU package remains an expert recovery route,
  outside the public catalog.
- Meshtastic T114: Linkboy's built-in, explicit UF2-volume writer. The official release file,
  downloaded from the vendor on demand, is GPL-3.0 external firmware, not a Retinue
  capability claim. A UF2 volume can
  disappear while Linkboy flushes a complete file: that expected Windows device-removal
  acknowledgement is recorded, but the package still requires an upstream interface check.

Hopspot was retired from both catalogs and stage assembly on September 26.
Its immutable artifacts and manifest remain in `Code/testing/murm/testing/receipts/prns-retirement`
as historical evidence, outside the active firmware offering. Retinue's V4
recovery instructions continue to use its own package and the ROM loader.

Older V4 artifacts have Windows, Intel-macOS, Apple-silicon-macOS, and Linux physical receipts.
The refreshed V4 artifact currently has Windows evidence only; older runs do not
qualify these new bytes on other hosts.
The current T114 UF2 installation has a Windows physical receipt. Its same-image
recovery retry remains open; historical v51 recovery does not qualify the new bytes.

The Phase D package shape is `persistent_state.schema = 1` with the
`native_node_guard` and preserved `0xE8000..0xEC000` reservation. Linkboy refuses to flash a
known armed native-node device with a package that lacks that declaration. The retained v47 and
v51 binaries predate this guard, so they declare the preserved range in their manifests but do
not claim guard support. The current T114 artifact emits the guard and passed
admission while armed; the same armed device refused the old v51 package.
First-flash, unarmed, unknown, and
foreign running states remain eligible for an explicitly compatible package. Legacy and
external packages intentionally omit the declaration.

The guard is based on the running application's `state=node-timebase-v1` status token. A
bootloader-only observation cannot recover the prior application state; callers carrying that
fact across a loader transition must retain it in the serialized device observation. Linkboy
does not invent a hardware read for it.

The 2026-08-31 WN1 software-only check did not update either immutable package. The current
V4 package writes `0x0..0x3F0000` and preserves `0x3F0000..0x400000`, which includes the
settings pair, announce-reservation pair, ordinary control rollback pair
(`0x3F4000..0x3F6000`), and the writable pending first-write A/B pair
(`0x3F6000..0x3F8000`). The current resident build also owns the Sennet packet-ID
reservation pair at `0x3F8000..0x3FA000`. The remaining tail is unallocated
future space; any vault needs its own selected range and overlap guard. September
30 rebuilt the V4 artifacts and independently verified the full preserved tail
before resident setup. T114 v51's immutable payload stops at `0x69400`; its native-node
preserved/guard declaration begins at `0xE8000`, so it does not contractually cover the new
`0xE6000..0xE8000` control pair. The installed current T114 package reserves all
three A/B pairs at `0xE6000..0xEC000` and writes only `0x26000..0x72100`.

## Qualified vendor releases

A stock Meshtastic or MeshCore release is listed by a complete, tracked manifest whose
payload names an https `url` instead of a `path`, with the file's `byte_length` and
`sha256` and a `[qualification]` record of what was checked: where the hash came from and
when, the structure checks it passed, and whether Signalman's stock-companion client was
checked against it on a real board (with a date and receipt) or not. Only listed releases
are offered. [QUALIFYING.md](QUALIFYING.md) is the bench procedure and lists the candidates
that are not yet qualified.

- **Nothing is downloaded until an owner asks to install.** Loading a catalog lists a
  download as on-demand and touches no network; neither do the tests.
- **The pinned hash is what vouches.** Neither vendor signs its files or marks its releases
  immutable, and Meshtastic's per-board host publishes only md5. A file replaced upstream no
  longer matches the pinned `sha256` and is refused.
- **What Linkboy checks.** An https URL only, and no redirect off https; a body no longer
  than the pinned length; then length, sha256 and layout exactly as for a local package
  (`FlashPackage::from_verified_parts`): for a UF2 the family id, block sequence, payload
  size and block span against `write_ranges`; for a V4 merged image the ESP image magic,
  sector-aligned non-overlapping parts inside the flash size and clear of preserved ranges.
  A refused download leaves nothing in the cache.
- **The cache** is `%LOCALAPPDATA%\Merely\Signalman\firmware\<sha256>\<file name>`
  (`SIGNALMAN_FIRMWARE_CACHE` overrides it; Linkboy's commands take `--cache-dir`). A cached
  file is verified again before every use and deleted if it no longer matches.
- **V4 images.** A vendor's merged ESP32-S3 image is a `merged-image` part at offset 0;
  Meshtastic's OTA and filesystem images are `additional-image` parts at the offsets the
  manifest states. `full_erase = true` says the package erases all of flash first; only a
  merged image may say so, with no preserved ranges and `state_impact = "replaced"`, and the
  plan asks the owner to confirm it.

When a radio is connected through Signalman's stock-companion client, the version it reports
is compared with the list: on a listed release (and whether a newer one is listed), on a
version the list does not have (nothing is offered), or unknown. Versions order by their
numeric `MAJOR.MINOR.PATCH`; a build commit never orders. The install itself is the owner
flow's one review and approval, which shows the vendor, version, size, sha256 and source.

Fetching uses ureq 3.5 on rustls with bundled webpki roots. Linkboy's CLI fetches only in
`linkboy qualify`; its other commands read a download from the cache. Signalman Desktop
fetches when the owner reviews a release, on a worker thread.

## Building and staging packages

`apps/signalman-desktop/tools/stage-firmware.ps1` builds every image an index templates from
HEAD and stages a catalog that verifies. It refuses a dirty tree unless given `-AllowDirty`,
and refuses a destination inside the repository. Nothing is downloaded.

```text
pwsh apps/signalman-desktop/tools/stage-firmware.ps1 -Index firmware/packages/index.toml `
  -Destination STAGE/firmware -LinkboyBinary LINKBOY -Espflash ESPFLASH [-IgnoreRustVersion]
```

- Heltec V4: `cargo +esp build -p tulle-heltec-v4-phy --release --target xtensa-esp32s3-none-elf
  -Zbuild-std=core` with `SOURCE_DATE_EPOCH` set to HEAD's commit time; `write_bytes` is the
  application image `espflash save-image` reports plus its `0x10000` offset; the write range
  runs from 0 to the first preserved range. espflash must be 4.5.0, the helper the template
  names. `-IgnoreRustVersion` is for an esp toolchain older than the declared rust-version.
- Heltec T114: `cargo build -p tulle-t114-phy --release --target thumbv7em-none-eabihf`,
  `rust-objcopy -O binary`, then `linkboy make-uf2 BIN UF2 0x26000 0xADA52840`; the write range
  and `write_bytes` come from the UF2 blocks.

Both builds set `RETINUE_FIRMWARE_REVISION` to HEAD. `linkboy stage-catalog` then writes each
manifest with `version` (`<expected version>-<short revision>`), `source_revision`, the
payload's `byte_length`, `sha256` and `write_bytes`, `write_ranges`, and the `build=` check, and
verifies the whole staged catalog before it returns. The assemble scripts beside it call it.

On one machine and checkout, two builds of the same commit are byte-identical. Builds from
another checkout path or machine are not expected to match: both images embed absolute paths.
A staged hash is therefore evidence about that stage, not a value the repository pins.

Inspect a staged catalog and package before connecting a board:

```text
linkboy catalog STAGE/firmware/packages/index.toml
linkboy inspect STAGE/firmware/packages/heltec-v4-current.toml
linkboy inspect STAGE/firmware/packages/t114-current.toml
linkboy inspect STAGE/firmware/packages/meshtastic-t114-2.7.26.54e0d8d.toml --cache-dir CACHE
```

A qualified download inspects only once its file is in the cache (`linkboy qualify URL`
puts it there).

The accepted plan is the only path that may write. Use the board-specific selection when a
loader can prove the processor but cannot name the carrier revision:

```text
linkboy plan PORT STAGE/firmware/packages/heltec-v4-current.toml v4@REVISION
linkboy plan PORT STAGE/firmware/packages/t114-current.toml t114@REVISION
```

Do not identify a board from a COM number, USB identifier, or processor alone. Read the plan's
state-impact and recovery sections before accepting a write.

## Recovery

- [Heltec V4 recovery](heltec-v4-current-recovery.md)
- [T114 recovery](t114-current-recovery.md)
- [Meshtastic T114](#meshtastic-t114), below.

### Meshtastic T114

The Meshtastic package writes only `0x26000..0xD9200`; the S140 SoftDevice below it and the
Adafruit UF2 bootloader above it are not written, so the bootloader survives whatever the
application does. Double-tap the T114's reset button: the `HT-n5262` volume mounts. Select
it explicitly and write the verified package again:

```text
linkboy flash-volume VOLUME firmware/packages/meshtastic-t114-2.7.26.54e0d8d.toml t114@2.x --receipt RECEIPT.json
```

To return to Retinue instead, write a staged `retinue.t114` package the same way (see
[T114 recovery](t114-current-recovery.md)) and record its own receipt. The v51 recovery page
this entry used to name described a removed image and left the tree on 2026-11-14.

The recovery page remains part of each package's public evidence. The index may use
`proven-recipe` only when installer and recovery receipts link to reproducible public runs and
name the host platforms that physically exercised the route.
