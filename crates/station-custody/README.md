# Station custody

This crate is Murm's Persona-backed station custody adapter, consumed by the
Signalman app. It obtains the operator identity from Castellan/Personae, derives a station-scoped Reticulum identity,
binds its Ed25519 half to a narrow, expiring RemoteAuth grant, and hands the
typed identity to Retinue's host library. The grant permits only
`transport.egress`, has no persona or private-read authority, and fails closed
at expiry. Retinue itself owns no Mere, Personae, or Castellan dependency.

Grant, renewal, revocation, and acknowledgement travel as bounded signed
control bodies inside normal authenticated LXMF messages. The Persona signs
each control frame through a device-specific attested child; the station signs
the acknowledgement with its derived Reticulum identity. A receiver accepts an
exact retransmission after a lost acknowledgement, but accepts a replacement
only before its prior deadline and only when the expiry grows. The receiver's
snapshot is non-secret state to persist beside the device's delegated identity.
`SitedStationHead` seals both through a caller-provided `SealedRecordStorage`,
persists an accepted transition before returning its acknowledgement, and
checks the receiver before every public radio operation. The process that
selects a board's secure-storage root remains deployment-owned. The
`station-provision` command creates that sealed record without exposing
the derived private identity:

```text
station-provision \
  --app-endpoint ENDPOINT \
  --station-root PATH \
  --record RELATIVE_PATH \
  --label NAME \
  --expires-hours HOURS
```

The command requires an explicit finite grant duration, installs the signed
grant before the radio opens, verifies the station-signed acknowledgement, and
refuses to replace an existing record. It prints only public receipt fields.
The physical radio receipt remains deployment work.

For a physically witnessed V4 first-owner claim, initialize the separate
controller scope explicitly, after the command has read the existing wallet
without migrating its sealed or legacy seed record:

```text
station-claim init \
  --authority-root PATH
```

`init` refuses a missing or locked wallet, creates
`first-owner-controller-id.json` exactly once with create-new semantics, and
prints only the resulting public controller fingerprint. It never opens USB.
Its current create-new final-file write is not atomic: if interruption leaves
malformed public scope JSON, `claim` fails closed. Before any board claim, the
operator may remove only that public scope file and rerun `init`.
The later `claim` action requires that existing scope and also reads the
wallet without creating an unlock root, wallet, or identity:

```text
station-claim claim \
  --authority-root PATH \
  --port COM6 \
  --region us915 \
  --frequency-hz 906875000 \
  --bandwidth-hz 250000 \
  --tx-power-dbm 17
```

The `claim` invocation itself is a durable board mutation after a fresh
physical presence window. It is not run automatically by this adapter or its
tests. Initializing a controller scope is not a board claim, and no physical
Claim or Resume receipt exists yet.

After a claim, `status` asks the running board for its control status under
the same controller identity, over the ordinary USB modem stream, with no
button gesture:

```text
station-claim status \
  --authority-root PATH \
  --port COM6 \
  --node NODE_HEX
```

The board verifies the signed command against its durable grant and journals
the outer replay counter before it answers, so `status` keeps that counter in
`first-owner-controller-counter.json` beside the scope record and advances it,
with a synced temporary file and rename, before every send. A counter the
board has already accepted is refused with silence; the command then reports a
carrier timeout. The record starts at one for a freshly claimed board and is
never rewound by this adapter.

`lifecycle` runs the controller's configuration lifecycle in one invocation
(wall_node_management D17, D18): signed Status, a ProvisionalApply of a
candidate built from the arguments, signed Status again as evidence, then
exactly one of commit, revert, or a wait past the board's deadline followed by
a Status read:

```text
station-claim lifecycle \
  --authority-root PATH --port COM6 --node NODE_HEX \
  --region us915 --frequency-hz 906875000 --bandwidth-hz 250000 \
  --spreading-factor 11 --coding-rate 5 --preamble-symbols 16 \
  --sync-word 0x2b --tx-power-dbm 17 \
  --relay-announces on --relay-packets on --max-hops 8 \
  --lifetime-ms 60000 --then commit|revert|expire --record RUN.json
```

The board-minted commit token stays in this process's memory and is zeroed
when the run ends; it is never printed, written or recorded. The outer counter
(`first-owner-controller-counter.json`) and the mutation sequence
(`first-owner-controller-sequence.json`) are each spent, with a synced
temporary file and rename, before the frame carrying them is sent; status
spends a counter only. The run record, refused if it already exists, carries
the change id, every counter and sequence used, the generations, the board
deadline, the configuration sent and the outcome, and no secret. The board's
answers carry no applied configuration and signed Status carries no radio
fields. A silent board is reported with what to check: the scope, a counter
record behind the board, a radio that was mid-frame, and for an apply a
profile the board cannot run.

This private integration package has its own workspace, excluded from Murm's
default radio workspace. Its radio dependencies are paths within this repository;
Signalman's desktop uses the same package by path. Dramatis/Personae and Djinn
retain credential derivation, grant policy and wallet custody. The existing
`mere/...` derivation namespaces and stored control formats remain byte-stable.
`source-import.json` records the twelve unchanged Rust files imported from Mere.

Check the adapter without starting a station:

```text
cargo test --manifest-path crates/station-custody/Cargo.toml --all-targets --locked
```

The ignored DR-C broker fixture is driven by
`mere/scripts/dr_c_receipts.py`; its absent/Locked cases remain a separate gate.
The portable boundary is `postilion::StationConfig`, which accepts a supplied
`PrivateIdentity`; this port is its first real Persona-backed consumer.

Linkboy remains below this port. Its package policy, immutable plans,
execution, recovery, and rescue CLI do not depend on Signalman or Mere.
