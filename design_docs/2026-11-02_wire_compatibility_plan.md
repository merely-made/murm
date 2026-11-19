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

## RNS 1.6 and LXMF 1.3

The RNS 1.6.0 and LXMF 1.3.0 wheels came from the upstream Reticulum release node.
They were verified against its signed manifests and the pinned signing identity.
The intake and local interop receipts are under
`Code/testing/murm/testing/receipts/rns-1.6.0-candidate`.

The oracle pin moved after 35 Retinue and 18 Outrider loopback gates passed
on Windows with the default network stamp settings. The final totals include targeted
reruns of two Retinue harness fixes and four Outrider gates with their
required release build. Extra link MTU, RNS config and offline AutoInterface
checks passed. UDP's Unix test dependency and a shared instance restart
failure reproduced with the old pins on Windows. Physical radios and RF
did not participate in qualification.

The [protocol refresh](2026-11-17_reticulum_protocol_refresh.md) implements
network stamps on destination announces, path requests and link requests in
Retinue, and fractional delivery and propagation costs in Outrider. Operator
policy controls emission, enforcement, costs and bounded generation. The
new mixed-runtime tests include enforcement on/off, authenticated app data and
ratchets, stamped cold path discovery, reciprocal links, insufficient-work
refusal and valid retry, and integer/fractional LXMF exchanges.

The shared `StampCost` uses stock's exact 1/256 quantization and digest
thresholds. Configured propagation values retain precision through
subtraction of flexibility before the resulting floor is quantized.
Legacy cost fields remain encoded and validated during the transition.
Existing integer utilities and tickets keep their prior behavior.

The signed LXST 0.6.0 manifest arrived through Reticulum; its complete wheel
was fetched through HTTPS after bounded Reticulum failures, then verified
against that manifest. LXST remains outside these implementations. Its
assessment and the fresh signed RNS 1.6.0 release query are under
`Code/testing/murm/testing/receipts/reticulum-refresh-stock`.
The stock RNode updater still names 1.86.

Destination stamp caches are process-local; durable host/shell provisioning
is separate. Physical RF, public-network scale and LXST media remain open
qualification scopes. Recheck signed release identity and on-network defaults
when qualifying a newer RNS peer.

## Progress

- 2026-11-17: added `crates/retinue/oracle/peer_rnsrs.py`, a local peer gate with rns-rs's `rnsd` (`663e5e3`) as the transport node. Two full runs: the second passed 8 of 8; in the first, the Retinue-hub control for the Retinue pair case ended `NO_ROUTE`, then passed 8 of 8 on repeat. That control has no rns-rs in it, and the cause is not found. Results are in the [survey refresh](2026-08-25_permissive_radio_protocol_compatibility_survey.md#hands-on-check-of-rns-rs).
- 2026-11-17: implemented the RNS 1.6 network stamp and LXMF 1.3 fractional-cost slice; see the linked protocol refresh and software receipts.
- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
