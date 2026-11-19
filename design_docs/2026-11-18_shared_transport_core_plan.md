# One shared transport core for Node and Endpoint

**Status, 2026-11-19: T0, T1 and T2 done and on `main`.** T2 is `be5a388`,
`7a2a36c` and `b4cf3b7` (fast-forwarded from the lane branch). Row 1 moved to
T3 (D13). T3 has not started and begins with row 1's fork.

**For:** the owner, 2026-11-18: "reducing duplicated code sounds good to me."

**Evidence:** [Retinue small plan, Findings (2026-11-18)](2026-11-02_retinue_small_plan.md#findings-2026-11-18-transport-routing-is-written-twice).
That section holds the size of the duplication and the table of 13 behavioural
differences, numbered as they are cited here.

## Purpose

`Node` (the board driver) and `Endpoint` (the desktop driver) each implement
transport routing: the route table, the reverse table, link bridges, type-2
forwarding, the announce relay gate and the packet filter. About 900 node lines
mirror about 970 endpoint lines, and the two copies behave differently in 13
known places. Structural decision 1 of the
[original small-node plan](archive_docs/2026-11-02/2026-07-31_retinue_small_plan.md)
requires that both "exercise one algorithm", so that the desktop is an oracle
for the board. This plan restores that for transport.

The target is the shape `crate::rebroadcast` already has: a clock-free type that
takes `now` and returns a decision, with each driver owning its locks, queues
and sends.

## Decisions

### shared_transport_core D1 — one core, differences first

Question (2026-11-18): write a plan for one shared transport core, with settling
the 13 differences against stock RNS as its first phase, or only settle the
differences for now? Answer: "The former".

Links: area retinue transport · draws on retinue_small structural decision 1 · plans T0–T5 · summary one shared transport core; the differences are settled before any code moves

### shared_transport_core D2 — where a relayed announce leaves

Difference 1: `Node` sends a relayed announce only on the interface it arrived
on; `Endpoint` sends it on every permitted interface.

The owner, 2026-11-18: "I suspect it was for keeping the meshtastic, meshcore,
and reticulum meshes separate, not shouting on each simultaneously. Plus that's
how the resident executive notion was supposed to work ... I don't think that's
a bad approach but it doesn't really seem as relevant now; that question has
moved up the stack from the embedded protocol handling". He asked whether there
is an argument for a different approach.

What the history shows: the rule dates from `4d5336c` (2026-10-09, review item
39 of the [RNS 1.5.7 comparative review](2026-10-07_rns_157_comparative_review.md)).
That change added relaying on the ingress interface so that a single-radio
repeater relays at all. Its message says the endpoint "relays on every permitted
interface including the ingress one". It gives no reason for the node stopping
at the ingress interface, and names no other mesh.

Proposed: the node follows the endpoint. Interface modes are Reticulum's own
control over which interfaces hear a relayed announce, and both drivers already
share `iface_mode::announce_permitted`.

Answer (2026-11-18): "I do agree with the different approach". The node relays
on every permitted interface, as the endpoint does. T0 still records the stock
reference for row 1; if stock disagrees, the row comes back as a question.

Links: area retinue transport · plans T0, T1 · summary Node relays announces on every permitted interface, as Endpoint does

### T0 rulings, D3 to D12 (2026-11-18)

Ten rows of the T0 table had more than one defensible answer. The owner chose
among named options; each answer below is the option's name as chosen. Row
numbers refer to the T0 table under Findings.

- **D3, row 2, relay with no live route:** "Require a live route". A relay is
  refused when its destination has no live route at release, as stock and the
  endpoint do. The node's arrival-interface fallback goes.
- **D4, row 4, when a carried link is refreshed:** "On accepted send". The
  node's rule. Admission alone does not keep a bridge slot.
- **D5, row 5, unroutable packets:** "Remember, then look up". Stock's and the
  endpoint's order. A new miss is still counted, apart from a filtered copy.
- **D6, row 3, which packet kinds skip the duplicate filter:** the owner first
  asked "is it needed?". Checked: both drivers already call one shared function
  (`node/tables.rs:143`), the channel rule is the earlier own-echo ruling
  covered by `tests/endpoint_link_echo.rs`, and stock re-encrypts a resend
  (`Packet.py:324`), so real retries are not blocked. Answer: "Keep it as is".
  Only the endpoint's carried-link path changes, by gaining the filter call.
- **D7, row 6, reverse proofs:** "Keep header, apply our ceiling". The endpoint
  stops rewriting the header; the node gains Retinue's hop ceiling.
- **D8, row 7, order of checks:** "Transit first, one outcome". The endpoint's
  order. A packet is handled once.
- **D9, row 8, return path and route touch around a send:** "Record first, undo
  on refusal". Record the return path before the send, remove it if the send is
  refused, and touch the route only once accepted.
- **D10, row 9, duplicate memory:** "Explicit profiles". Size, time limit and a
  separate announce memory are parameters. The board keeps its 60 s rule.
- **D11, row 11, a link request with too few key bytes:** "Carry it like
  stock". Derive its id, record the bridge and forward it; the destination
  refuses it. This was not the recommended option.
- **D12, row 13, removing an interface:** "Clear everything tied to it". One
  removal step drops return paths and pending relays bound to the interface.
  As put: "A relay whose destination is still reachable another way is lost
  until the next announce." The option not taken was stock's shape, where a
  pending relay still leaves by another live route. So a pending relay is
  bound to the interface it was heard on, and is purged with it.

Links: area retinue transport · draws on T0 results · plans T1 · summary ten T0 rows ruled; nine as recommended, row 11 follows stock

### T1 fork rulings, D14 and D15 (2026-11-18)

The T1 lane set rows 5 and 8 aside as forks. Each answer is the option's name
as chosen.

- **D14, row 5, refines D5:** the endpoint's public `RoutingCounters` has no
  counter for a route miss (`endpoint/routing.rs:113`), so "a new miss is
  counted once, a repeat is not" could not be asserted there. Options: add the
  counter, or keep existing counters (the lane's recommendation). Answer: "Add
  the desktop counter". The endpoint gains `unroutable_packets`, and both
  drivers assert the same rule.
- **D15, row 8, refines D9:** recording a return path in a full table evicts
  the oldest entry, and re-recording replaces one (`node/transit.rs:271`,
  `endpoint/transit.rs:122`). Options: on refusal, put back what was
  displaced, or remove the new entry only. Answer: "Put back what was
  displaced". A refused send leaves the return-path table as it was.

Links: area retinue transport · refines D5, D9 · plans T1 · summary the endpoint counts route misses; a refused send restores displaced return paths

### shared_transport_core D13 — row 1 lands in T3

Refines D2. The T1 lane found that row 1 needs Node to know its interfaces.
`Node::poll` takes one interface (`node/poll.rs:32`), Node keeps no list of
them (`set_interface_mode` stores only non-default modes, `node/routes.rs:92`),
and one call returns at most `ACTIONS` actions (8 on the boards,
`node/action.rs:66`). Every caller in the tree passes one interface
(`RADIO = 0`, `radio-hand/src/channel/node.rs:31`), so the old and new rules
send the same packets on today's boards.

Options put (2026-11-18): move it to T3, where the interface list becomes an
input of the shared core; add a bounded interface registry to Node now (the
lane's recommendation); or pass the list at each poll. Answer: "Move it to T3".

D2 stands. Row 1 stays open until T3; T1 leaves Node's relay on the arrival
interface.

Links: area retinue transport · refines D2 · plans T1, T3 · summary Node's interface list is designed once, as a T3 input; row 1 is not a T1 change

### T2 storage rulings, D16 to D18 (2026-11-19)

Put with the numbers in "T2 storage measurement" and the two readings the
inventory left open. Each answer is the option's name as chosen.

- **D16, how one implementation serves 16 and 4096 entries:** options were a
  storage trait with the per-packet sequence written once; one
  open-addressing table for both drivers (fastest, twice the slots, needs a
  keyed hash or a library table); or shared entry types and decision
  functions with each driver writing the sequence itself. A linear array was
  not offered: 41,945 ns per type-2 packet at 4096 against 323 ns today.
  Answer: "Storage trait". The trait is small (get, insert, remove,
  evict-least); the board implements it over arrays and the desktop over
  `HashMap`; a faster container can be added later as one more
  implementation.
- **D17, what "packet filter" covers in T2:** the transit duplicate memory, or
  also the link-data echo windows (`LinkPacketMemory` on the desktop,
  `sent_link_data` and `received_link_data` on the board), which serve links
  the node itself terminates. Answer: "Transit memory only". The echo windows
  stay with each driver's link code and are a follow-on.
- **D18, expiry:** the board sweeps its tables before every packet; the
  desktop checks age on lookup and never sweeps. Options: lookups check age
  and each driver decides when to sweep; the same with the board sweeping on
  poll only; or both drivers sweep before every packet (about 7 µs per packet
  at 4096). Answer: "Check on lookup, driver sweeps". Shared lookups check age
  themselves, the core offers a sweep call, and the board keeps calling it
  before every packet and on poll, so the board's behaviour and counters do
  not change.

Links: area retinue transport · draws on T2 inventory, T2 storage measurement · plans T2 · summary a storage trait with the sequence written once; transit memory only; age checked on lookup, sweep scheduled by the driver

### T2 stage 1 fork rulings, D19 and D20 (2026-11-19)

The T2 lane committed the shared module alone (`be5a388`, no driver changed)
and set two choices aside. Each answer is the option's name as chosen.

- **D19, the shape of the shared entries:** the board keeps the key inside
  each entry with `u64` millisecond times; the desktop keeps the key as the
  map key with `Instant` times. Existing tests build these entries directly,
  so one side's tests change in how they build or reach an entry whichever
  shape is shared (about 15 board lines, or about 14 desktop lines, or about
  27 desktop lines plus the board's for one concrete `u64` type). Options: key
  beside the entry with generic time (the lane's build); key inside the entry;
  one concrete entry in `u64` milliseconds, as `crate::rebroadcast` has.
  Answer: "Key beside entry, generic time". Entries carry no key and are
  generic over the driver's time. Test lines that build or reach an entry may
  change; no assertion may.
- **D20, where a route's lifetime comes from:** `InterfaceMode::route_ttl`
  lives in `node/params.rs`, which needs `alloc`, and the shared tables must
  build without it. Options: each route operation takes a lifetime function
  from the driver; or `route_ttl` and its constants move to `iface_mode.rs`.
  Answer: "Driver passes a function". Nothing moves and no public item
  appears in a new feature set.

Links: area retinue transport · refines D16 · plans T2 · summary shared entries carry no key and are generic over time; the driver passes the route lifetime

## Phases and done-conditions

### T0. Settle the differences

For each of the 13 rows:

- read both sides in full, including the rows marked "reported" and "suspected";
- find what stock RNS 1.6.0 does, pinned to the qualified wheel, with file and
  line;
- class the row as board right, desktop right, both wrong, or an intended
  difference that becomes a parameter.

Also read the seven endpoint files the reviewer pass skipped, and add any new
difference to the table.

Read how rns-rs (`663e5e3`) shapes its transport engine's interface to its
driver: capacity, time and output. This is for design only (licence adoption
record, 2026-11-18: learn first, no wholesale adoption). Record what was read.

**Done when:** every row carries a stock reference and a class; each row with
more than one defensible answer has come back as a question and has a ruling;
D2 is ruled.

### T1. Converge behaviour in place

Change whichever driver is wrong for each settled row, one row per change. Each
change starts from a failing test on the wrong side. Row 1 is not a T1 change
(D13).

**Done when:** for every settled row, `node/tests/` and `endpoint/tests/` assert
the same rule and pass; the live suite and `peer_rnsrs.py` pass; the T114 and V4
images build.

### T2. Share the tables

Move the route entry, link bridge, reverse entry and packet filter into one
clock-free module that both drivers use.

**Fork, returned and ruled (D16 to D18):** how one implementation serves 16
entries on the board and 4096 on the desktop. A storage trait, with the
per-packet sequence written once against it; the packet filter here is the
transit duplicate memory only; lookups check age and each driver schedules its
own sweep.

**Done when:** `LinkBridge`, `ReverseEntry`, the route entry and the packet
filter are each defined once; the allocation-free floor and the `alloc`-off
build still build.

### T3. Share the forwarding decisions

Move bridge admission, type-2 forwarding, reverse-proof handling and the
announce relay gate into the shared module as functions that return a decision.
Drivers keep locking, queues, `Actions`, statistics and policy selection.

Row 1 lands here (D13): the core takes the list of attached, permitted
interfaces as an input, and Node gains whatever it needs to supply that list
and to carry unfinished sends across polls. That API comes back as a fork
before code.

**Done when:** neither `node/` nor `endpoint/` decides whether or where a
carried packet goes; each only supplies inputs and carries out the decision.

### T4. Test the shared core once

One behaviour suite runs against the shared module at the board and desktop
capacities. Driver tests keep only what a driver owns.

**Done when:** each behaviour from the mirrored test files is asserted in one
place; the count of tests removed and kept is recorded here.

### T5. Qualify

**Done when:** the Rust suite, the live suite and `peer_rnsrs.py` pass at the
final revision; the firmware images build, and their flash and RAM figures are
recorded beside the figures from before T2.

## Constraints

- No behaviour changes in T2 or T3. A difference found there goes back to T0.
- The board path stays `no_std`, bounded and clock-free, and takes `now` from
  its caller.
- Endpoint-only features stay in the endpoint: path discovery, held announces,
  gravity, routing selectors, the link-MTU clamp and tunnels.
- Stock RNS source may be read and adapted under the
  [licence adoption record](2026-10-05_reticulum_license_adoption.md); each
  adaptation goes in `NOTICE`.

## Deferred and follow-on

- Whether the board gains path discovery and transport answers, which today
  only the endpoint has.
- The interop gate shape (one script and one example program per gate) and the
  receipt layer. Both were measured on 2026-11-18 and are outside this plan.
- The link-data echo windows (`LinkPacketMemory`, `sent_link_data`,
  `received_link_data`) are written twice and are not part of T2 (D17).
- The scan for the oldest entry on every insert into a full table, and the
  desktop's prune of lapsed bridges on every link request: 12 to 30 µs per
  packet at 4096. An eviction order kept beside the table would remove both.
  Not ruled; it is separate from the storage trait.
- The standalone Heltec V4 node
  ([wall node](2026-11-02_wall_node_management_plan.md) WN2, then WiFi in WN3).
  Asked on 2026-11-19 whether WN2 comes before this plan's next phases or after
  T3, the owner: "Probably after T3." A V4 with LoRa and WiFi is the first node
  with two interfaces, so WN3 is the first consumer of the interface list that
  row 1 needs (D13). Today's V4 image is a modem with no allocator and no
  network stack (`firmware/heltec-v4-phy/Cargo.toml`).

## Findings

- 2026-11-18: the size and the 13 differences are in the
  [small-node plan's Findings](2026-11-02_retinue_small_plan.md#findings-2026-11-18-transport-routing-is-written-twice).
  Three rows were confirmed by reading both sides (1, 3, 5); the rest are
  reported or suspected.
- 2026-11-18: the Rust suite is 774 tests in 34 seconds at `704a04f`. Run time
  is not the cost of the mirrored tests; upkeep is.

- 2026-11-18, candidate row 24, not ruled: carrying link traffic over a
  bridge, stock marks the path to the link's destination as used
  (`Transport.py:2437` to `2440`). Neither driver touches a route on that path
  (`node/transit.rs` and `endpoint/transit.rs` touch only on type-2
  forwarding). Seen while reviewing row 8; what stock uses that timestamp for
  was not read.

### T0 results (2026-11-18)

Checked the working tree on `main`, HEAD `704a04f4933496eef03f47871ce9904c729d6d03`.
Rust paths below start at `crates/retinue/src/`. Stock paths start at
`crates/retinue/oracle/.venv/Lib/site-packages/RNS/`, installed **RNS 1.6.0**
(`_version.py:1`, `rns-1.6.0.dist-info/METADATA:3` in the parent directory).
The wheel pin is `rns==1.6.0`, SHA256
`fcf2d07eebafea7e56a94542bd74646e451b0f33bb3efb5f914bbbba38b39f36`
(`oracle/requirements.txt:2`, relative to `crates/retinue/`); its installed
wheel tag is `py3-none-any` (`rns-1.6.0.dist-info/WHEEL:4`). The original
wheel file is not present in the oracle directory. No upstream source was
fetched, adapted or copied.

Each class concerns the difference named, not every behaviour of the enclosing
function. Ten rows were put to the owner and are ruled in D3 to D12; where a
rule cell below says "Proposed" or names a question, the decision governs. Other intended differences preserve the plan's existing driver
boundaries; a named input does not move that feature into the core. A successful
send below means an accepted action or queue handoff, not an on-air receipt.
Retinue uses wire hops; RNS adds the receiving hop at `Transport.py:2028`.

| # | Confirmed or corrected statement | Node `file:line` | Endpoint `file:line` | Stock `file:line` | Class | Shared-core rule |
|---|---|---|---|---|---|---|
| 1 | Confirmed: Node relays only on ingress; Endpoint uses every permitted interface, ingress included. | `node/rebroadcast.rs:113` | `endpoint/rebroadcast.rs:263` | `Transport.py:2594`, `Transport.py:1672`, `Transport.py:1675` | desktop right | D2 stands: enumerate attached, permitted egress interfaces. Stock's ordinary relay has no attached-interface restriction. |
| 2 | Confirmed: with no live route at release, Node substitutes ingress; Endpoint supplies no source mode and refuses the relay. | `node/rebroadcast.rs:114` | `endpoint/rebroadcast.rs:224`, `endpoint/rebroadcast.rs:260`; `iface_mode.rs:74` | `Transport.py:1688`, `Transport.py:1694` | desktop right | Proposed: refuse an ordinary relay without a live source route. Q2 leaves the bounded-table fallback open. |
| 3 | Confirmed: Node filters admitted bridge packets; Endpoint's bridge path and `forward_on` do not consult a duplicate memory. | `node/transit.rs:126`, `node/transit.rs:152`; `node/tables.rs:143` | `endpoint/router.rs:108`, `endpoint/router.rs:138`; `endpoint/transit.rs:324` | `Transport.py:1851`, `Transport.py:2227`, `Transport.py:2426` | board right | Filter after bridge admission, before output; never let a third interface poison the memory. Context exemptions remain Q3. |
| 4 | Confirmed: Endpoint touches `seen` before transit policy, hop and output admission; Node touches it only after accepting output. Stock refreshes ordinary bridge data after transmit; LRPROOF marks validation without touching its timestamp. | `node/transit.rs:163`, `node/transit.rs:170` | `endpoint/router.rs:115`, `endpoint/router.rs:126`; `endpoint/transit.rs:337` | `Transport.py:2435`, `Transport.py:2438`, `Transport.py:2934` | board right | Proposed: refresh on accepted output. Stock's transmit call has no acceptance result. Proof-only refresh is also part of Q4. |
| 5 | Confirmed: Node looks up a route before hashing and counts an unroutable copy each time; Endpoint remembers the hash first and has no unroutable count here. | `node/transit.rs:197`, `node/transit.rs:203`, `node/transit.rs:209` | `endpoint/transit.rs:189`, `endpoint/transit.rs:195` | `Transport.py:2234`, `Transport.py:2295`, `Transport.py:2389` | desktop right | Proposed: remember an eligible type-2 packet before route lookup; distinguish a new miss from a filtered copy. Q5. |
| 6 | Confirmed: Node increments a reverse proof's hops without a transit ceiling or header rewrite; Endpoint checks the ceiling and clears its type-2 header/transport ID. Stock has an ingress ceiling but preserves the header when carrying the proof. | `node/transit.rs:301`, `node/transit.rs:323` | `endpoint/router.rs:317`; `endpoint/transit.rs:337`, `endpoint/transit.rs:349` | `Transport.py:2022`, `Transport.py:2028`, `Transport.py:3013` | both wrong | Proposed: keep the proof's header and enforce Retinue's configured early forwarding ceiling on this path too. Literal stock boundary behaviour is Q6. |
| 7 | Confirmed: Node checks ordinary reverse proofs before bridge/type-2 transit; Endpoint checks them after transit and returns after handling transit. Stock checks type-2, then link transport, then reverse proofs; type-2 can fall through. | `node/ingest.rs:81` | `endpoint/router.rs:99`, `endpoint/router.rs:148`, `endpoint/router.rs:313` | `Transport.py:2292`, `Transport.py:2397`, `Transport.py:2444`, `Transport.py:3009` | desktop right | Proposed: reverse proofs come after transit classification. Multiple matching tables and stock's fall-through need Q7, not an assumed equivalence. |
| 8 | Confirmed: Node remembers the reverse path and touches the route after output acceptance; Endpoint does both before, and does not undo the reverse entry on refusal. Stock records reverse before transmit, touches path after the attempt. | `node/transit.rs:254`, `node/transit.rs:259`, `node/transit.rs:263` | `endpoint/transit.rs:225`, `endpoint/transit.rs:228`, `endpoint/transit.rs:230`; `endpoint/shared.rs:477` | `Transport.py:2383`, `Transport.py:2386`, `Transport.py:2387` | both wrong | Proposed: prepare return state before output can execute, undo it on refusal, touch the path after acceptance. Neither driver implements that whole transaction. Q8. |
| 9 | Confirmed: Node shares two 32-hash generations between transit and announce relay, rotating on capacity or 60 s; Endpoint uses two 32,768-hash generations without time ageing and a separate 4096-entry FIFO for announces. Stock uses two generations, no age limit, and permits repeated SINGLE announces through its packet filter. | `node/params.rs:272`, `node/params.rs:277`; `node/tables.rs:75`; `node/transit.rs:361` | `endpoint/dedup.rs:12`, `endpoint/dedup.rs:128`, `endpoint/dedup.rs:140`, `endpoint/dedup.rs:162` | `Transport.py:330`, `Transport.py:974`, `Transport.py:1897`; `Storage.py:182`, `Storage.py:207` | intended difference | Inputs: `packet_hash_capacity`, `announce_hash_capacity`, `dedup_max_age` and `separate_announce_memory`. Preserve explicit profiles if Q9 accepts them. Stock swaps above 500,000 current entries. |
| 10 | Confirmed: a gravity repoint bypasses Endpoint's relay hash check; held announces at wire hops 126 or more are refused. Node has no gravity override or held-announce path. The threshold matches stock after its receiving-hop increment. | `node/ingest.rs:129`, `node/ingest.rs:220` | `endpoint/announces.rs:108`, `endpoint/announces.rs:373`, `endpoint/announces.rs:441`; `endpoint/paths.rs:150` | `Transport.py:1897`, `Transport.py:2524`; `Interfaces/Interface.py:275` | intended difference | Inputs: `gravity_repointed`, `held_admission`. These remain endpoint-owned features; the core must not reject an admitted gravity update merely because its packet hash repeats. |
| 11 | **Corrected:** both forward a request with fewer than 64 key bytes without creating a bridge, if other gates pass. Node's clamp is a no-op on such a payload, and failed `link_id` only skips bridge insertion. Stock derives a hash even for a short request and records a bridge; its destination rejects invalid request lengths. | `node/transit.rs:221`, `node/transit.rs:236`, `node/transit.rs:254`; `link/wire.rs:134`, `link/wire.rs:175` | `endpoint/transit.rs:208`, `endpoint/transit.rs:230` | `Link.py:421`, `Transport.py:2374`, `Link.py:188` | both wrong | D11: carry it as stock does. Derive the id, record the bridge and forward; the destination refuses it. |
| 12 | **Corrected:** both have size refusals. Node checks its logical MTU before emitting an action; Endpoint checks the final encoded packet plus access-code overhead in `Iface::push`. Endpoint also clamps request MTU using both interfaces and strips signalling toward a protocol-only interface. | `node/transit.rs:159`, `node/transit.rs:221`, `node/transit.rs:230`, `node/transit.rs:325` | `endpoint/transit.rs:260`; `endpoint/shared.rs:459`; `endpoint/interface.rs:225`; `endpoint/mtu.rs:87` | `Transport.py:2017`, `Transport.py:2338`; `Packet.py:238` | intended difference | Input: `output_limits` with logical/egress MTU and signalling support. Drivers enforce final wrapped frame limits. A common 500-byte cap would break negotiated larger links. |
| 13 | **Corrected scope:** ordinary Node forget and Endpoint detach retain reverse entries and pending relays/caps. Endpoint's shared-instance-loss path explicitly purges them. Stock removes ordinary reverse entries in housekeeping when either interface disappears; pending relay output is rechecked against a source route. | `node/routes.rs:120` | `endpoint/attach.rs:117`; `endpoint/local.rs:160`; `endpoint/rebroadcast.rs:114`; `endpoint/transit.rs:164` | `Transport.py:1027`, `Transport.py:1251`, `Transport.py:1694`, `Transport.py:4195` | both wrong | Proposed: one interface-removal input purges reverse entries and interface-bound pending work. Whether an ordinary relay may survive by another live route is Q13. |
| 14 | New: Endpoint holds verified unknown-route announces during ingress bursts, replacing by destination and releasing nearest first, and can block destination-rate relay while still learning. Node learns and schedules directly, without these admission ledgers. | `node/ingest.rs:147`, `node/ingest.rs:201`, `node/ingest.rs:220` | `endpoint/announces.rs:224`, `endpoint/announces.rs:420`; `endpoint/announces/held.rs:39`, `endpoint/announces/held.rs:65` | `Transport.py:2046`, `Transport.py:2056`, `Transport.py:2583`; `Interfaces/Interface.py:274`, `Interfaces/Interface.py:282` | intended difference | Input: `announce_admission` supplying hold/release and relay permission. Holding and rate ledgers stay outside the shared transport core; a relay refusal must not undo route learning. |
| 15 | New: Endpoint measures incoming/outgoing path-request rates and withholds recursive work under burst/egress control. Node deduplicates tags and answers only for itself, without a rate ledger or recursive search. | `node/ingest.rs:236` | `endpoint/discovery/rates.rs:86`, `endpoint/discovery/rates.rs:134`; `endpoint/discovery.rs:208`, `endpoint/discovery.rs:368` | `Interfaces/Interface.py:218`, `Interfaces/Interface.py:245`; `Transport.py:2131`, `Transport.py:3942` | intended difference | Inputs: `path_discovery_enabled`, `path_request_admission`. Discovery and rate sampling remain driver-owned; the board does not gain recursive discovery in this plan. |
| 16 | New: Endpoint has ingress/egress selectors, outgoing permission, INTERNAL flags, bitrate and cap-share overrides. Node supplies global relay flags, interface mode/airtime, default INTERNAL flags and a fixed 2% cap. | `node/params.rs:298`, `node/params.rs:322`; `node/rebroadcast.rs:117` | `endpoint/iface_policy.rs:73`; `endpoint/routing.rs:45`; `endpoint/shared.rs:399`; `endpoint/rebroadcast.rs:209` | `Transport.py:1676`, `Transport.py:1698`, `Transport.py:1706`, `Transport.py:1757`; `Interfaces/Interface.py:117` | intended difference | Input: `interface_policy` with explicit permissions, mode flags and cap rate. Defaults can represent the board's current policy. Both Retinue drivers cap all modes, unlike stock's final-branch-only cap at `Transport.py:1745`; this row does not claim full stock cap parity. |
| 17 | New: Endpoint authenticates tunnel synthesis, mirrors routes and restores eligible paths when the tunnel returns; Node has no synthesis interception or tunnel store. | `node/ingest.rs:53`, `node/ingest.rs:94` | `endpoint/router.rs:65`; `endpoint/tunnel.rs:221`, `endpoint/tunnel.rs:239`, `endpoint/tunnel.rs:321` | `Transport.py:3092`, `Transport.py:3099`, `Transport.py:3135` | intended difference | Input: `tunnel_restore` supplying an optional admitted route update. Tunnel identity, storage and reconnect handling remain endpoint-owned. |
| 18 | New: Endpoint drains four inbound lanes in priority order, waiting for space for stream data and dropping full control lanes. Node processes each caller-supplied packet synchronously and returns bounded actions. Stock prioritises the same classes, but drops every full lane. | `node/ingest.rs:18`; `node/action.rs:60`, `node/action.rs:79` | `endpoint/inbound_queue.rs:33`, `endpoint/inbound_queue.rs:150`, `endpoint/inbound_queue.rs:291` | `Transport.py:60`, `Transport.py:74`, `Transport.py:186`, `Transport.py:2165` | intended difference | Inputs: `ingress_limited` and a bounded `output_sink`. Drivers own queue order and backpressure; core decisions consume the supplied admission facts and report output refusal. |
| 19 | New: Node derives its relay delay from packet hash and node identity in a fixed 500 ms window; Endpoint draws fresh randomness within a configurable window. Stock draws fresh randomness in a 0.5 s window. | `node/rebroadcast.rs:33`; `node/params.rs:285` | `endpoint/rebroadcast.rs:52` | `Transport.py:169`, `Transport.py:2596` | intended difference | Input: `relay_delay_ms`, supplied by the driver. Scheduling consumes the bounded delay and `now`; the core does not own a clock or RNG. |
| 20 | New: Endpoint increments `forwarded_packets` before queue admission, including a refused/oversized/offline output; Node increments only after its action is accepted. Stock increments TX after the outgoing call returns without exception. | `node/transit.rs:163`, `node/transit.rs:171`, `node/transit.rs:254` | `endpoint/transit.rs:344`, `endpoint/transit.rs:351`; `endpoint/shared.rs:460` | `Transport.py:1552`, `Transport.py:1557` | board right | Report accepted output separately from attempted output and actual wire transmission. Drivers count their accepted handoff; the shared core returns the disposition. |
| 21 | New: Endpoint makes shared-instance ingress hop-transparent and removes the transport stamp from zero-hop client announces. Node treats every interface as an ordinary hop. | `node/ingest.rs:38`, `node/transit.rs:213` | `endpoint/router.rs:46`; `endpoint/local.rs:114` | `Transport.py:2211`, `Transport.py:2214`, `Transport.py:2273` | intended difference | Inputs: `local_hop_adjustment`, `local_client_announce`. Default to ordinary wire hops; shared-instance normalization happens exactly once before common route/relay decisions. |
| 22 | New: relaying an ordinary type-1 broadcast announce, Node adds a type-2 header and transport ID but preserves the broadcast propagation bit. Endpoint also sets the transport propagation bit. The packet encoder does not infer it from the header type. | `node/transit.rs:364`; `packet.rs:290` | `endpoint/announces.rs:458` | `Transport.py:940`, `Transport.py:944`, `Transport.py:945` | desktop right | Set the complete transport-announce header, including propagation, when constructing a relay. This is a wire-flag difference; no live failure is claimed. |
| 23 | New: a valid type-2 link request naming this node as transport and a local destination reaches Node's local accept path. Endpoint classifies it as transit and returns after a policy refusal or missing-route lookup, before local accept. Stock allows that request to reach a registered local destination. | `node/transit.rs:185`, `node/transit.rs:188`; `node/ingest.rs:223`; `node/links.rs:152` | `endpoint/router.rs:121`, `endpoint/router.rs:146`, `endpoint/router.rs:148`, `endpoint/router.rs:197` | `Transport.py:2295`, `Transport.py:2389`, `Transport.py:2813` | board right | Recognize a registered local destination before treating a packet naming this transport as carried traffic. Local admission remains separate from transit permission. |

Read all seven previously skipped files, including their in-file tests:
`endpoint/announces/held.rs`, `endpoint/discovery/rates.rs`,
`endpoint/iface_policy.rs`, `endpoint/mtu.rs`, `endpoint/tunnel.rs`,
`endpoint/inbound_queue.rs`, `endpoint/shared.rs`. Rows 14-18 record additional
behaviours exposed by these files; the MTU finding corrects row 12. Rows 19-23
come from following their callers and output paths. Row 13 is a scope correction,
not a claim that ordinary detach already cleans up.

### T2 inventory of the four tables (2026-11-19)

A read-only lane listed every operation on the route, link bridge, reverse and
duplicate-memory tables in both drivers at `5befe56`. Paths start at
`crates/retinue/src/`. Checked here: the three cost claims marked "checked".
The rest is the lane's reading.

| Table | Node | Endpoint |
|---|---|---|
| Routes | `BoundedVec<Route, ROUTES>` (`node.rs:86`), 16, key is a field, linear find | `Mutex<HashMap<AddressHash, PathEntry>>` (`shared.rs:176`), 4096; entry has no destination and no link cost |
| Link bridges | `BoundedVec<LinkBridge, ROUTES>` (`node.rs:94`), 16 | `Mutex<HashMap<AddressHash, LinkBridge>>` (`shared.rs:207`), 4096; entry has no link id |
| Reverse | `BoundedVec<ReverseEntry, ROUTES>` (`node.rs:97`), 16 | `Mutex<HashMap<AddressHash, ReverseEntry>>` (`shared.rs:210`), 4096 |
| Duplicate memory | `HashGenerations<32>` (`node/tables.rs:59`), rotates by count or 60 s | `HashList`, two `HashSet`s of 32,768 (`endpoint/dedup.rs:140`), no time; separate 4096 announce window |

Costs that grow with table size on a per-packet path:

- Node sweeps all three tables for expiry on every ingest and poll
  (`node/ingest.rs:81`, `node/routes.rs:138`; checked). Every non-announce
  packet also does a linear bridge find.
- Endpoint prunes lapsed bridges with a full `retain` on every carried link
  request, at any fill (`endpoint/transit.rs:329`; checked).
- Endpoint's reverse insert, when the table is full of live entries, does a
  full `retain` and a full minimum scan per carried type-2 packet, with the
  table lock held across the send (`endpoint/transit.rs:148`; checked).
- Endpoint's route insert at capacity does the same two scans
  (`endpoint/paths.rs:210`).
- For bridged data, Endpoint's `carry_on` looks the link id up in the path
  table, which cannot hit (`endpoint/transit.rs:407`).

What a shared implementation has to absorb or take as a parameter:

1. Time: `u64` milliseconds against `Instant`.
2. Key: inside the entry on the board, the map key on the desktop.
3. Route fields: the board's route carries `destination` and `link_cost`; the
   desktop keeps link cost in a separate bounded table, and `PathEntry` is
   also embedded in tunnel mirrors and route facts.
4. Capacity: one `ROUTES` constant bounds three board tables; the desktop has
   a constant per table. Only `crate::rebroadcast::Rebroadcasts` already takes
   a runtime capacity from both drivers (8 and 4096).
5. Expiry: eager sweep on the board, lazy on the desktop (on lookup, on take,
   or on insert at capacity; nothing sweeps). Board lookups rely on the sweep
   having run; desktop lookups check age themselves.
6. Eviction: the same policies, with different tie-breaking (vector order
   against map order), and the desktop expires before evicting.
7. Time limits: route and bridge lifetimes are configurable on the board; the
   desktop's bridge lifetime is a constant. `REVERSE_TIMEOUT` is defined twice
   with the same value.
8. Counters: the board counts expired and evicted routes and bridges and
   refused bridges; the desktop counts evicted paths and policy refusals.
9. Undo of a reverse entry (D15): the board relies on the new entry being
   last; the desktop keeps a list of displaced entries.
10. Lock order on the desktop: `reverse_table` is held while `path_table` is
    taken; `link_transport` is held while `address_book` is taken.

Two readings the lane could not settle, both since ruled (D17, D18):
whether "packet filter" in T2 means only the transit memory or also the
link-data echo windows (`sent_link_data`, `received_link_data`,
`LinkPacketMemory`); and whether the board's per-ingest sweep is behaviour to
keep or a driver choice.

Not determined: `size_of` of any entry, the locks `push_to` takes, and the
test-only operations.

### T2 storage measurement (2026-11-19)

A lane wrote a host benchmark, `crates/retinue/examples/transport_table_cost/`
(nine files, 2433 lines), committed as `04d72d3` on branch
`transport-core-t2-measure`. It is not on `main`. It models the four tables in
eight shapes with stand-in entry types, not the real drivers: today's desktop,
today's board, A1 (one bounded array type, linear scan), A2 (one bounded
open-addressing type, twice as many slots as entries), B (a storage trait with
the per-packet logic written once, over arrays and over `HashMap`) and C
(shared entry types and decision functions, each driver doing its own lookups,
over arrays and over `HashMap`). Four workloads, capacities 16 and 4096 (filter
generations of 32 and 32,768), half and full, five repetitions, 90% lookup
hits, every packet hash new. Locks, `Instant::now` and checks that touch no
table are left out of every shape. The harness and the candidate code were
read here. Every shape carried the same share of the same packets in every
cell, so they decide alike.

Median nanoseconds per carried packet at 4096, half full / full. These are
from a rerun of the committed program here, which was calmer than the lane's
run (4 of 128 rows spread more than 20% across repetitions, against most rows
at 16 in the lane's). The two runs agree on every ordering below.

| Shape | Bridged data | Type-2 data | Link request | Reverse proof |
|---|---|---|---|---|
| today's desktop | 195 / 213 | 323 / 20,484 | 11,892 / 29,905 | 85 / 88 |
| today's board | 40,642 / 57,713 | 49,078 / 72,692 | 50,252 / 75,880 | 9,284 / 19,699 |
| A1 linear | 34,763 / 44,979 | 41,945 / 62,726 | 44,019 / 63,912 | 2,625 / 5,589 |
| A2 open addressing | 52 / 63 | 84 / 11,988 | 82 / 18,151 | 34 / 60 |
| B over arrays | 35,205 / 45,806 | 43,329 / 63,115 | 43,093 / 64,639 | 2,587 / 5,388 |
| B over `HashMap` | 200 / 170 | 328 / 16,798 | 321 / 21,378 | 100 / 95 |
| C, board container | 33,143 / 44,431 | 40,860 / 60,896 | 40,915 / 60,619 | 2,508 / 4,956 |
| C, desktop container | 128 / 140 | 236 / 17,009 | 233 / 18,744 | 80 / 86 |

At 16, half full / full, for type-2 data: today's board 111 / 171, A1 90 / 135,
A2 49 / 134, B over arrays 93 / 143, C board 91 / 138, today's desktop
263 / 692, B over `HashMap` 207 / 546, C desktop 213 / 565. The build was the
workspace release profile (`opt-level = "s"`, LTO), on a host CPU.

What the numbers say:

- A linear array does not serve 4096. Its cost is the filter: up to 65,536
  hashes compared per packet, 33 to 65 µs. The three 4096-entry tables alone
  cost 2.5 to 5.6 µs (the reverse-proof column, which skips the filter).
- B and C cost the same over the same container, at both sizes. The trait is
  free.
- B and C over `HashMap` match today's desktop at 4096 (C is up to a third
  quicker), except on link requests, where today's desktop prunes every lapsed
  bridge on each request: 12 µs at half fill.
- Every hashed shape, A2 included, pays 12 to 30 µs per insert into a full
  table: the scan for the oldest entry. Neither that nor the prune is a
  storage question; an eviction order kept alongside the table would remove
  both.
- A2 is the fastest shape at both sizes: 2.5 to 4 times B over `HashMap` at
  4096 when half full, and up to twice the arrays at 16 when half full.
- At 16 the candidates over arrays are a fifth to a quarter quicker than
  today's board, which sweeps all three tables before every packet.

Memory: the array shapes hold the four tables inline, 3.5 kB at 16 and 1.67 MB
at 4096. A2 holds twice that. The `HashMap` shapes hold 48 to 104 bytes inline
and an estimated 3.0 to 3.4 MB on the heap when full.

How much is written once: B's per-packet sequence is about 130 lines, once,
plus about 25 lines per container. C shares 102 lines of decision functions
and writes the sequence in each driver, about 100 lines on the board and 140
on the desktop.

The array shapes, A2, B's trait with its logic, and C's decision functions use
only `core`; the lane compiled them for `thumbv7em-none-eabihf` in a scratch
crate (lane's report, not rerun here).

Choices the lane made that the plan does not: A2 hashes on the key's low 64
bits with linear probing, which a sender who picks keys could pile into one
run, so a real A2 needs a keyed hash or a library table; the candidates check
age on lookup and evict the oldest when full, where the board sweeps and the
desktop prunes; bridge eviction is one ranked pass. Not measured: `heapless`
0.9's `IndexMap`, a fixed-capacity map already among retinue's dependencies;
time on a Cortex-M4; anything with the desktop's locks.

### rns-rs boundary (2026-11-18)

Read at `663e5e32d526c42ffaf4044ef322cd800b248d10`, with a clean checkout.
Design learning only; nothing adopted.

`rns-core` owns a transport state machine; `rns-net` owns scheduling and I/O.
`TransportConfig` supplies retention capacities, byte budgets and timeouts;
`InterfaceInfo` supplies interface capabilities, policy and sampled rates.
Inbound frames carry `now` as seconds in an `f64`; maintenance also takes
`now`, and callers lend an RNG. Processing returns typed transport actions,
including directed sends, broadcasts and local deliveries. An alternative
entry point appends into a caller-owned vector so the driver can reuse it.
The driver dispatches those actions and performs the actual sends.

Retinue can learn the explicit separation of facts, decisions and execution,
and keep capacity and time visible at the boundary. The compact and parallel
runtime engines schedule the same protocol core; the ESP32 still has its own
loop. This is not an allocation-free example: the core uses allocated maps
and vectors, and some limits can be unlimited. Nor does returning an action
establish successful output admission. Retinue still needs its bounded storage
choice and the refusal/commit contract in Q8. None of this settles T2's storage
fork.

Files read under `C:/Users/mark_/Code/crates/rns-rs/` (source excerpts unless
noted; paths are the complete read inventory, including searches):

- `docs/architecture.md`, `docs/engines.md`.
- `rns-core/src/transport/mod.rs` (`InboundFrame`, engine boundary).
- `rns-core/src/transport/types.rs` (interface facts, actions, configuration).
- `rns-core/src/transport/engine_state.rs` (construction and interface removal).
- `rns-core/src/transport/inbound_engine.rs` (receive entry points and reusable output).
- `rns-core/src/transport/maintenance.rs` (`tick` and caller-supplied time).
- `rns-core/src/transport/retention.rs` (capacity enforcement).
- `rns-core/src/transport/jobs.rs` (symbol/comment search only).
- `rns-net/src/driver.rs` (engine construction).
- `rns-net/src/driver/dispatch.rs` (action execution).

## Questions for the owner

None open. The ten T0 questions are ruled in D3 to D12. Their evidence and
options are in this file's history and in the T0 table.

## Progress

- 2026-11-18: plan written. D1 and D2 answered.
- 2026-11-18: T0 source reading recorded: 23 rows, including 10 additions;
  proposed classes are 4 board right, 5 desktop right, 4 both wrong and 10
  intended differences. Ten owner questions remain open. First-pass rows 11,
  12 and the scope of 13 corrected; seven skipped files and the pinned rns-rs
  boundary read. No source changes, builds, tests or interop runs; T1 not begun.
- 2026-11-18: the ten questions were put to the owner in three rounds and ruled
  (D3 to D12). The stock lines behind each question, and rows 1, 2 and 22, were
  re-read before asking. Rows 14 to 21 and 23 were not rechecked. T0 is done.
- 2026-11-18: T1 started in a Codex lane (owner: "Yes, Codex lane") on branch
  `transport-core-t1`, cut from `704a04f`. Thirteen rows change: 1 to 8, 11,
  13, 20, 22 and 23. Row 23's stock lines (`Transport.py:2292`, `2813`) were
  re-read first and hold. Baseline in the worktree: 773 of 774 tests pass.
  `reliable_exchange_survives_loss_including_identify` fails 6 of 6 at its
  20 s limit; on `main` it passed in 19.68 s the day before. The lane does not
  touch that test.
- 2026-11-18: two lane starts produced no commits. The first could not start a
  shell in its sandbox. The second proved row 22 (Node test failed, then
  passed, 775 of 776) and then could not write the worktree's git index under
  Codex's workspace-write sandbox; it reverted its edits. It also found that
  `cargo test -p retinue --no-default-features` does not compile at `704a04f`:
  `tests/network_stamps.rs` (`f3deed5`) has no `required-features` entry.
  Confirmed here, 13 errors. Operational answers from the owner: the lane runs
  with full access, and that gate is repaired first on the lane branch.
- 2026-11-18: the third start stopped at the `dependency-bans-native` hook: a
  fresh resolve of `native/` holds `simple-dns` 0.12.0 and 0.13.1 (`iroh-dns`
  1.4.1 directly, and through `n0-dns-resolver` 0.1.1). `main`'s own
  `native/Cargo.lock` (2026-11-17) holds only 0.12.0. The worktree had been
  given the root lock but not the five nested ones; with all six copied from
  `main` every hook passes. The gate repair is commit `ddef5f4` on
  `transport-core-t1`, made here to prove the commit path. Fourth start is
  running, beginning at row 22. About 230k Codex tokens went to the three
  failed starts and two probes.
- 2026-11-18: the fourth start committed rows 22 (`5e24939`) and 2 (`13dde48`)
  on `transport-core-t1` and stopped at row 1's fork. Both diffs were read
  here. Row 2 changed three existing Node tests under D3: with 4 routes and 8
  relay slots, a poll that sent 8 relays now sends 3 or 4, because only
  destinations still holding a route relay. Tip: 777 of 778 tests, the no-alloc
  gate and the thumb `alloc` check pass (lane's report; not rerun here). Row 1
  was put to the owner and ruled (D13). The lane continues on the ten
  remaining rows and now sets a fork aside instead of stopping.
- 2026-11-18: the fifth start committed rows 7 (`af2cdb1`), 23 (`b03beea`),
  3 (`a16a051`), 4 (`09dfca1`), 20 (`d5eb8f9`), 11 (`f800244`) and 6
  (`ffe7a94`), each with a same-named test in `node/tests/transport_core.rs`
  and `endpoint/tests/transport_core.rs`. The source diffs were read here.
  One existing endpoint test changed under row 3
  (`a_link_bridge_accepts_only_its_two_interfaces` now sends distinct packets
  each way). Row 11 has a `NOTICE` entry. Lane's report at `ffe7a94`: 791 of
  792 tests, the no-alloc gate and the thumb `alloc` check pass. Run here at
  `ffe7a94`: `run_live.py`, 35 of 35 gates against stock RNS 1.6.0. The lane
  set rows 5, 8 and 13 aside. Row 13 was already answered by D12 as put; rows
  5 and 8 are ruled in D14 and D15. `peer_rnsrs.py` and the firmware builds
  have not been run on the branch.
- 2026-11-18: the sixth start committed rows 5 (`66bda85`), 8 (`57bb9a4`) and
  13 (`e3cb010`), again with same-named tests in both drivers and no existing
  test changed. The source diffs were read here, including the lock order
  around the endpoint's return-path table, which is now held across the send:
  no other site takes it under another lock. Run here at `e3cb010`:
  - `cargo nextest run -p retinue`: 799 of 800; the one failure is the
    baseline timing test.
  - `cargo test -p retinue --no-default-features`: 36 of 36. Thumb `alloc`
    check: builds.
  - `run_live.py`: first run 34 of 35. `interop_ifac.py` failed when stock
    raised `'TCPClientInterface' object has no attribute 'ifac_size'`; it then
    passed 6 of 6 alone, and a second full run passed 35 of 35.
  - `peer_rnsrs.py`: 8 of 8, rns-rs `663e5e3` rebuilt on the Unix host.
  - T114 image: builds. V4 image: the installed `esp` toolchain is 1.97.0 and
    the crate requires 1.97.1, so it builds only with `--ignore-rust-version`.
  T1's done-condition is met on the branch for the twelve rows, with those two
  qualifications. Codex used about 729k tokens across T1's six starts and two
  probes.
- 2026-11-19: the owner said "Merge it". `main` fast-forwarded from `704a04f`
  to `e3cb010`; no file in the merge overlapped the uncommitted documents. The
  lane's worktree, branch and Cargo target are removed. Not pushed.
- 2026-11-19: the session's documents and `peer_rnsrs.py` are committed on
  `main` as `5befe56` (owner: "Commit, then proceed."), not pushed. T2's fork
  work started in two lanes on cheaper models (owner: "Feel free to
  orchestrate ... into lanes"): a read-only inventory of every operation on
  the four tables in both drivers, and a measurement lane on branch
  `transport-core-t2-measure` in `Code/worktrees/murm-transport-core-t2` that
  benchmarks the candidates at 16 and 4096 entries. No T2 code is written
  until the storage fork is ruled.
- 2026-11-19: the measurement lane committed its benchmark as `04d72d3` on
  `transport-core-t2-measure` (nine files under
  `crates/retinue/examples/transport_table_cost/`; hooks passed; not on
  `main`). Its run was on a machine at 93 to 100% load. The harness and
  candidate code were read here and the program rerun once: 4 of 128 rows
  spread over 20%, and the rerun's numbers are the ones recorded under
  "T2 storage measurement". The lane's "about half of today's desktop" for B
  and C over `HashMap` did not hold on the rerun and is not recorded. The
  storage fork and the two open readings were put to the owner and ruled
  (D16 to D18). No T2 code is written. The measurement worktree, branch and
  target are still in place.
- 2026-11-19: the owner said "And push" and "And pull if needed". `main` was
  26 ahead of `origin/main` and 0 behind; pushed at `5befe56` (the 14 commits
  of this plan's work and 12 earlier ones). T2's code started in an opus lane
  (owner: "You can use sonnet or opus and orchestrate subagents.") on branch
  `transport-core-t2` in `Code/worktrees/murm-transport-core-t2-tables`, cut
  from `5befe56`. The lane writes the shared module first, with no driver
  change, and stops for a review of its surface before either driver adopts
  it.
- 2026-11-19: the T2 lane committed stage 1 as `be5a388` on
  `transport-core-t2`: a private `crate::transport` module (750 lines, and 526
  of tests) with the storage trait over `heapless::Vec` and `HashMap`, the
  three entries, the table operations and the transit memory. No driver uses
  it yet. The module was read here. Lane's gates: nextest 834 of 835 (the
  known timing test), no-alloc 54 of 54, thumb `alloc` check builds; not rerun
  here. The trait has nine operations, not the four D16 named: exact undo
  (D15) and the board's two removal orders need `restore`, `retain` and
  `sweep`. The desktop's packet memory is read only through `insert`
  (`endpoint/dedup.rs:126`, checked), so the board's turnover rule gives the
  same answers there. The lane's two forks were put to the owner and ruled
  (D19, D20). The lane continues with Node, then Endpoint, then the transit
  memory. Found by the lane and left alone:
  `--no-default-features --features alloc --all-targets` does not compile at
  `node/tests/transport_core.rs:273` (a T1 test names a function that only
  builds with `tokio`); it is not a gate.
- 2026-11-19: the T2 lane finished on `transport-core-t2`: `7a2a36c` moves
  Node onto the shared tables and `b4cf3b7` moves Endpoint. The route entry,
  link bridge, return-path entry and transit memory are each defined once, in
  `crate::transport`; the drivers keep type aliases (`PathEntry`,
  `LinkBridge`, `ReverseEntry`, `HashList` on the desktop; `RouteSlot` and
  four aliases on the board). Both drivers' path-request tag memories moved
  with the type. No fork in these stages. 30 files, 1690 lines added and 568
  removed, of which 1275 are the stage 1 module and its tests.
  - Read here, old code against new: eviction order, counters and their call
    sites, lock order, and the board's sweep before every packet are as they
    were. The board's `touch_route` now touches only a live route; both of its
    callers already hold a live route at the same `now`.
  - Tests: no endpoint test file changed. About 45 board test lines changed in
    how they build or reach an entry (D19), in `node/tests/bridges.rs`,
    `routes.rs`, `transit.rs` and `transport_core.rs`; no assertion changed.
  - Run here at `b4cf3b7`: nextest 834 of 835 (the known timing test); no-alloc
    tests 54 of 54; thumb `alloc` check builds; `run_live.py` 35 of 35 on the
    first run; `peer_rnsrs.py` 8 of 8 against rns-rs `663e5e3`, rebuilt on the
    Unix host and removed after; T114 image builds; V4 image builds with
    `--ignore-rust-version`, as before.
  - T114 image against `5befe56`: text 374,614 to 376,670 bytes (2,056 more),
    bss 138,668 to 138,812 (144 more).
  - Left as they were: Node's and Endpoint's two `REVERSE_TIMEOUT` constants
    (Node's is public); the generic `make_room` helper, still used by the
    tunnel restore and `single.rs`; the echo windows and the announce window.
  - Not measured: stack on the board. A refused send now restores from a
    bounded list of up to `ROUTES` displaced return paths held on the stack
    during the call (about 640 bytes at 16), where it held one before.
  - The module carries `allow(dead_code)` when `alloc` is off, because both
    drivers need `alloc` and only its tests use it on the floor.
  T2's done-condition is met on the branch. Not merged.
- 2026-11-19: the owner said "Merge commit push proceed!". `main`
  fast-forwarded from `5befe56` to `b4cf3b7`, the tree the gates ran on. This
  plan is committed with it and `main` is pushed. The two T2 worktrees, their
  branches and targets are removed, the measurement branch
  (`transport-core-t2-measure`, `04d72d3`) with them: its numbers are in
  "T2 storage measurement" and its benchmark is not kept.
