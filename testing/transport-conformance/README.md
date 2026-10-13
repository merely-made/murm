# Transport conformance suite

An unpublished, isolated Cargo workspace. It links the isolated retained-repair
host core and changes no production protocol or radio behavior. Merely-owned code is
MPL-2.0; imported BLAKE3 and RFC artifacts retain their separately recorded
licenses and attribution in `vectors/manifest.json`.

Run from this directory:

```sh
cargo test --locked
cargo run --locked -- --list
cargo run --locked -- --output receipt.json
cargo run --locked -- --simulate --output exploration.json
```

The output path must be new; the runner refuses to overwrite receipts. Without
`--output`, it prints JSON to stdout. Exit status is 0 when executable cases
pass, 1 for a case failure, and 2 for a runner/setup error. `unsupported` is an
explicit coverage status and never contributes to the pass count. Once these
dependencies are cached, the same commands accept `--offline`.

Each receipt uses `transport-conformance.receipt/v1`, names the profile and
implementation, records the repository revision and dirty state, hashes the
compiled suite source/dependency lock/case metadata/corpus manifest, and returns
case IDs, specification sections, provenance, evidence classes and observations.
The revision is the base repository commit, not a claim that uncommitted suite
code belongs to it; the suite source hash identifies those exact compiled inputs.
All imported corpus artifacts are hash-checked before running.

## Retained repair prototype

`--simulate` runs four editable design experiments from
[scenarios/retained-repair-v1.toml](scenarios/retained-repair-v1.toml): finite
production with one/four radios, fresh priority with zero/one reserved repair
lane, and sustained overload with expiry. It invokes actual bounded RaptorQ
encoding and decoding, signs manifests/symbols with a public Ed25519 fixture
key, and records authenticated wire bytes, collection opportunities, successful
reconstruction, cumulative unmet demand and saved-state observations.

This mode emits `transport-conformance.simulation-exploration/v1`. It describes
the current prototype and makes no conformance, production-admission or
physical-throughput claim. Channels are ideal parallel transmitter lanes;
receivers already know the channel plan, manifest cadence and slot clock.
The [scenario guide](scenarios/README.md) explains the knobs and bounded JSON
`--simulation-peer` run/checkpoint/resume interface. The
[prototype profile](../../design_docs/2026-10-11_retained_repair_simulation_profile.md)
records the authored schedule and exact narrow CBOR arrays; its field schema is
[retained-repair-v1.cddl](scenarios/retained-repair-v1.cddl).

The bounded peer can also carry externally supplied original bytes using
`generation.original_hex`, and return exact digest-checked decoder output in
`recovered_object_hex`. The
[scenario guide](scenarios/README.md) records the input/output bounds. The
Mere-owned `retained_receive` example passes those opaque bytes through its
existing operation-record decoder and production MeshStore admission path.

The selected [2026-10-11 exploration report](results/retained-repair-exploration-20261011.json)
is a historical design observation with compiled source/config fingerprints.
Use small local checks and experiments while changing this shape; postpone
broad qualification and regression machinery until behavior stabilizes.
Publish through direct commits and pushes to `main`. GitHub Actions remains
disabled in Mere/Retinue by owner request; this work creates no workflows or PRs.

## Incremental retained peer

The isolated [`retained-repair` core](../../crates/retained-repair/README.md)
supports publication after startup, four retained originals, explicit expiry,
prepared-frame acceptance/deferral and a manifest-driven collector. Build the
persistent JSON adapter from this directory:

```sh
cargo build --locked --offline
target/debug/transport-conformance --retained-peer
```

Unlike `--simulation-peer`, this mode processes newline-terminated JSON without
waiting for EOF. It first emits `{schema:"retinue.retained-peer/v1",kind:"hello",
implementation,limits,scope,time}`. Each request is
`{schema:"retinue.retained-peer/v1",request_id:u32,now_slot:u32,command:{op,...}}`.
IDs strictly increase. `now_slot` is a monotonic logical slot, not seconds;
commands advance expiry before operating. A reply has `kind:"response"`,
`request_id`, `requested_now_slot`, actual `now_slot`, `status:"ok"|"refused"`,
`result` or `refusal:{code,detail,retry_at_slot?}`, implementation, producer/
collector stats and pending token. Refused commands may still advance expiry;
malformed/schema-rejected requests do not. After `recover`, use returned
`now_slot`, because that finite helper advances logical time internally.

| Command | Inputs and successful result |
| --- | --- |
| `init` | `public_fixture:true`, 32-byte `namespace_hex`, 32-byte public throwaway `authority_seed_hex`, `channels:1..4`, `manifest_interval:1..4096`, `repair_floor:0..channels`. One initialization per process; no secret credentials. |
| `publish` | `generation:u64`, `original_bytes_hex` of 1–16,384 bytes, `expires_at:u32`. Returns `outcome:"inserted"|"already_retained"` and metadata. Same-generation bytes/expiry changes conflict; new generations increase strictly. Full retention refuses before encoder construction, naming next expiry. |
| `snapshot` | Returns `inventory`, `retained_original_octets`, `collector_inventory`, `pending`. |
| `lookup_metadata` | `generation`; returns `metadata:{generation,digest_hex,object_octets,publish_at,expires_at}`. Digest hashes the whole opaque original, not its inner operation ID. |
| `prepare` | `channel`, optional `requested_generation`; returns `prepared:null` or `{token:u64,channel,lane,generation,esi,frame_bytes_hex,frame_octets,frame_blake3}`. Only one pending preparation; deferral retains it exactly. |
| `settle` | `token:u64`, `decision:"accept"|"defer"`; returns `settlement:"accepted"|"deferred"`. Acceptance names local sink custody and advances counters once. |
| `receive` | `frame_bytes_hex` of at most 512 bytes; returns manifest/symbol/duplicate/completed outcome. Completion returns `recovered`. |
| `recovered` | `generation`; returns `{metadata,original_bytes_hex,actual_decoder_output:true,application_admission:false}` from completed retained decoder state. |
| `recover` | `generation`; finite host loopback helper, refusing outstanding preparation. Returns `recovered` in the same shape, start/end slots and frame counts. It drops fresh frames and delivers signed manifests/repairs. Prior collector state/stats and input provenance are reported; earlier inputs can have included fresh symbols. This is owner-selected local recovery, not network dispatch. |

`expiry` exposes producer/collector object metadata and any invalidated token.
Its `during_command` array also preserves internal recovery expiry transitions
on success or refusal. Generation/slot exhaustion is explicit; a new epoch
requires a distinct namespace and new owner state. No restart persistence is
supplied. Original signed application bytes remain opaque; existing Mere
decoding/policy/storage owns admission.

Lines including newline are capped at 64 KiB before JSON parsing. Each direction
is capped at 4 MiB including hello/replies, with at most 4096 requests. Input,
actor and output queues each have one slot. The process has a 60-second lifetime,
five-second idle deadline and ten-second actor/output deadlines; timeout exits
without waiting for blocked standard threads. A parent must discard this
volatile process after closure/uncertainty. The core caps 512 unique symbols per
object and 4096 decode calls per epoch; `recover` caps 1024 frame settlements.
The 64 KiB original cap does not measure total codec heap or MCU suitability.

Run one small dynamic process experiment, writing a new output path:

```sh
python3 retained_live.py --peer target/debug/transport-conformance \
  --output /tmp/retained-live.json
```

The selected [2026-10-12 report](results/retained-live-20261012/exploration.json)
records publication of B while A is incomplete, fast fresh reception of B,
late channel-zero repair-only reconstruction of A, exact deferral, stale-token
refusal, pre-codec signature refusal, full retention and expiry. It retains the
process transcript and executing binary/compiled-source identity. The
[validation record](results/retained-live-20261012/validation.json) binds the
selected Cargo artifact and 42 source/validation inputs to commit `73616a78`.
These are
host delivery opportunities; the report starts no sockets, radios or store.
One collector holds separate A/B decoder states. The driver explicitly selects
A on channel zero and B on channels one through three, and discards every fresh
A packet to force repair-only reconstruction. This demonstrates coexistence
and late recovery rather than automatic scheduling fairness or independent
receiver processes.
The Mere-owned exact-inventory/admitted-ingress composition is published in
`26eb8038`, with acceptance `abdf6d51`. Its
[guide and receipt](https://github.com/merely-made/mere/blob/abdf6d51556dd175f57440199a8335e37bb19f1a/crates/mesh/mesh/examples/inventory_ingress/README.md)
own that evidence: actual decoder bytes cross QUIC into the existing resident
mesh admission owner, with separate inventory/submission authority and shared
service capacity. Retinue's peer continues to own only opaque host transfer state.

## Exercised scope

`metadata.toml` is the case catalogue. Current executable cases cover:

- All 35 official BLAKE3 input lengths in unkeyed, keyed and derive-key modes,
  including the corpus's extended output bytes: 105 comparisons. Vectors and
  Apache-2.0 license are imported from upstream commit
  `f55849f89cd85083c1c9daa2c5de20766f309c00`.
- RFC8032 section 7.1 TEST 1: derived Ed25519 public key, deterministic signature
  and strict verification; separately authored message/signature corruptions.
- RFC6330 four-octet FEC payload IDs and twelve-octet Object Transmission
  Information, network byte order, truncation/extra-length rejection, positive
  T/Z/N/Al, alignment and per-source-block symbol limit. The maximum transfer
  length is **942,574,504,275 octets**, incorporating Verified Technical erratum
  [5548](https://www.rfc-editor.org/errata/eid5548), rather than the uncorrected
  value printed in the original RFC.
- A bounded packet adapter that checks length, source-block range and resource
  limits before invoking assertion/index-based `raptorq` APIs.
- Three same-library RaptorQ reconstruction scenarios with two source blocks,
  erased packets, reversed delivery and duplicates, plus incomplete-object
  behavior. These are **composition** observations, not independent codec
  interoperability or full RFC6330 decoder conformance.

- Selected independent OpenRQ interoperability: six objects with T=128, N=1,
  Al=1, one or two blocks, initial K-prime transitions and partial final symbols.
  All source symbols and selected repair symbols are compared byte for byte:
  264 symbol comparisons. The default run decodes Java packets with Rust in 24
  delivery scenarios and checks 24 retained Java decodes against the exact Rust
  packets used in the live campaign. Deliveries include source-only, mixed loss,
  repair-only, reversal and duplicates. The Java direction in this receipt is
  **captured independent peer evidence**; the default Rust run executes no Java.

The imported independent implementation is OpenRQ Java at commit
`45f4437ba96d7e69a6ee05a2e51cf79cda90448e`, under Apache-2.0. Its source,
toolchain, wrapper and generator pins are recorded in
`vectors/openrq-interop.json`. The authored objects and harness are MPL-2.0.
OpenRQ requires Al=1, so this uses a distinct shared profile; the foundation
composition scenarios retain Al=4. OpenRQ omits final source-symbol padding;
the wrapper restores zero padding to T before full-symbol comparison and records
that normalization. OTI, payload IDs and repair bytes are compared unchanged.

To rerun both decoders live with pinned infrastructure, follow
[the independent campaign instructions](interop/README.md). Once its external
cache is bootstrapped, this command rebuilds the Rust peer offline, runs fresh
Java and Rust processes, and compares their results to the retained corpus:

```sh
python3 interop/openrq_interop.py --build-rust-peer \
  --receipt results/openrq-live.json
```

That campaign currently qualifies a macOS aarch64 Temurin toolchain and reports
missing infrastructure explicitly. The normal Cargo suite replays the corpus
portably without downloading OpenRQ or requiring Java. The retained
`results/foundation-20261011.json` is historical TC0 evidence; the later
`results/independent-20261011.json` reports TC1's compiled source and corpus.

## Bounded profile and remaining scope

The composition adapter accepts nonempty objects up to 64 KiB, symbols up to
256 octets, at most 16 nonempty source blocks, at most 256 source symbols per
block, N=1, Al=4, ESI up to 4095, and
exactly one full symbol per packet. Those are profile restrictions. RFC6330
allows F=0, requires receivers to support multi-symbol packets, and permits
omission of padding in a final source symbol. The metadata preflight accepts
F=0, while the bounded composition profile refuses it. The reserved OTI byte
does not influence parsed parameters; serialization emits zero.

Parameter preflight is not symbol authentication. These tests do not establish
protection against malicious, correctly sized repair symbols, a production
memory bound, embedded execution, radio throughput, channel isolation or power
use. The separate `--simulate` mode explores fresh/repair scheduling. Neither
mode implements FLUTE/ALC/NORM transport, a Stickleback admission adapter or
store mutation. Those remain separate composition choices in the owning plans.

**Complete RFC6330 codec conformance remains unsupported.** Multiple sub-blocks,
multi-symbol packet reception, shortened final-source packets, all systematic
index transitions and broader parameter/recovery/resource behavior need further
qualification. The independent vectors do not prove recovery from every K
symbols or measure recovery probability. Add spec-referenced cases before
broadening claims, and keep normative requirements, project-profile choices,
interoperability, composition and measured performance distinct in receipts.
