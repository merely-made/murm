# Retained-repair design experiments

These inputs explore the current prototype's scheduling choices. Run from the
`testing/transport-conformance` directory:

```sh
cargo run --locked --offline -- --simulate
```

The JSON output preserves each scenario, observations, frame/byte accounting,
cumulative demand timeline, terminal hash and receiver action trace hash. The
four experiments answer three questions:

- **Finite burst:** one and four independently tuned radios collect two objects,
  followed by a long production tail. Compare their actual RaptorQ completion
  slots and remaining undelivered bytes.
- **Repair floor:** a four-radio receiver joins after an older object's original
  burst. Four large new objects keep fresh demand available. Compare a disabled
  floor with one reserved repair lane, including the old object's progress and
  the number of fresh frames displaced by repairs.
- **Sustained overload:** publish 32-symbol objects every eight slots. That offers
  four source symbols per slot against a one-radio ceiling of one collected
  symbol per slot. Expiry keeps retained state bounded; cumulative produced
  minus completed bytes preserves undelivered work after expiry.

The finite experiment also records an in-process resume from a serialized
partial-run snapshot. It reports terminal and trace equality as a design
observation. The standalone peer exposes saved-state resume for further
exploration; this campaign does not run a fresh-process qualification matrix.

Edit `retained-repair-v1.toml` and rerun to change the prototype knobs. It is
embedded in the executable, so Cargo rebuilds when these inputs change. Useful
first changes are the repair floor, object publication interval, retention
window, manifest interval, radio count and tuning phase. Keep namespace and
object IDs equal when comparing scheduling variants.

To explore a custom scenario without changing the embedded inputs, send a JSON
request to the peer:

```sh
cargo run --locked --offline -- --simulation-peer <<'JSON'
{"op":"run","scenario":{"schema":"transport-conformance.simulation/v1","id":"custom-burst","namespace":1001,"slots":128,"channels":4,"manifest_interval":7,"repair_floor":1,"generations":[{"id":1,"publish_slot":0,"expires_slot":128,"object_octets":4097}],"receivers":[{"id":"one","radios":1},{"id":"four","radios":4}]}}
JSON
```

`op="checkpoint"` takes the same `scenario` and `after_slot`. Its returned
checkpoint can be passed unchanged in an `op="resume"` request. The snapshot
contains scheduler cursors, retained originals, receiver collection and pending
deliveries, and carries a signature. Resume reconstructs codec state from the
saved collection and reports that work separately.

A slot has no assigned physical duration. Each channel can send at most one
bounded authenticated frame per slot; each independently tuned radio can capture
one channel. Manifest repeats, signatures and wrapper bytes count toward the
recorded budget. Four receivers/radios on the same tuned channel would not gain
four independent opportunities in this model. Partial-radio receivers know the
manifest cadence and anchor on channel zero at that cadence; between anchors
they hop through contiguous channel windows. This assumes an acquired channel
plan and slot clock. Beacon discovery and physical clock acquisition still
need a design. The finite input uses a seven-slot manifest period against four
channels; drift and tuning knobs can explore missed anchor opportunities.

The core also exposes bounded sleep, join, retune, drift, seeded loss, burst loss,
reordering, duplicate, tamper and malformed-frame knobs. These are available for
subsequent design exploration. Their presence does not qualify radio hardware
or establish standards conformance. A later stable shape can supply the broader
conformance and regression set.

The manifest and symbol envelope are an authored profile. Their narrow fixed
CBOR arrays use unsigned integers and byte strings, shortest argument widths and
definite lengths, following [RFC8949 core deterministic encoding](https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2.1).
Array layouts avoid map ordering; rejecting other forms is this profile's
choice. The packet body retains the [RFC6330 payload ID and OTI formats](https://www.rfc-editor.org/rfc/rfc6330.html#section-3),
while retention, repair scheduling, channel opportunities and authenticated
wrapping remain prototype decisions.
