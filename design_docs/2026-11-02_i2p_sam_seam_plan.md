# Retinue I2P SAM seam

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-21_i2p_sam_seam_plan.md`; the original is archived whole.

## Open phases and done conditions

- **Identity fork:** decide whether non-transport nodes keep Retinue's stable I2P destination or mirror RNS's ephemeral identity unless the host requests static identity. Record the privacy and interoperability consequences before changing behavior. Decided 2026-11-05 for anonymous carriers: see the identity decision below; the stable-destination versus ephemeral question for non-transport nodes stays open.
- **Current-cohort interop:** repeat the other stock-RNS dial/IFAC variants, cross-router and restart cases against the actual receiving revision with a TCP positive control. Identify stock versions, keyfile/b32, route retention, redial cadence and recovery separately. Current Windows reconciliation reran only Retinue-dials-RNS without IFAC; retained earlier receipts remain historical cohort evidence.
- **SAM datagrams:** implement and qualify Datagram2/3 only when a consumer needs them; this is a deferred feature, not a prerequisite for the present stream seam.
- **Upstream findings:** preserve the original upstream ledger and bring any filing to the owner first. The emissary debug assertion mitigation and termination reporting do not establish absence of the underlying condition; i2pd split HELLO, RNS lookup failure and IFAC default behavior retain their exact observed scopes.

## Decisions and constraints

Use the owned MPL SAM client `truchman`; `murm-i2p` is a default-off host provider with explicit transit/publish inputs, external routers remain alternatives, and RNS keyfile layout is mirrored. Arti is deferred. `fresh_session_per_redial` defaults on, with an explicit host opt-out; recovery timing is observed, not guaranteed within one redial period. Preserve the accepted reseed RSA advisory rationale and revisit when its dependency changes.

## Findings, 2026-11-02

The [crates.io registry](https://crates.io/api/v1/crates/truchman) reports `truchman` 0.1.1 published 2026-10-24 and not yanked. Its published archive records Git revision `37f60c111e567d7e1a7b62ba323803a7fc6f044f`; all ten packaged Rust source files match current main after line-ending normalization. Publication of 0.1.1 is therefore complete, superseding the original unpublished note; future source changes require their own version and release decision. `crates/truchman/Cargo.toml` declares 0.1.1. `crates/retinue/src/endpoint/i2p.rs` defaults fresh-session redial on; `endpoint/iface_config/carriers.rs` provides section attachment; `native/crates/murm-i2p/src/lib.rs` defines the embedded provider. I2P0–I2P3 and historical I2P4 are recorded at their original scopes; October 26 restored missing code and ran one current stock-RNS variant. Locations and the original `truchman` publication were resolved later and are not open forks.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
- 2026-11-03: Moot relay-pool P2 adds direct SAM carriage behind optional
  `mere-transport/i2p` and `murm/relay-i2p`, using existing Truchman and secure
  Noise/Notochord setup. It changes no Retinue identity default. Pool privacy,
  public-router qualification and receipts belong to
  `gemot/design_docs/moothold_docs/implementation_strategy/2026-11-03_moot_relay_pools_plan.md`;
  its scripted and pool-specific public qualification does not close the
  Retinue interoperability gates.
- 2026-11-14: `murm-i2p` stays although no manifest calls it yet. Mark: "We can keep murm-i2p, on the same rationale that we would allow using alternate reticulum implementations; modular backends fit different profiles. That's the webrender lesson to a tee". External routers such as i2pd remain the alternative.

## Decision, 2026-11-05: I2P identity is unassociated by default

Release log decision D3 (2026-11-03, Mark, asked which key roots a site's address):

> you want all three; root, associated, unassociated addresses. unassociated first and foremost is the most secure and can be associated in a layer above at convenience, but deriving disposable personas from root for permission management is a legitimate benefit when you need to manage other people's identities without owning them (kids, biz, orgs, hierarchy). anonymity is important and manageable identity is important.

For an anonymous carrier, including I2P, the destination keys come from a Personae `UnassociatedKeypair` (fresh Ed25519, no derivation from any root, dramatis `crates/personae/src/unassociated.rs`), never from `derive_keypair` or the persona's plain identity. Consequences:

- A destination published for an unassociated identity links to no persona unless its holder signs an `AssociationClaim` (insigne `association`), a layer above the key. Associated destinations (derived personae) and root destinations stay available when the holder or a managing party picks them.
- Interoperability: Retinue's stable keyfile destination is unchanged for hosts that ask for a static identity. The unassociated key is a stored, custodied static key, so it survives restarts without being derived from anything; a host that wants an ephemeral destination simply generates a new one.
- Transport binding: `P2pandaTransport::builder_for` and `ReticulumTransport::builder_for` take any `CarrierKeypair` (derived or unassociated); a master key still cannot reach a transport. Direct SAM destination keys now come from this type: `mere-transport` (`i2p` feature) `anonymous_i2p_key(&UnassociatedKeypair)` builds the SAM private key (Ed25519 signing and X25519 encryption, HKDF children of the key's seed) that `I2pSession::create` takes; the argument type admits no persona or derived key, and a test shows no persona, derived or carrier key bytes in the destination. Not yet run against a live router. The stable-versus-ephemeral question for non-transport nodes remains open.
