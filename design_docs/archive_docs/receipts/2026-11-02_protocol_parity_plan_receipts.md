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

## 2026-11-14 — D4 staging run and removal

Staged at `e5ff193e7b7f5a16c26e2eb74820ce1b67dbe7a1` (clean tree) on this host:

```text
pwsh -File apps/signalman-desktop/tools/stage-firmware.ps1 -Index firmware/packages/index.toml `
  -Destination STAGE/firmware -LinkboyBinary linkboy.exe -Espflash espflash.exe -IgnoreRustVersion
```

espflash 4.5.0 (cargo-installed, not the release binary the assemble scripts admit).

| staged payload | bytes | sha256 | write_bytes | write_ranges |
|---|---|---|---|---|
| `heltec-v4-phy/tulle-heltec-v4-phy` | 7,053,280 | `c2c7bea1ba0ff7b1ae3424707f32f8dc0e0582fefda2380905e4fc1afe7b2813` | 475,104 | 0x0..0x3f0000 |
| `t114-phy/tulle-t114-phy-current.uf2` | 740,352 | `7c6748bcd2f37adca284bfa41304e755d7d34c3d8d1b6c11baf95b4348536674` | 370,176 | 0x26000..0x80600 |

Both manifests say `version = "0.0.1-e5ff193"` and `build=e5ff193e7b7f5a16c26e2eb74820ce1b67dbe7a1`; the Meshtastic package was copied unchanged. `linkboy catalog` verified the staged index (3 packages) and `linkboy inspect` both staged manifests. With one bit flipped (V4 byte 0x2000, T114 byte 0x1028), `inspect` refused each with a hash mismatch and `catalog` refused the index. `linkboy inspect` on `heltec-v4-current.template.toml` refused it as invalid package TOML.

Tests: `cargo test -p linkboy` 84 before, 95 after (11 new); Signalman Desktop 86 before, 86 after with no built image in the tree; `cargo test -p signalman --lib firmware` 3 of 3.

Removed files (bytes, path):

```text
6933912	firmware/heltec-v4-phy/tulle-heltec-v4-phy
2030	firmware/packages/t114-v51.toml
623104	firmware/t114-phy/tulle-t114-phy-current.uf2
36354	firmware/t114-phy/tulle-t114-phy-v10.bin
37230	firmware/t114-phy/tulle-t114-phy-v10.zip
66274	firmware/t114-phy/tulle-t114-phy-v11.bin
67150	firmware/t114-phy/tulle-t114-phy-v11.zip
66242	firmware/t114-phy/tulle-t114-phy-v12.bin
67118	firmware/t114-phy/tulle-t114-phy-v12.zip
73730	firmware/t114-phy/tulle-t114-phy-v13.bin
74606	firmware/t114-phy/tulle-t114-phy-v13.zip
73506	firmware/t114-phy/tulle-t114-phy-v14.bin
74382	firmware/t114-phy/tulle-t114-phy-v14.zip
73666	firmware/t114-phy/tulle-t114-phy-v15.bin
74542	firmware/t114-phy/tulle-t114-phy-v15.zip
75266	firmware/t114-phy/tulle-t114-phy-v16.bin
76142	firmware/t114-phy/tulle-t114-phy-v16.zip
75650	firmware/t114-phy/tulle-t114-phy-v17.bin
76526	firmware/t114-phy/tulle-t114-phy-v17.zip
79746	firmware/t114-phy/tulle-t114-phy-v18.bin
80622	firmware/t114-phy/tulle-t114-phy-v18.zip
79714	firmware/t114-phy/tulle-t114-phy-v19.bin
80590	firmware/t114-phy/tulle-t114-phy-v19.zip
26658	firmware/t114-phy/tulle-t114-phy-v2.bin
27536	firmware/t114-phy/tulle-t114-phy-v2.zip
79618	firmware/t114-phy/tulle-t114-phy-v20.bin
80494	firmware/t114-phy/tulle-t114-phy-v20.zip
156994	firmware/t114-phy/tulle-t114-phy-v21.bin
157870	firmware/t114-phy/tulle-t114-phy-v21.zip
155906	firmware/t114-phy/tulle-t114-phy-v22.bin
156781	firmware/t114-phy/tulle-t114-phy-v22.zip
211938	firmware/t114-phy/tulle-t114-phy-v23.bin
212814	firmware/t114-phy/tulle-t114-phy-v23.zip
210306	firmware/t114-phy/tulle-t114-phy-v24.bin
211182	firmware/t114-phy/tulle-t114-phy-v24.zip
213474	firmware/t114-phy/tulle-t114-phy-v25.bin
214350	firmware/t114-phy/tulle-t114-phy-v25.zip
214338	firmware/t114-phy/tulle-t114-phy-v26.bin
215214	firmware/t114-phy/tulle-t114-phy-v26.zip
214658	firmware/t114-phy/tulle-t114-phy-v27.bin
215534	firmware/t114-phy/tulle-t114-phy-v27.zip
218562	firmware/t114-phy/tulle-t114-phy-v28.bin
219438	firmware/t114-phy/tulle-t114-phy-v28.zip
220130	firmware/t114-phy/tulle-t114-phy-v29.bin
221006	firmware/t114-phy/tulle-t114-phy-v29.zip
36994	firmware/t114-phy/tulle-t114-phy-v3.bin
74240	firmware/t114-phy/tulle-t114-phy-v3.uf2
37863	firmware/t114-phy/tulle-t114-phy-v3.zip
220290	firmware/t114-phy/tulle-t114-phy-v30.bin
221166	firmware/t114-phy/tulle-t114-phy-v30.zip
225250	firmware/t114-phy/tulle-t114-phy-v31.bin
226126	firmware/t114-phy/tulle-t114-phy-v31.zip
228066	firmware/t114-phy/tulle-t114-phy-v32.bin
228942	firmware/t114-phy/tulle-t114-phy-v32.zip
255234	firmware/t114-phy/tulle-t114-phy-v33.bin
256110	firmware/t114-phy/tulle-t114-phy-v33.zip
257986	firmware/t114-phy/tulle-t114-phy-v34.bin
258862	firmware/t114-phy/tulle-t114-phy-v34.zip
258018	firmware/t114-phy/tulle-t114-phy-v35.bin
258894	firmware/t114-phy/tulle-t114-phy-v35.zip
258018	firmware/t114-phy/tulle-t114-phy-v36.bin
258894	firmware/t114-phy/tulle-t114-phy-v36.zip
258018	firmware/t114-phy/tulle-t114-phy-v37.bin
258894	firmware/t114-phy/tulle-t114-phy-v37.zip
257826	firmware/t114-phy/tulle-t114-phy-v38.bin
258702	firmware/t114-phy/tulle-t114-phy-v38.zip
257954	firmware/t114-phy/tulle-t114-phy-v39.bin
258828	firmware/t114-phy/tulle-t114-phy-v39.zip
38306	firmware/t114-phy/tulle-t114-phy-v4.bin
76800	firmware/t114-phy/tulle-t114-phy-v4.uf2
257922	firmware/t114-phy/tulle-t114-phy-v40.bin
258797	firmware/t114-phy/tulle-t114-phy-v40.zip
262338	firmware/t114-phy/tulle-t114-phy-v41.bin
263214	firmware/t114-phy/tulle-t114-phy-v41.zip
262018	firmware/t114-phy/tulle-t114-phy-v42.bin
262894	firmware/t114-phy/tulle-t114-phy-v42.zip
262018	firmware/t114-phy/tulle-t114-phy-v43.bin
262894	firmware/t114-phy/tulle-t114-phy-v43.zip
262050	firmware/t114-phy/tulle-t114-phy-v44.bin
262926	firmware/t114-phy/tulle-t114-phy-v44.zip
268130	firmware/t114-phy/tulle-t114-phy-v45.bin
269005	firmware/t114-phy/tulle-t114-phy-v45.zip
270562	firmware/t114-phy/tulle-t114-phy-v46.bin
271438	firmware/t114-phy/tulle-t114-phy-v46.zip
270818	firmware/t114-phy/tulle-t114-phy-v47.bin
275234	firmware/t114-phy/tulle-t114-phy-v48.bin
276109	firmware/t114-phy/tulle-t114-phy-v48.zip
275234	firmware/t114-phy/tulle-t114-phy-v49.bin
276110	firmware/t114-phy/tulle-t114-phy-v49.zip
38338	firmware/t114-phy/tulle-t114-phy-v5.bin
76800	firmware/t114-phy/tulle-t114-phy-v5.uf2
275138	firmware/t114-phy/tulle-t114-phy-v50.bin
276014	firmware/t114-phy/tulle-t114-phy-v50.zip
275394	firmware/t114-phy/tulle-t114-phy-v51.bin
550912	firmware/t114-phy/tulle-t114-phy-v51.uf2
276270	firmware/t114-phy/tulle-t114-phy-v51.zip
282306	firmware/t114-phy/tulle-t114-phy-v56.bin
283182	firmware/t114-phy/tulle-t114-phy-v56.zip
38338	firmware/t114-phy/tulle-t114-phy-v6.bin
39207	firmware/t114-phy/tulle-t114-phy-v6.zip
36322	firmware/t114-phy/tulle-t114-phy-v7.bin
37191	firmware/t114-phy/tulle-t114-phy-v7.zip
26178	firmware/t114-phy/tulle-t114-phy.bin
27038	firmware/t114-phy/tulle-t114-phy.zip
```

Total: 104 files, 25,657,523 bytes.
