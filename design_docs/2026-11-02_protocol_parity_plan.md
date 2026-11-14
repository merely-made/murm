# Protocol and firmware feature parity

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-16_protocol_parity_plan.md`; the original is archived whole.

## Open phases and done conditions

- **Parity matrix:** reconcile current Retinue interfaces/shared-instance and management coverage against the review; inventory pinned reference releases and each endpoint, relay/server, management and optional product feature, with owner, applicability and software/physical/firmware status.
- **Tucket:** *Withdrawn 2026-11-14 ([protocol_parity D1](#protocol_parity-d1--foreign-meshes-through-stock-radios)): MeshCore is reached through a stock companion radio; native Tucket roles are no longer built.* Extend from bounded endpoint/contact-restore profiles to selected multiple-hop/scoped routes, companion/management and current-release differences; qualify repeater, room-server persistence and sensor roles separately against stock peers, including restart, retry, failure and bounded state on the named image.
- **Retinue:** complete selected host interface, service and platform cases and bounded embedded transport/resource/restart/hostile-traffic cases. Compare against the named RNS reference and keep host, embedded node and modem roles distinct.
- **Sennet:** *Withdrawn 2026-11-14 ([protocol_parity D1](#protocol_parity-d1--foreign-meshes-through-stock-radios)): Meshtastic is reached through a stock companion radio; native Sennet roles are no longer built.* Extend selected routing, channel, discovery/configuration and standalone PKI/counter behavior; independently qualify chosen security, position, telemetry and module roles against public specifications and stock captures, including malformed input, restart and board memory. Meshtastic 2.8 ACK proof is not established by earlier correlated ACKs.
- **Product acceptance:** for each advertised image/role measure full workload and resource reserves and qualify applicable USB, BLE/Wi-Fi, display, GNSS/sensors, persistent configuration, upgrades/recovery, sleep, unattended behavior, power-cut and driver/deadline faults. Shared-radio missed windows are a separate controller gate. Requalify when the workload or image changes.

## Decisions

### protocol_parity D1 — foreign meshes through stock radios

**2026-11-14 · new.** Mark, on MeshCore (tucket) and Meshtastic (sennet):

> considering the stack, tucket and sennet are not so important nowadays ... reticulum covers what the other two don't and what they do. the only value would be interoperation with existing non reticulum nodes, and for that, a thinner layer is possible.

Card answer, "Stock radios only": interoperation goes through the existing stock-companion client; a stock MeshCore or Meshtastic radio holds the keys, we hold none and compose no crypto. tucket and sennet shrink to the frame types that client uses; the resident-protocols firmware image and WN7's foreign-adapter condition are withdrawn. It costs one stock radio per foreign mesh. Standing rule: "We should remain strictly consumers, not forking standards."

Links: area radio · plans Tucket, Sennet, Product acceptance · governs crates/tucket, crates/sennet, apps/signalman/src/radio_comms/companion.rs · summary foreign LoRa meshes are reached through stock companion radios; native Tucket/Sennet and the resident image are withdrawn

Landed 2026-11-14: the desktop stations, Signalman's resident journal, the resident image and the T114 Tucket service, radio-hand's resident runtime and the native protocol code are removed; tucket keeps `companion::DeviceInfo`, sennet keeps `protobuf` and the text-payload limit. Whether a Reticulum-only resident runtime is wanted again is not decided here.

### protocol_parity D2 — the IFAC mask keeps its own expand loop

**2026-11-14 · new.** Card answer, "Keep the loop as is": `mask` in `crates/retinue/src/ifac.rs` stays, the one hand-written primitive that does. It is HKDF-SHA256 as RNS's `HKDF.py` computes it, which past RFC 5869's 255 blocks (a link MTU above 8 KiB) wraps its one-byte block counter instead of refusing; the `hkdf` crate refuses there, so a library call would break wire compatibility on those links. No code change; the function's comment says so.

Coverage, checked 2026-11-14: the RNS 1.4.2-captured vector (`crates/retinue/tests/fixtures/ifac_packet.json`, a 47-byte frame) and `matches_pinned_rns_wire_bytes` pin the path below 255 blocks, where the loop equals the `hkdf` crate. The wrap itself is pinned by `the_mask_is_hkdf_and_wraps_its_counter_past_255_blocks`, written from `HKDF.py`, not by a captured RNS frame of that length.

Links: area retinue · governs crates/retinue/src/ifac.rs · summary the IFAC mask's hand-written HKDF-Expand stays because RNS wraps the block counter past 255 blocks

### protocol_parity D3 — the carrier crates fold into Signalman

**2026-11-14 · new.** After D1's cut `crates/tucket` was 45 counted lines (`DeviceInfo`) and `crates/sennet` 118 (the protobuf reader and `MAX_APPLICATION_PAYLOAD = 232`). Card answer, "Fold into Signalman, drop the crates": Signalman's stock-companion client owns those types and both crates leave the workspace.

They now live in `apps/signalman/src/radio_comms/companion/`: `meshcore.rs` (`DeviceInfo`), `protobuf.rs` (the reader) and `meshtastic::MAX_APPLICATION_PAYLOAD`, all public under `signalman::radio_comms::companion` with names and shapes unchanged. The structural-capture test, its three fixtures and the capture scripts moved with them (`apps/signalman/tests/`, `apps/signalman/capture/`), as did MeshCore's MIT notice (`apps/signalman/NOTICE`) and Sennet's clean-room record (`apps/signalman/capture/PROVENANCE.md`, now scoped to the reader, test, fixtures and scripts). The names remain published on crates.io at 0.0.2 and are no longer built from this repository; whether to yank or keep them is the owner's.

Still pending: `crates/station-custody/src/authority/tucket.rs` and `sennet.rs` have no caller in Murm but are half of a Djinn grant contract with Dramatis (`djinn-grants.json.in`); they leave in a joint removal with Dramatis.

Links: area radio · draws on protocol_parity D1 · governs apps/signalman/src/radio_comms/companion · summary tucket's and sennet's remaining frame types live in Signalman's companion module; both crates are removed from the workspace

### protocol_parity D4 — built V4 images leave the tree

**2026-11-14 · new.** Card answer, "Remove both", under the standing rule that build output is not kept in repositories: the two built Heltec V4 images under `firmware/heltec-v4-phy/` leave the tree.

Carried out for `tulle-heltec-v4-resident` (10,833,608 bytes, the withdrawn resident-protocols image): its package entry went on 2026-11-14 with D1, and nothing else read it. The blob stays in git history.

Not carried out for `tulle-heltec-v4-phy` (6,933,912 bytes), which is the payload of the live `retinue.heltec-v4` package. `firmware/packages/heltec-v4-current.toml` names it by `[payload] path = "../heltec-v4-phy/tulle-heltec-v4-phy"` with its `byte_length` and `sha256` and `source_revision = 711816a`; Linkboy (`apps/linkboy/src/package/verified.rs`) loads the payload from beside the manifest and refuses a hash mismatch; `index.toml` and `windows-v4-staging-index.toml` list the package; Signalman Desktop reads `index.toml` at run time; both `apps/signalman-desktop/tools/assemble-*-stage.ps1` scripts copy the file. The manifest format has no field for a payload outside the tree (no URL, no build step), and `cargo +esp build` writes to `target/`, so a fresh build at main neither lands at that path nor matches the pinned hash. Removing it needs a format or offering decision (a release-artefact URL in the manifest, a build-and-stage step that rewrites the hash, or withdrawing the V4 package): open, for the owner.

**2026-11-14 · refines.** Card answer, "Build-and-stage step": "A script builds the image and writes its hash into the manifest at staging time; nothing is downloaded." The card named the cost: "The pinned hash stops being a fixed, reviewed value and depends on a reproducible build, which a fresh build at main does not give today." The ruling also covers the two built T114 UF2 images, `tulle-t114-phy-current.uf2` (623,104 bytes, payload of `retinue.t114` in `index.toml`) and `tulle-t114-phy-v51.uf2` (550,912 bytes, pinned by `t114-v51.toml`).

Not yet carried out. The assessment found more build-dependent fields than the hash and a test dependency the ruling did not see:

- A manifest carries seven values a build decides: `byte_length`, `sha256`, `write_bytes` (V4: the espflash application image plus its `0x10000` offset, 402,880 + 65,536 = 468,416 for the pinned ELF), `write_ranges` (T114: Linkboy's UF2 check requires them to equal the blocks' span), `version`, `source_revision` and the `build=` revision in `manual_check`. A HEAD T114 image spans `0x26000..0x80600`, not the pinned `0x26000..0x72100`.
- Every `FirmwareCatalog::load` verifies every payload the index names (`PackageIndex::verify_packages`). Signalman Desktop's developer fallback and its tests load the checkout's `firmware/packages/index.toml`, and `owner_flow` asserts "the repository's own package catalog must verify": with the V4 image moved aside, 15 of its 86 tests fail (all 13 owner-flow, 2 accessibility). Signalman's `catalog_loads_and_resolves_verified_packages` does the same. Without tracked images that catalog cannot verify.
- The installed application already reads the stage: `default_catalog_path` takes `firmware/packages/index.toml` beside the executable, or `SIGNALMAN_CATALOG_PATH`, and Linkboy takes any manifest path. Where the generated manifest lives is therefore settled (the stage); its tracked form and the tests' catalog are not.
- `assemble-public-stage.ps1` has staged `t114-v51.toml` and its UF2 since before `index.toml` moved T114 to `t114-current.toml` on 2026-09-30, so its stage names a manifest it does not copy; `t114-v51.toml` is not in any index. v51 cannot be rebuilt from HEAD.

Reproducibility, measured 2026-11-14 at `19fa737` on this Windows host (receipt: [archive_docs/receipts](archive_docs/receipts/2026-11-02_protocol_parity_plan_receipts.md)). The V4 release build needs `--ignore-rust-version` (esp toolchain 1.97.0-nightly against the declared 1.97.1). Two clean V4 builds differ in 3 bytes, the build time in the ESP application descriptor; with `SOURCE_DATE_EPOCH` set to the commit time, which `esp-bootloader-esp-idf` honours, two clean builds are byte-identical. Two clean T114 builds and their Linkboy `make-uf2` encodings are byte-identical. Both images embed absolute paths (the V4 ELF's debug info names the checkout and user directory, the T114 binary the cargo registry path), so another checkout path or machine is not expected to match; this was not measured. No build at HEAD matches the tracked images, which were built from `711816a`; a rebuild at `711816a` was not attempted.

Forks for the owner: whether the tracked manifest becomes a template without the seven build fields or keeps its shape with placeholders; how Desktop and Signalman tests get a verifiable catalog once the images leave (a synthetic test package, or a staged catalog that the tests require); whether the v51 image and manifest leave with the stale public-stage line; whether staging sets `SOURCE_DATE_EPOCH`. Until answered, the three images stay tracked.

Links: area firmware · governs firmware/heltec-v4-phy, firmware/t114-phy, firmware/packages · summary built firmware images leave the tree for a build-and-stage step; the resident image is removed, the packaged V4 and T114 images wait on the manifest form and the tests' catalog

## Constraints

Keep device role, protocol feature and firmware-product acceptance separate; one endpoint exchange does not prove a router. Named 16 KiB untouched-stack and 25% free-heap reserves apply to the qualified memory profile, not all future images. Public contact storage contains pseudonymous associations, excludes secrets, labels, routes and pending operations, and requires its own scoped opt-in. Its commit is last, malformed or mismatched committed state refuses before erasure, and an uncertain readback is not a rollback promise. V4 live writes use the board quiet guard and make their listening blackout explicit. The current T114 endpoint uses volatile private setup; power cuts and installed daemon/UI remain unqualified.

## Findings, 2026-11-02

`crates/radio-hand/src/instances.rs` owns the bounded runtime; `crates/tucket/src/instance.rs` restores public contacts and handles ACKs, and `crates/sennet/src/instance.rs` requires a caller-reserved packet-ID interval. The later named pressure campaign measured painted high-water and fragmented heaps on both boards; selected V4 storage and T114 Tucket-only service have separate bounded reboot/traffic evidence. These are not simultaneous three-network traffic, broad role parity or unattended acceptance. Host contact recovery and the later board recovery supersede earlier open statements at their exact scopes only.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
- 2026-11-14: D1 recorded; the Tucket and Sennet phases are withdrawn and the native code removed. Product acceptance no longer includes the resident image. Retinue and the parity matrix are unchanged.
- 2026-11-14: D2 recorded; the IFAC mask's expand loop is documented as kept, no code change.
- 2026-11-14: D3 landed; tucket and sennet folded into Signalman's companion module and removed from the workspace. The validation suite `sennet-tucket-sans-io` became `signalman-comms`, and `sennet-capture-tooling` became `meshtastic-capture-tooling`.
- 2026-11-14: D4 in part; the resident V4 image is no longer tracked. The packaged `tulle-heltec-v4-phy` image stays until its package format is decided.
- 2026-11-14: D4 refined to a build-and-stage step and assessed; V4 and T114 builds reproduce locally (V4 only with `SOURCE_DATE_EPOCH`). Not carried out: the manifest form and the tests' catalog are back with the owner. No code change.
