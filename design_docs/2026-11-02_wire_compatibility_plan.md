# Wire compatibility and ownership boundaries

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-09-27_wire_compatibility_plan.md`; the original is archived whole.

## Open phases and done conditions

- **Embedded IFAC:** choose volatile confidential provisioning or a separate durable vault/recovery design. Negotiate an extended versioned exchange before secrets; cover unsupported peers, fragmentation, wrong recipient/context, tampering, redacted diagnostics and protected startup. Durable storage additionally needs torn-write, rollback, downgrade and identity preservation. Then qualify identified board images against stock positive and negative physical peers. Existing software carrier tests do not close this gate.
- **Compressed stream resources:** measure total peak allocation including decoder workspace and allocator overhead at configured bounds; preserve terminal errors, already delivered prefixes and the distinction between admitted proofs and later decoding failures.
- **Stock and stress:** separately qualify queue pressure, natural route expiry, discovery metadata and applicable public-network behavior. For selected Sennet and Tucket profiles test identified stock releases, maximum lengths/refusals, restart and packet-ID continuity, retries, routing and role-specific behavior. Later native Tucket/Sennet and T114 passes are bounded profiles; use [protocol parity](2026-11-02_protocol_parity_plan.md) as owner for broader roles. Keep maximum-size Sennet stock acceptance open after interrupted USB trials.
- **Image acceptance:** complete the exact resident image's full visit/cancellation proof, fault, memory-high-water and unattended gates, and release-52 same-image recovery retry; preserve firmware and identity. See the controller and parity plans for subsequent qualified scopes.

**Withdrawal note, 2026-11-14 ([protocol_parity D1](2026-11-02_protocol_parity_plan.md#protocol_parity-d1--foreign-meshes-through-stock-radios)).** Mark: "Stock radios only": a stock radio holds the keys, we hold none and compose no crypto, and the resident-protocols image is withdrawn. The Sennet and Tucket clauses of **Stock and stress** and all of **Image acceptance** concern native code and an image that are removed; the Retinue clauses of Stock and stress are unaffected.

## Constraints and decisions

Credentials must not be appended to the identity/settings body or sent through an incompatible fixed-length resident message. A signature alone is not confidentiality. Existing settled topology, IFAC and stream behavior remains; keep historical failure and peer provenance distinct. Full protocol roles and private-channel decisions are not implied by a text exchange.

## Deferred evidence question

The relocated NomadNet interoperability record retains unsent upstream issue drafts for the named nomadnet-rs request-ID/response decoding and rns-rs activation observations. Keep their measured-version scope and diagnostic controls; owner review is required before any filing. The record is `Code/testing/murm/design_docs/2026-09-13_nomadnet_go_resource_compression_receipt.md`.

**Link-ID flag-mask evidence.** The original two stock RNS request/link-ID captures both used flags `0x02`, so they establish payload clipping but do not independently distinguish `flags & 0x0f` from an unmasked flag. When this evidence question is pursued, capture an independently generated stock multi-hop or other applicable request with discriminating flag bits and its accepted link ID, and compare the exact preimages. Done when the observation either distinguishes the alternatives or identifies why the requested flag shape cannot be exercised; do not claim a presently demonstrated interoperability failure. The [original v0 plan](archive_docs/2026-11-02/2026-07-06_retinue_v0_plan.md) retains the historical context; the optional question is carried here instead of a separate v0 successor.

## Findings, 2026-11-02

`crates/radio-hand/src/retinue_carrier.rs` implements the shared IFAC adapter and `crates/retinue/src/channel/buffer.rs` has a configurable decoded-frame limit; provisioning, total allocator cost and physical qualification are separate. The stock-RNS Channel/Buffer gate passed at its named baseline; RNS 1.5.7 later replaced the earlier pin. `crates/retinue/src/endpoint/i2p.rs` implements a separately qualified I2P path, so the old statement that I2P behavior was unqualified at the repin is historical; broader I2P variants are owned by [I2P SAM](2026-11-02_i2p_sam_seam_plan.md).

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
