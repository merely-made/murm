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

### protocol_parity D4 — built firmware images leave the tree

**2026-11-14 · new.** Card answer, "Remove both", under the standing rule that build output is not kept in repositories: the two built Heltec V4 images under `firmware/heltec-v4-phy/` leave the tree.

Carried out for `tulle-heltec-v4-resident` (10,833,608 bytes, the withdrawn resident-protocols image): its package entry went on 2026-11-14 with D1, and nothing else read it. The blob stays in git history.

Not carried out for `tulle-heltec-v4-phy` (6,933,912 bytes), which is the payload of the live `retinue.heltec-v4` package. `firmware/packages/heltec-v4-current.toml` names it by `[payload] path = "../heltec-v4-phy/tulle-heltec-v4-phy"` with its `byte_length` and `sha256` and `source_revision = 711816a`; Linkboy (`apps/linkboy/src/package/verified.rs`) loads the payload from beside the manifest and refuses a hash mismatch; `index.toml` and `windows-v4-staging-index.toml` list the package; Signalman Desktop reads `index.toml` at run time; both `apps/signalman-desktop/tools/assemble-*-stage.ps1` scripts copy the file. The manifest format has no field for a payload outside the tree (no URL, no build step), and `cargo +esp build` writes to `target/`, so a fresh build at main neither lands at that path nor matches the pinned hash. Removing it needs a format or offering decision (a release-artefact URL in the manifest, a build-and-stage step that rewrites the hash, or withdrawing the V4 package): open, for the owner.

**2026-11-14 · refines.** Card answer, "Build-and-stage step": "A script builds the image and writes its hash into the manifest at staging time; nothing is downloaded." The card named the cost: "The pinned hash stops being a fixed, reviewed value and depends on a reproducible build, which a fresh build at main does not give today." The ruling also covers the two built T114 UF2 images, `tulle-t114-phy-current.uf2` (623,104 bytes, payload of `retinue.t114` in `index.toml`) and `tulle-t114-phy-v51.uf2` (550,912 bytes, pinned by `t114-v51.toml`).

The assessment that followed found more than the hash. A manifest carries seven values a build decides: `byte_length`, `sha256`, `write_bytes` (V4: the espflash application image plus its `0x10000` offset, 402,880 + 65,536 = 468,416 for the old pinned ELF), `write_ranges` (T114: Linkboy requires them to equal the UF2 blocks' span), `version`, `source_revision` and the `build=` revision in `manual_check`. Every `FirmwareCatalog::load` verifies every payload its index names, and Signalman Desktop's developer fallback and tests loaded the checkout's index: with the V4 image moved aside, 15 of Desktop's 86 tests failed. `assemble-public-stage.ps1` had staged `t114-v51.toml` since before `index.toml` moved T114 to `t114-current.toml` on 2026-09-30, so its stage named a manifest it never copied.

**2026-11-14 · refines.** Card answers to those forks. Tracked form: "A template without those fields": the tracked file carries the package's identity, device and instructions, the staging step writes the full manifest into the stage beside the image, and a tracked manifest can never be mistaken for a verifiable one. Tests: "A synthetic package built in the test", needing no firmware toolchain and exercising the same verification code; the developer fallback to the checkout's catalog goes. Old images: "Remove all but the recovery image": `tulle-t114-phy-v47.zip` and `t114-v47.toml` stay "because a recovery image must exist before anything can be built". The driver added, on the measurement below, that staging sets `SOURCE_DATE_EPOCH` to HEAD's commit time for the V4 build. The Meshtastic release package is not decided here (stock firmware on demand is a separate question) and is staged as it was.

Landed 2026-11-14:

- `firmware/packages/heltec-v4-current.template.toml` and `t114-current.template.toml` replace the two manifests. They carry `template = 1`, which the strict manifest shape rejects, and none of the seven build fields; Linkboy refuses either as a package. `index.toml` and `windows-v4-staging-index.toml` are unchanged and name the manifests a stage holds.
- `linkboy stage-catalog` (`apps/linkboy/src/catalog/stage.rs`, `package/template.rs`) completes each templated index entry from its built image, copies untemplated entries with their verified parts, and verifies the staged catalog as a whole before returning. A UF2's `write_bytes` and `write_ranges` come from the same block walk the layout check uses; an espflash ELF's write range runs from 0 to the first preserved range. Verification itself is unchanged.
- `apps/signalman-desktop/tools/stage-firmware.ps1` builds every templated image from HEAD (refusing a dirty tree without `-AllowDirty`, and a destination inside the repository), sets `RETINUE_FIRMWARE_REVISION`, and for the V4 build `SOURCE_DATE_EPOCH`; it needs espflash 4.5.0 for the V4 write size and `rust-objcopy` for T114, and encodes the UF2 with `linkboy make-uf2`. Both assemble scripts call it. The public stage no longer stages v51, and neither stage lists the retired `prns.hopspot` id. Nothing is downloaded.
- Desktop's owner-flow and accessibility tests and Signalman's catalog test stage the repository index with synthetic images through `stage_catalog`. `default_catalog_path` takes `SIGNALMAN_CATALOG_PATH` or the stage beside the executable, nothing else.
- Removed: 104 tracked files, 25,657,523 bytes: `tulle-heltec-v4-phy`, every built T114 `.bin`, `.zip` and `.uf2` except v47, and `t114-v51.toml`. They stay in git history. `t114-v51-recovery.md` stays because the Meshtastic entry's `recovery_url` names it.

What a reviewer can verify from the repository alone: the source, each template, the build commands and their environment, the verification code, and that the templates cannot pass as packages. What they cannot: the bytes of any image Linkboy will flash, or its hash. A staged manifest records the revision it was built from and is evidence about that stage only; it is not a pinned, reviewed value, as the card said. Earlier receipts qualify the `711816a` bytes, which are no longer in the tree.

Reproducibility, measured at `19fa737` on this Windows host ([receipts](archive_docs/receipts/2026-11-02_protocol_parity_plan_receipts.md)): two clean V4 builds differed only in the application descriptor's build time and were byte-identical with `SOURCE_DATE_EPOCH` set; two clean T114 builds and their UF2 encodings were byte-identical. The esp toolchain here needs `--ignore-rust-version` (1.97.0-nightly against the declared 1.97.1). Builds from another checkout path or machine are not expected to match: both images embed absolute paths (the V4 ELF's debug info names the checkout and user directory, the T114 binary the cargo registry path). This was not measured.

Links: area firmware · governs firmware/packages, firmware/heltec-v4-phy, firmware/t114-phy, apps/linkboy/src/catalog/stage.rs, apps/linkboy/src/package/template.rs, apps/signalman-desktop/tools · summary the repository keeps package templates; a staging step builds each image from HEAD and writes its manifest beside it; only the v47 recovery image and the Meshtastic release stay tracked

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
- 2026-11-14: D4 refined to a build-and-stage step and assessed; V4 and T114 builds reproduce locally (V4 only with `SOURCE_DATE_EPOCH`). Not carried out: the manifest form and the tests' catalog went back to the owner. No code change.
- 2026-11-14: D4 landed; package templates, `linkboy stage-catalog` and `stage-firmware.ps1`, synthetic-catalog tests, and 104 built files (25,657,523 bytes) removed. v47 and the Meshtastic release stay tracked.
