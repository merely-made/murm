# Protocol and firmware feature parity: receipts

## 2026-11-14 — D4 build reproducibility

Source `19fa737252e84db0d05a2f8bbe32e66bbe1bd5ef`, Windows host, one build
directory per run cleared before each "clean" build (both the target directory
and the workspace `target/build` build-dir). `RETINUE_FIRMWARE_REVISION` set to
that commit for every build.

Tracked images (pinned at `711816a`):

| file | bytes | sha256 |
|---|---|---|
| `firmware/heltec-v4-phy/tulle-heltec-v4-phy` | 6,933,912 | `6585c03d82880a2c61c3e4d47b5e93fe5d4752ba08b1c4b1de87eb1a0e71fca3` |
| `firmware/t114-phy/tulle-t114-phy-current.uf2` | 623,104 | `27424c8e634c10fe84a85823990d63ea8c11de23cd008e3ae877896984e6cfc9` |
| `firmware/t114-phy/tulle-t114-phy-v51.uf2` | 550,912 | `3b802471f5402f38cf4ab30c39d9acb9a9e893aaf2a588455536b57706452f1b` |

Each equals its manifest's `byte_length` and `sha256`.

V4: `cargo +esp build -p tulle-heltec-v4-phy --release --target xtensa-esp32s3-none-elf -Zbuild-std=core`
fails without `--ignore-rust-version` (esp 1.97.0-nightly; radio-face,
radio-hand, retinue, selvage, seneschal and the firmware declare 1.97.1).

| run | bytes | sha256 |
|---|---|---|
| clean A | 7,053,276 | `e4f83faea5088edcf82e564689356bb5125da615f46f6bf95c594d73fec3febd` |
| clean B | 7,053,276 | `b4e91711a20619971949622b52c1d030a8cafa48f2688a510f571eb27383853f` |
| clean C, `SOURCE_DATE_EPOCH=1794647750` | 7,053,276 | `a54e5cf608aec4b44646f29e07a1d617fc9249395ce7d287b05a643ba5a986b7` |
| clean D, same epoch | 7,053,276 | `a54e5cf608aec4b44646f29e07a1d617fc9249395ce7d287b05a643ba5a986b7` |

A and B differ in 3 bytes at file offset 0x1074-0x1077, inside
`.flash.appdesc`: build times 15:23:50 and 15:26:46 (the tracked image says
21:12:39 2026-09-30). `esp-bootloader-esp-idf` 0.5.0's build script takes
`SOURCE_DATE_EPOCH`. The ELF holds the checkout directory name once and the
user directory 215 times (debug info). `espflash save-image --chip esp32s3`
gives 402,880 application bytes for the tracked ELF (manifest `write_bytes`
468,416 = 402,880 + 0x10000) and 409,568 for run C.

T114: `cargo build -p tulle-t114-phy --release --target thumbv7em-none-eabihf`,
`rust-objcopy -O binary`, `linkboy make-uf2 BIN UF2 0x26000 0xADA52840`.

| run | ELF sha256 | BIN bytes / sha256 | UF2 bytes / sha256 |
|---|---|---|---|
| clean 1 | `26e863569f99d453295a082cb186fc6f2cac3fa40e612a845b7d352b479c7bcb` | 370,018 / `42e22714b147ec0dd753cd88108ad62ffe826692e072faae02c311e6e520ce30` | 740,352 / `f9268710cab211012b58e360589aba3a89f4e4b37f8143198f986d6754179765` |
| clean 2 | identical | identical | identical |

The UF2 carries 1,446 blocks, 370,176 payload bytes, spanning
`0x26000..0x80600`. The binary holds the user directory 30 times (cargo
registry panic locations).

Catalog dependency: with the V4 image moved aside,
`cargo test --manifest-path apps/signalman-desktop/Cargo.toml` passed 71 and
failed 15 (13 owner-flow, 2 accessibility); with it restored, 86 of 86.
`cargo test -p linkboy` passed 84 at baseline.
