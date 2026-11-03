# Retinue I2P SAM seam

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-21_i2p_sam_seam_plan.md`; the original is archived whole.

## Open phases and done conditions

- **Identity fork:** decide whether non-transport nodes keep Retinue's stable I2P destination or mirror RNS's ephemeral identity unless the host requests static identity. Record the privacy and interoperability consequences before changing behavior.
- **Current-cohort interop:** repeat the other stock-RNS dial/IFAC variants, cross-router and restart cases against the actual receiving revision with a TCP positive control. Identify stock versions, keyfile/b32, route retention, redial cadence and recovery separately. Current Windows reconciliation reran only Retinue-dials-RNS without IFAC; retained earlier receipts remain historical cohort evidence.
- **SAM datagrams:** implement and qualify Datagram2/3 only when a consumer needs them; this is a deferred feature, not a prerequisite for the present stream seam.
- **Upstream findings:** preserve the original upstream ledger and bring any filing to the owner first. The emissary debug assertion mitigation and termination reporting do not establish absence of the underlying condition; i2pd split HELLO, RNS lookup failure and IFAC default behavior retain their exact observed scopes.

## Decisions and constraints

Use the owned MPL SAM client `truchman`; `murm-i2p` is a default-off host provider with explicit transit/publish inputs, external routers remain alternatives, and RNS keyfile layout is mirrored. Arti is deferred. `fresh_session_per_redial` defaults on, with an explicit host opt-out; recovery timing is observed, not guaranteed within one redial period. Preserve the accepted reseed RSA advisory rationale and revisit when its dependency changes.

## Findings, 2026-11-02

The [crates.io registry](https://crates.io/api/v1/crates/truchman) reports `truchman` 0.1.1 published 2026-10-24 and not yanked. Its published archive records Git revision `37f60c111e567d7e1a7b62ba323803a7fc6f044f`; all ten packaged Rust source files match current main after line-ending normalization. Publication of 0.1.1 is therefore complete, superseding the original unpublished note; future source changes require their own version and release decision. `crates/truchman/Cargo.toml` declares 0.1.1. `crates/retinue/src/endpoint/i2p.rs` defaults fresh-session redial on; `endpoint/iface_config/carriers.rs` provides section attachment; `native/crates/murm-i2p/src/lib.rs` defines the embedded provider. I2P0–I2P3 and historical I2P4 are recorded at their original scopes; October 26 restored missing code and ran one current stock-RNS variant. Locations and the original `truchman` publication were resolved later and are not open forks.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
