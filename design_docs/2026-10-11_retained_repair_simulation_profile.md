# Retained repair simulation prototype

**Status:** exploratory host model, implemented on 2026-10-11. This
document records the current simulation choices so they can be changed with
the prototype. It is not an RF wire protocol or a qualification gate. The owning sequence and later
admission/device gates are in the
[transport conformance plan](2026-10-11_transport_conformance_plan.md).

## Standards and evidence boundary

The model uses the pinned `raptorq 2.0.1` implementation for actual RFC 6330
encoding and reconstruction. It uses the existing bounded `Al=4, N=1` profile,
exact twelve-octet OTI and four-octet payload IDs. The separately qualified
OpenRQ `Al=1` corpus remains separate. Selected interoperability evidence does
not establish complete RFC 6330 conformance or reconstruction from every set
of K symbols.

Manifest and symbol authentication use Ed25519 signatures over exact authored
CBOR array encodings. The narrow encoder follows RFC 8949 core deterministic
rules: definite lengths and shortest integer/length representations; this
schema has no maps, floats or tags. The envelope, trust root and schedule are
project choices. They do not establish COSE, TLS, FLUTE, NORM or Reticulum wire
compatibility. Namespace and generation identity bind coding metadata and
payload IDs to their retained object.

The [CDDL field schema](../testing/transport-conformance/scenarios/retained-repair-v1.cddl)
declares two bodies. A manifest is `[1, 0, namespace, generation, publish,
expiry, OTI, object_digest, channels, manifest_interval, repair_floor]`; a
symbol is `[1, 1, namespace, generation, BLAKE3(manifest_body), packet]`.
The signed envelope is `[body_bytes, signature]`, with the signature over the
exact encoded body bytes. A fixed published test key is the simulation
authority; it is not a user credential. Structural validation also rejects
unknown fields/types, overlong encodings, wrong lengths and trailing bytes.
CDDL is a readable field contract here; independent general-purpose CBOR/CDDL
validation remains a separate qualification slice.

All successful reconstruction metrics mean decoded original bytes whose hash
matches the manifest. They do not mean domain authorization or store admission.
The existing operation processor remains the eventual admission owner.

## Time, capacity and retention

Time is an integer slot index. The modeled transmitter has parallel capacity
to offer one bounded frame on every channel in a slot. The four-channel
experiments therefore assume four simultaneous transmitting lanes as well as
the compared receiver radios. Every manifest repeat, source frame, repair frame
and authentication byte contributes to the modeled transmission accounting.
The slot octet budget is a declared model input. No slot-to-seconds, modulation,
range, interference, duty-cycle or energy conversion is assumed.
Channels are ideal independent lanes. Multiple modeled radios can collect
simultaneous lanes without adjacent-channel leakage or shared-antenna costs.
Those assumptions must be revisited for an actual radio implementation.

Each generation declares a namespace-local ID, publication slot, expiry slot,
object length and coding metadata. Retained originals regenerate fresh repair
symbols while a generation remains live. A repeated symbol ID adds no new
information. Fresh source work has priority only after the declared repair
floor is reserved. Repair work rotates among live generations, and spare
fresh capacity may carry repairs. Floor zero is permitted as an explicit
counterfactual to test starvation; the proposed retained-repair schedule uses
a positive floor. Anchor manifests consume channel zero first; the available
data lanes then receive up to the requested repair floor. A one-channel
manifest slot has no remaining data lane. Expiry terminates
transmission/reception eligibility.

Repeated anchor-channel manifests let receivers learn a generation without
receiving its first announcement. Receivers already know the channel plan,
manifest cadence and common slot clock; acquisition of an unknown beacon or
clock is future prototype work. Full-radio receivers remain assigned to all
channels. Partial receivers hop a contiguous channel window and prefer channel
zero on predicted manifest slots. Join time, phase, dwell, sleep, drift and
nominal dwell guards reduce collection opportunities. The anchor override does
not model a synthesizer's actual settle transition. Radio capture is assessed
at transmission; a bounded reorder queue and duplication then affect delivery
without inventing extra radio capture or transmitter airtime. Seeded erasure
and burst erasure are repeatable impairment inputs, not RF measurements.

Catch-up claims name the finite workload, horizon and retention lifetime.
Reports retain cumulative produced and completed object bytes, including
unmet demand from expired work. An incomplete expired generation remains
unmet; historical successful completions survive expiry. A source rate
exceeding a receiver's one-symbol-per-opportunity
upper bound cannot be made recoverable by retaining repairs. A bounded live
queue alone is not evidence of stable demand or eventual recovery.

## Resource and trust bounds

Scenario validation runs before codec construction. The model caps slots,
channels, receivers, generations, live generations, retained originals,
frame size, received packets, decode work, reorder delay and peer/checkpoint
bytes. Exact limits are serialized with the campaign's selected profile.
Authentication checks precede decoder allocation and decode work. Duplicate,
unknown-generation, expired, malformed, unauthenticated and over-budget inputs
have distinct observations. These host bounds do not qualify an MCU heap,
interrupt budget or radio coexistence profile.

## Checkpoint and process contract

A checkpoint retains the next slot, transmitter cursors and originals,
round-robin position, pending deliveries, receiver manifests and collected
signed packets, counters and trace state. Codec structures are reconstructed
from validated bounded state. Replay work is reported separately from the
original run's decode work. Resume begins at the saved next slot.

The JSON process interface is a bounded test harness, not a radio transport.
It supports run, checkpoint and resume operations. The retained finite-burst
experiment serializes and restores a real partial checkpoint in process, then
compares terminal and trace hashes with the uninterrupted run. A separate small
manual smoke used three fresh peer processes, restored seven pending deliveries
at next slot six, and matched the normalized report and hashes; that observation
was stdout-only and has no retained receipt. The frontier report accurately
records its own process-resume scope as unexercised. Broad restart qualification
is deferred with the other stable-contract evaluation.

## Experiments and publication

Human-authored TOML scenarios declare experiment inputs. A machine-written
JSON report retains their assumptions, source identity, trace digests,
reconstruction observations and capacity bounds. Compare single-/multi-radio
collection, positive-/zero-floor scheduling and finite bursts/continued
overload. A partial-state resume can expose retention/state design mistakes.
Incomplete and expired delivery remain visible. These comparisons guide the
next implementation choice; they do not freeze the interim design into a
conformance or regression contract. Keep smoke checks small and postpone broad
qualification until the stack's shape is stable.

The selected [exploration report](../testing/transport-conformance/results/retained-repair-exploration-20261011.json)
records four experiments on macOS aarch64. These observations describe the
checked-in inputs, not an invariant every future scheduler must preserve:

| Design question | Current observation |
|---|---|
| Finite production: can one radio catch up? | Four radios reconstruct the 4,097- and 1,281-byte objects at slots 9 and 33; one radio at 64 and 53. Both finish with zero unmet bytes within the 160-slot lifetime. |
| Does fresh priority starve retained work? | A receiver joining at slot 8 gets zero symbols of the older object with floor zero. Floor one supplies 11 unique symbols and reconstructs it at slot 43. |
| What does the floor cost? | The paired 96-slot experiments transmit the same 364 data frames and 103,305 wire bytes. Fresh frames change from 345 to 257; repairs from 19 to 107. |
| Can repairs overcome continued overload? | 1,024 offered source symbols exceed a one-radio ideal upper bound of 256 opportunities. All 131,072 produced object bytes remain unmet for that receiver, including 106,496 bytes expired incomplete. Four radios complete eight objects (32,768 bytes), leaving 98,304 unmet bytes. |
| Does saved partial state resume? | The finite checkpoint after slot 25 restores matching terminal and trace hashes; rebuilding codec state requires 57 separately counted replay calls. |

The early shape is a shared retained-object sender with priority and repair
policy, explicit receiver opportunities, and authenticated generation envelopes.
The next useful prototype is a small owner-local receive adapter that passes
one recovered signed object to the existing operation processor. That will
expose the delivery/admission seam before expanding carriers or qualification.

Run and record the campaign locally. GitHub Actions is disabled in Mere and
Retinue by owner request; this work uses direct commits and pushes to `main`,
without workflow additions or PRs.

Primary specifications: [RFC 6330](https://www.rfc-editor.org/rfc/rfc6330),
[verified erratum 5548](https://www.rfc-editor.org/errata/eid5548),
[RFC 8032](https://www.rfc-editor.org/rfc/rfc8032), and
[RFC 8949 core deterministic encoding](https://www.rfc-editor.org/rfc/rfc8949#section-4.2.1).
