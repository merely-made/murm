# Transport standards and retained repair conformance

**Status, 2026-10-11:** TC0's isolated host suite is implemented. TC1 now has a
selected independent OpenRQ campaign for `Al=1, N=1`: six objects, 264 exact
source/repair symbol comparisons and 24 live cross-decoding scenarios in each
direction. The offline suite verifies the imported corpus and replays the Rust
side; the local process campaign executes both implementations. Broader RFC
6330 coverage, production FEC delivery and radio measurements remain open.
TC3 now has an exploratory retained-repair scheduler and four design experiments
using actual coding/authentication, described in the
[prototype profile](2026-10-11_retained_repair_simulation_profile.md).
TC4's bounded host prototype now passes one recovered signed V2 mesh job record
to Mere's existing admission owner, preserving original bytes and identity.
It observes insertion, duplicate arrival and three invalid-inner-record refusals;
the continuation now carries these recovered bytes over real local QUIC.
Additional carriers and wider restart/qualification work remain open.
Publication uses direct commits and pushes to `main`; validation runs
locally. GitHub Actions and PRs are not part of this project's requested flow.

**Execution posture:** prototype the composed stack toward a useful final shape
before expanding qualification. Code and canonical documents carry the work.
Use small checks and experiments to answer current design questions; the gates
below describe later evaluation, not a requirement to certify each interim
state. Broad conformance, regression automation and physical campaigns belong
after the behavior and contracts have stabilized.

Mark authorized orchestration and prefers implementing published standards when
they cover the needed behavior. This plan turns the Reticulum/Dash composition
and fresh-data/retained-RaptorQ-repair discussion into bounded implementation
targets. Cross-repository research lives once in
`mere/design_docs/2026-08-24_standards_survey_brief.md` §10. Its authored inventory
is `mere/support/standards/transport_catalog.toml`; reusable suites and their
provenance are in `mere/support/standards/prior_art.toml`.

## Target and ownership

The target is a shared conformance corpus with selectable profiles and small
owner-local adapters, alongside a carrier-independent signed-object delivery
path. The corpus should be useful to another implementation without depending
on our application state. Compatibility claims name the profile, specification
revision, implementation revision and tested requirements.

| Component | Owner and boundary |
| --- | --- |
| Coding leaf | Bounded RFC 6330 source-block coding, payload identifiers and OTI. No routing, application authority or channel policy. Initially qualified as a host dependency; embedded suitability requires a separate resource profile. |
| Object transfer | Original signed operation bytes, transfer metadata, bounded decoder state, completion, retention and repair regeneration. Stickleback owns replicated-space receive/store mechanics; it does not replace domain admission. |
| Delivery adapters | Murm carries IP and bilateral Reticulum traffic. Outrider or a later Dash adapter can provide courier delivery. Full FLUTE/ALC, NORM or BPv7 are separate selectable interoperability profiles. |
| Radio execution | Tulle supplies modem interfaces; radio-hand owns device execution and the schedule. A host request or modeled airtime is not a physical radio observation. |
| Admission | Existing Mere OperationProcessor verifies structure/signature/body, then asks the domain policy, then changes the store. Link ACK, courier retention, reconstruction and admission are distinct events. |
| Test corpus | Retinue's standalone host suite, with external peer adapters and machine-written JSON results; human-reviewed profiles and inventories use TOML. No new universal application-state owner. |

Reticulum-compatible wire algorithms remain those specified by Reticulum. BLAKE3
is useful for application objects and corpus identity but does not replace
SHA-256/HKDF-SHA256 inside a Reticulum compatibility claim. Additional COSE
manifests must not silently re-sign or re-encode the original operation.

## Profiles and test contract

Separate three kinds of specification: published protocol requirements, selected
options/restrictions, and our new behavior. A conforming RFC 6330 codec inside a
CBOR radio transfer profile is possible. Calling that profile FLUTE requires the
actual FLUTE transport, FDT and other required behavior. Standards guidance,
experimental research, project protocols and vendor PHY commands retain their
own provenance in the catalog.

The planned corpus has stable case IDs, requirement clauses, required
capabilities, inputs, observable expected behavior, resource envelopes and
oracle provenance. Each imported vector retains its source revision, license
and content hash. The adapter controls are test interfaces, never additions to
the tested wire protocol. The eventual external-process interface exposes
capabilities, reset/configuration, byte input/output, virtual time, observable
events, checkpoint/restore and resource metrics. The first executable is a
smaller in-process seed runner; that does not close the portable-adapter gate.

Results preserve raw pass/fail and distinguish unsupported, skipped, timeout,
infrastructure error and inconclusive measurement. Expected-failure metadata
tracks implementation debt without converting it into a standards pass. Each
case has an evidence class: requirement/vector, independent interoperability,
composition, simulated delivery, fuzz/resource, performance or physical.
Agreement between one Rust codec and its bindings is one implementation lineage.
Coverage is a requirement/profile matrix, not a single compliance percentage.

The benchmark layer measures useful admitted payload, completion latency,
repair amplification, decoder memory/CPU, retained storage, airtime and energy.
Reports name source rate, radio count, loss model, PHY, channel width, object
sizes, security overhead and hardware. Performance numbers are not conformance
results. Simulated RF and physical qualification are different campaigns.

## Retained repair profile to specify

Use bounded, independently decodable generations. Send new systematic symbols
and early repairs while retaining old objects and generating additional repair
symbols for receivers that missed them. Symbol identity includes the transfer
namespace, generation, source-block number and ESI; repeated symbols do not add
new information. State and scheduling limits include the number of active
objects, block size, received-symbol budget, decode attempts and retained bytes.

The schedule needs authenticated repeated rendezvous information, so losing one
packet does not lose the only pointer to the next channel. It must define slot
time, channel sequence, independent radio streams, tuning guards, clock drift,
late joining, generation manifests and expiry. Physical scheduling is new
project work; no cataloged RFC standardizes this entire arrangement.

Reserve a minimum repair budget for retained objects and bound newest-object
priority. Advertise whether a receiver is still inside the retention horizon.
Feedback, where a return path exists, requests a bounded deficit rather than a
list of every lost frame; suppress redundant requests. If a full NORM profile
is selected, implement its specified behavior instead of naming custom feedback
NORM. Loss recovery is probabilistic: arbitrary K collected symbols do not
unconditionally guarantee reconstruction.

Authentication must precede expensive decode where the selected bearer/profile
can provide it. QUIC authenticates received connection packets; raw broadcast
requires its own explicit per-symbol or batch authentication decision. A final
object hash/signature can detect a bad reconstruction but cannot identify a
poisoned symbol before decoding. Tests must bound CPU/memory for unauthenticated
metadata, replayed IDs and malicious repairs. Signed manifests bind codec
parameters, object identity and retention information. Encryption is composed
with the existing group-security owner, with an explicit binding of ciphertext
and coding metadata; FEC does not invent a second group-security protocol.

Catch-up is conditional on capacity. For a given receiver, the long-run source
rate it is expected to recover must be below its useful collection capacity,
with room for loss, coding overhead and repair contention. It must also recover
each generation before its expiry. More radios can reduce completion latency
and collect more streams; coding cannot make an indefinitely overloaded
single-radio receiver catch up. The model must include both finite bursts and
permanent overload, and report backlog growth/refusal rather than declare
eventual delivery without conditions.

## Phases and done-conditions

| Gate | Target | Done-conditions |
| --- | --- | --- |
| TC0 | Catalog and suite seed | Canonical catalog and prior-art provenance validated; isolated executable emits per-case requirement/evidence/status/revision records. Official crypto vectors and reviewed RFC 6330 wire/boundary cases run. Missing independent peers are explicit unsupported results. This is a foundation, not whole-RFC qualification. |
| TC1 | Independent RFC 6330 codec qualification | Selected OpenRQ `Al=1, N=1` campaign is implemented; exact symbol and bidirectional decode cases cover K-prime transitions, uneven two-block partition, padding and repair-only recovery. Pinned tooling and fixtures make those cases reproducible. Broader alignment, sub-block, ESI and resource coverage remains open before wider claims. |
| TC2 | Portable corpus/adapters and existing carriers | External-process capability/byte/event contract; register existing stock-RNS oracle gates and a real QUIC independent-peer campaign. Optional features remain explicit. Result schemas and requirement traceability work across at least two implementation lineages; vector imports retain licenses. |
| TC3 | Retained-repair prototype, implemented | Authored schedule and signed manifest/symbol arrays; actual codec; one-/multi-radio collection, loss/reorder/duplicates, sleep/drift/guards, late join, repair rotation, expiry and partial-state resume. Four selected experiments expose catch-up, floor cost and overload behavior. The profile and inputs may change; stable-contract evaluation is later work. |
| TC4 | Signed-object receive and local QUIC prototypes, implemented; wider evaluation open | The selected host example passes actual recovered original bytes/IDs through bounded real QUIC to the existing mesh admission owner, observing insertion, duplicate arrival, wrong-mesh/body/signature refusals, pre-owner framing refusals and in-process store reopen. Later evaluation covers expired transfers, actual two-carrier duplicates and fresh-process recovery after the contracts stabilize. |
| TC5 | Device resource profile | Selected codec/profile compiles for the actual target; measured peak heap/stack, decode time, retained storage and interrupt/radio coexistence fit named limits. no_std availability alone does not close this gate. |
| TC6 | Physical multi-radio qualification | Named boards, firmware hashes, radios, antennas, channels and instruments; actual payload goodput, range, adjacent-channel isolation, retune/clock cost, collision behavior, power and source/repair airtime. Regulatory band/duty/power conditions are explicit. 1–2 Mbps remains a target until this campaign measures it at the intended range. |
| TC7 | Optional full protocol adapters | Select real peers/use cases first. Each FLUTE/ALC, NORM, BPv7/BPSec, SCHC/LoRaWAN, CoAP/OSCORE or MLS branch gets its dependency closure, selected options, independent peer and its own conformance matrix. Their spec counts are not a mandate to implement them all. |

## Findings — 2026-10-11

Source snapshots: Retinue `7aa87f2e9fdd973e4729e610d7130189dc22c0a0`, Mere
`0e78fcffb140395a5c0d2f64a4dfda2b81751681`, fetched before inspection. The primary
checkouts have unpublished local commits and lag their remotes, so this work
uses isolated branches from those fetched refs.

- Tulle's `crates/tulle/src/modem.rs` has an object-safe sans-io modem seam and
  heterogeneous modem collections, but its current parameters are LoRaParams.
  That is not FLRC/HaLow qualification.
- Mere's Reticulum stream adapter at
  `mere/crates/murm/transport/src/reticulum_transport.rs` is bilateral and
  default-off; current blob/sync remain on the iroh path. JoinedSpace still
  constructs p2panda Endpoint/Gossip/LogSync. Non-IP frontier sync is new work.
- Mere's `mere/crates/stickleback/src/processor.rs` separates structural
  validation, domain admission and store insertion. This is the recovery
  endpoint; passing a decoder is not application acceptance.
- The current RNS oracle requirements at
  `../crates/retinue/oracle/requirements.txt` pin RNS 1.5.7/LXMF 1.2.0.
  `../crates/retinue/oracle/run_live.py` contains 35 process-isolated gates.
  They were inspected, not rerun for this suite seed. Its README's older
  versions must not supply a new run's provenance. Prns remains withdrawn as a
  trusted donor/reference by the existing owner policy.
- No RaptorQ/FLUTE/NORM implementation was found in the inspected production
  source. The existing deterministic routing simulator has topology/cuts/clocks
  but no multi-channel FEC model.
- [RFC 6330 §3](https://www.rfc-editor.org/rfc/rfc6330.html#section-3) specifies
  a four-octet payload ID and eight-plus-four-octet OTI. Verified technical
  [erratum 5548](https://www.rfc-editor.org/errata/eid5548) corrects maximum F to
  942,574,504,275 bytes. The seed must validate lengths/parameters before calling
  codec constructors or deserializers with assertions/indexing.
- The bounded survey initially found no broadly maintained independent RaptorQ
  vector corpus. This slice adds a six-object corpus generated by the pinned
  OpenRQ Java peer and compared against Rust `raptorq 2.0.1`. OpenRQ is old and
  supports only `Al=1, N=1`; its restrictions remain explicit.
- [Dash's September 16 update](https://p2panda.org/2026/09/16/dash-router-sync-api.html)
  describes a standalone networking/routing/sync system feeding the p2panda
  processing pipeline and says proof-of-concept implementation is next.
  This supports a separate adapter and simulation campaign, not a claim that
  Dash currently provides our radio profile or unconditional delivery.

## Progress

- **2026-10-11:** three parallel research passes completed: primary standards
  metadata, reusable suites/provenance and current owner seams. Created isolated
  Mere/Retinue worktrees from fetched remotes. TC0 executable and durable catalog
  work started; later gates remain open.
- **2026-10-11, TC0 foundation:** the isolated
  [suite](../testing/transport-conformance/README.md) and
  [machine receipt](../testing/transport-conformance/results/foundation-20261011.json)
  record seven passing groups, no failures and one unsupported independent
  encoder campaign. BLAKE3 covers 35 lengths in three modes (105 comparisons);
  Ed25519 covers RFC 8032 TEST 1 plus two mutations; RFC 6330 covers selected
  exact metadata/parameter cases and three same-library recovery scenarios.
  No production integration, independent RaptorQ interoperability, embedded,
  performance or RF claim follows. Source fingerprint:
  `ca28b14b5a6a3958ce9fdd1549a594890c2897fd49f017abdf0a36cdc9ce8207`.
- **2026-10-11, review and verification:** an independent review matched the
  imported official vector bytes and found two seed gaps. Fixed exact corpus
  schema/artifact-set/duplicate verification, and bounded source-block work to
  256 symbols before decoder construction. The accepted 64 KiB/T4/Z1 overload
  is now refused. Scoped locked offline tests, Clippy with warnings denied and
  formatting pass; root reran the complete scoped test after the review fixes.
  Catalog parse/count/profile-reference checks pass. Mere's before/after
  active-doc and D2 audit results introduce no new findings; existing broken
  references, one missing D2 record and the pre-existing snapshot-digest error
  remain baseline debt. This foundation receipt predates the TC1 campaign below.
- **2026-10-11, TC1 selected interoperability:** the pinned OpenRQ peer at
  `45f4437ba96d7e69a6ee05a2e51cf79cda90448e` and Rust `raptorq 2.0.1` agree on
  264 exact source/repair symbols for six deterministic objects. Each decoder
  reconstructs the other encoder's packets in 24 live scenarios, including
  all-source, mixed loss, repair-only and reversed repair packets with
  duplicates. Cases cover K=10/11/12/13, partial final symbols and unequal
  two-block partitions. OpenRQ's shortened final source symbol is explicitly
  zero-padded to the selected full-symbol representation before comparison;
  repair bytes are unchanged. This does not measure arbitrary-K recovery
  probability or complete RFC conformance.
  The [independent corpus receipt](../testing/transport-conformance/results/independent-20261011.json)
  records eight passing groups, zero failures and one explicitly unsupported
  full-codec qualification, and distinguishes current Rust replay from retained
  Java observations; the
  [live peer receipt](../testing/transport-conformance/results/openrq-live-20261011.json)
  records actual process execution, source/toolchain/wrapper digests and the
  Rust executable hash. The [reproduction guide](../testing/transport-conformance/interop/README.md)
  uses explicit bootstrap, archive verification, fresh Java compilation and a
  current locked offline Rust build. Missing tooling is an infrastructure
  failure. The suite is registered in `validation/manifest.toml`, with local
  offline cases and machine receipts. Broader TC1 qualification and
  TC2–TC7 remain open.
- **2026-10-11, owner workflow correction:** removed the newly added transport
  workflow, disabled GitHub Actions in Mere and Retinue, closed the temporary
  draft PRs and published directly to `main`. Existing CI observations remain
  historical evidence. Subsequent work uses local checks and experiments,
  with code and canonical docs as the execution frontier.
- **2026-10-11, retained-repair prototype:** implemented an isolated slot model
  using actual `raptorq`, Ed25519 and a narrow authored deterministic CBOR
  profile. It retains originals, reserves a configurable repair floor and
  rotates live generations through persisted queues. Full-radio receivers stay
  assigned to channels; partial receivers use preconfigured manifest cadence
  and clock assumptions. Authentication precedes decoder work. All manifest,
  source, repair and signature bytes count in the model. The
  [four-experiment report](../testing/transport-conformance/results/retained-repair-exploration-20261011.json)
  records finite catch-up, the zero/one-floor capacity tradeoff, cumulative
  unmet demand under overload, and in-process partial-state resume. Five small
  local tests and strict Clippy passed; a separate manual fresh-process smoke
  matched pending-delivery resume, without a retained receipt. These observations
  guide the prototype and do not close conformance, admission, MCU or RF gates.
  The report fingerprints the compiled source/config and names its dirty base
  revision honestly. The owner-local continuation below now passes one recovered
  original signed object through the existing operation processor; broader
  qualification waits for a stable shape.
- **2026-10-11, signed-object receive prototype:** added bounded external
  originals to the isolated simulator and return the actual decoder bytes after
  manifest length/digest verification. Retinue leaves the application record
  opaque. `mere/crates/mesh/mesh/examples/retained_receive/README.md` and
  `mere/crates/mesh/mesh/examples/retained_receive/receive-exploration-20261011.json`
  own the selected experiment and its source/executable identity. A current
  1,015-byte CBOR operation record (eight source symbols) reconstructs at slot 2
  for four radios and slot 16 for one radio joining at slot 6. The same original
  header/body and operation ID enter `MeshStore::accept_outcome` with the
  receiver's independently chosen mesh: the first arrival inserts one posted
  job, the second is a duplicate. Correctly delivered wrong-mesh, altered-body
  and altered-signature records are refused without changing the store or board.
  In-process redb close/reopen preserves the accepted bytes and effect. Six
  small suite tests and strict scoped Clippy passed; the existing external
  smoke also covers saved recovered bytes after decoder-state expiry. This is
  real host admission with modeled collection; the original report does not
  establish live carrier, worker/blob, fresh-process restart, MCU or RF
  qualification. The real-carrier continuation below adds local QUIC without
  expanding broader qualification.
- **2026-10-11, bounded real QUIC handoff:** Mere's same receive example now
  offers `--carrier quic`, carrying actual recovered bytes through production
  p2panda-net/iroh `Transport` streams. Stickleback owns a generic bounded opaque
  object-stream leaf; the receiving host checks ALPN/transport peer and clean
  frame completion, then calls the existing independently scoped mesh admission
  owner. An acknowledgement follows that owner's result; transport identity
  never replaces the inner signed author. The selected report lives once at
  `mere/crates/mesh/mesh/examples/retained_receive/quic-handoff-exploration-20261011.json`.
  The finite host caps objects at 16 KiB, acknowledgements at 2 KiB, attempted
  sessions at 12, request frames at 64 KiB and reserved request/reply frames at
  128 KiB, with one in-flight exchange and explicit I/O/campaign deadlines.
  Refused attempts consume those budgets. Actual loopback routes and active
  paths are observed. Oversized, truncated and stalled streams stop before
  admission, and subsequent valid transfers still reach the owner. The saved
  run completed 11 attempts within its budgets and retained one operation and
  one posted job. The object leaf's four scoped checks and the example's strict
  scoped Clippy check pass; the full strict Stickleback crate lint encounters
  two existing unchanged store lints, and passes with those lint classes
  excluded. The example remains a bounded host composition prototype. The
  global production accept queue, other carriers, cross-host service wiring,
  worker/blob delivery, MCU and physical radio behavior retain their own work.
