# One shared transport core for Node and Endpoint

**Status, 2026-11-19: T0 and T1 done.** T1 is on `main` at `e3cb010` (twelve
rows and the no-alloc gate repair, fast-forwarded from the lane branch; not
pushed). Row 1 moved to T3 (D13). T2 has not started and begins with its
storage fork.

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

**Fork to return before code:** how one implementation serves 16 entries on the
board and 4096 on the desktop. Candidates are one bounded type with a capacity
parameter, a small storage trait, or shared entry types and decision functions
over driver-owned containers. Bring back lookup cost per carried packet at 4096
entries for each, measured.

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
