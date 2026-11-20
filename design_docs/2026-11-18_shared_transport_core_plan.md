# One shared transport core for Node and Endpoint

**Status, 2026-11-19: done. T0 to T5 are on `main` at `bc017a2`.** `Node` and
`Endpoint` share their transit tables (T2) and forwarding decisions (T3) in
`crate::transport`; route life follows stock 1.6.0 (T2a); the shared module's
tests run at the real table sizes (T4). What each driver still decides is
listed under "Deferred and follow-on".

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

### T3 rulings, D21 to D24 (2026-11-19)

Put with the evidence in "T3 inventory of the forwarding decisions". Each
answer is the option's name as chosen.

- **D21, how the board gets its interface list (row 1, refines D13):** options
  were a list the board learns as interfaces are named; an explicit attach
  call; or the list passed at each poll. Answer: "Learned as named". The board
  keeps one bounded interface table. An interface joins when the shell first
  names it in `ingest`, `poll` or a per-interface setting, and leaves on
  `forget_interface`. No caller or test changes; a single-radio board behaves
  as now.
- **D22, a relay that does not fit in one poll:** one due relay becomes
  several sends under D2, and a poll returns at most `ACTIONS` actions.
  Options: carry the remainder; wait for room for all; send what fits and drop
  the rest. Answer: "Carry the remainder". The board sends on as many
  interfaces as fit and keeps the entry with the interfaces still owed.
- **D23, statistics:** the shared functions return a decision with a reason,
  and the drivers count seven of those reasons differently today (inventory
  rows d to k). Options: reasons shared and counters as today; the same
  counters in both now; add only the missing ones. Answer: "Reasons shared,
  counters as today". No public counter changes in T3; the differences are a
  follow-on.
- **D24, carriage switched off while a bridge or return path exists
  (inventory row a, T0 row 25):** the board treats the packet as not transit
  and leaves the tables alone; the desktop drops and counts it, consuming a
  proof's return path first. Stock with transport off skips its transport
  block (`Transport.py:2271`, `2884`, `3009`). Options: the board's rule; the
  desktop's rule; leave both. Answer: "Board's rule, as stock". The desktop
  changes: with carriage off the packet goes on to local handling and the
  return path is not consumed. Its per-interface ingress selector still
  drops, after checking that carriage is on.

Links: area retinue transport · refines D2, D13 · draws on T3 inventory · plans T3 · summary the board learns its interfaces as named and carries a relay's remainder; reasons are shared and counters stay; carriage off means not transit

### T3 rulings, D25 to D27 (2026-11-19)

Put with the evidence in "Stock 1.6.0 against the T3 differences". Each answer
is the option's name as chosen.

- **D25, a route's life when it carries traffic (T0 row 24, corrected):**
  both drivers extend a route's life each time it carries a packet; stock
  1.6.0 only marks the path used, moves expiry on an announce or on proven
  liveness, and culls never-used paths. Options: follow stock after T3, once
  in the shared core (recommended here); follow stock before T3; keep
  Retinue's rule as an intended difference. Answer: "Follow stock, before
  T3". This is phase T2a.
- **D26, order of the announce relay gate (T0 row 26):** the board checks the
  hop ceiling and then remembers the hash; the desktop remembers, then
  rate-blocks, then checks the hop ceiling; stock remembers first. Options:
  the desktop's order; the board's order. Answer: "Desktop's order". On the
  board a repeated over-limit announce is filtered, not counted again.
- **D27, hop count on a carried link's packets (T0 row 27, candidate):** stock
  carries a link packet only if its hop count matches the count recorded for
  that side; neither driver records or checks one. Options: record it and
  decide later; add the check after T3; intended difference. Answer: "Record
  it, decide later". Not ruled; not part of T3.

Two differences were not put as questions because sharing leaves one answer.
The shared sequence reads the route once per carried packet, where the desktop
reads it up to three times (inventory row b); and a new bridge whose request
is not sent is removed, which is the desktop's code and the board's effect
(row c; stock keeps it, as D9 already departs from stock for return paths).

Links: area retinue transport · refines D9 · draws on Stock 1.6.0 against the T3 differences · plans T2a, T3 · summary route life follows stock 1.6.0 before T3; the announce gate remembers first; the bridge hop check is recorded, not ruled

### T2a rulings, D28 to D30 (2026-11-19)

Put with the evidence in "T2a inventory". Each answer is the option's name as
chosen.

- **D28, which events move a route's expiry besides an announce:** options
  were both of stock's events; our own links only; announces only. Answer:
  "Both of stock's events". Expiry moves when this node, as initiator,
  establishes a link, and when it carries a link proof whose signature it
  verified. A proof passed on the side check alone moves nothing. The board's
  pending link gains the destination hash.
- **D29, the never-used cull:** stock culls a never-used path after 1.75 h on
  a transport node and 12 min on a leaf, ten times that under 25 kbit/s. The
  board does not originate path requests. Options: the desktop only, with
  stock's numbers; both drivers; record "used" with no cull yet. Answer:
  "Desktop only, stock's numbers". The shared core takes the linger time as a
  parameter; the board passes none until it can ask for paths.
- **D30, what goes first in a full table:** options were never-used, then
  least recently used; oldest announce (today's code); never-used, then
  oldest announce. Answer: "Never-used, then least recently used". Expired
  routes first; then never-used routes, soonest expiry first; then the route
  used longest ago.

What D25 carries without a further question, each as stock does it:

- Sending or carrying by a route marks it used and moves nothing else. A send
  by a direct route marks it too (`Transport.py:1618`); today only a send
  through a relay touches the route.
- Carrying across a bridge marks the link destination's route used (`2440`).
- An announce that replaces a route keeps its "used" mark (`2510`, `2726`).
- Timing stays as ruled and as it is: a carried packet marks the route once
  the send is accepted (D9); our own send marks it before the send.
- Not taken: stock's unused-path pool of 2048 (D30 covers a full table), and
  its path state (unknown, unresponsive, responsive).

Links: area retinue transport · refines D25 · draws on T2a inventory · plans T2a · summary expiry moves on an announce, an own link and a verified carried link proof; the desktop culls never-used routes with stock's linger; a full table drops never-used routes first, then the least recently used

### T2a rulings, D31 and D32 (2026-11-19)

Put after the lane's four commits were read. Each answer is the option's name
as chosen.

- **D31, announce replay after a cull (refines D29):** a replayed or older
  announce is refused only while a live route exists
  (`announce_freshness.rs:222`). Once the desktop culls a never-used route,
  12 min on a leaf or 1.75 h on a transport node, the next announce for it is
  a first sighting, where before that took the 7-day lifetime. Stock keeps
  its replay memory on the path entry it culls and behaves the same. Options:
  accept it, as stock; keep refusing older announces for the full lifetime
  with a third freshness state; no cull for now. Answer: "Accept it, as
  stock". A known property: a leaf desktop has 12 minutes of replay
  protection for a destination it has never used.
- **D32, which interface is slow (refines D29):** stock lingers ten times
  longer under 25 kbit/s. The lane counted only a configured bitrate; the LoRa
  station in postilion sets an airtime allowance and no bitrate
  (`crates/postilion/src/station.rs:66`). Options: bitrate, else infer from
  the airtime allowance; bitrate only. Answer: "Bitrate, else airtime". More
  than 160 ms for a 500-byte packet is under 25 kbit/s.

Links: area retinue transport · refines D29 · plans T2a · summary the cull's replay window is accepted as stock's; a slow interface is known by bitrate, else by airtime allowance

### T3 rulings, D33 and D34 (2026-11-19)

Two things D21 left open, put before the lane reached the interface table.
Each answer is the option's name as chosen.

- **D33, the size of the board's interface table (refines D21):** the board's
  three per-interface tables (modes, airtime, announce caps) are each fixed at
  4 by a constant; today's boards name one interface and the V4 node will name
  two. Options: fixed at four; a fifth `Node` parameter with default 4 (the 13
  places that spell out Node's parameters change). Answer: "Fixed at four".
  One constant; `Node`'s type does not change.
- **D34, a further interface when the table is full (refines D21):** `ingest`
  and `poll` return no error, so the board cannot refuse as
  `set_interface_mode` does. Options: served, not a relay target; the same
  with a new public counter; replace the least recently named. Answer:
  "Served, not a relay target". Packets on the extra interface are handled
  and carried as now; relayed announces do not fan out to it; the first
  interfaces named keep their slots; nothing counts it.

Links: area retinue transport · refines D21 · plans T3 · summary the board's interface table holds four; a fifth interface is served but is not a relay target

### T3 fork ruling, D35 (2026-11-19)

The T3 lane set D24's desktop change aside. Checked here against the source.

- **D35, whether carriage off still counts (refines D24, applies D23):**
  today the desktop counts `policy_rejected` for a packet it would have
  carried (`endpoint/router.rs:132`, `336`); the board and stock count
  nothing. Two public integration tests assert the count is 1
  (`tests/endpoint_transit_policy.rs:92`, `222`). Options: still counted;
  not counted (the lane's recommendation). Answer: "Still counted". With
  carriage off the packet goes on to local handling and a proof's return path
  is kept, as D24 ruled, and `policy_rejected` still moves once. The desktop
  looks up the return path without consuming it. The board keeps counting
  nothing; that difference joins the statistics follow-on.

Links: area retinue transport · refines D24 · applies D23 · plans T3 · summary carriage off is not transit on the desktop, and policy_rejected still counts it

### T3 fork ruling, D36 (2026-11-19)

The T3 lane set this aside after writing the shared functions. Checked here
against the source. It corrects inventory row c, which this plan called the
same result by different means.

- **D36, a carried link request that is not sent, with the bridge table full
  (applies D15):** recording the new bridge evicts another. The board records
  only if the send will fit (`node/transit.rs:237`), so an unsent request
  evicts nothing. The desktop records, evicts, then removes only the new
  bridge on refusal (`endpoint/transit.rs:260`), so the evicted bridge is
  lost. Options: put back what was displaced; room as an input, each driver
  as today (the lane's recommendation); remove the new bridge only. Answer:
  "Put back what was displaced". A refused send leaves the bridge table as it
  was, in both drivers, by one shared sequence. The desktop changes; the
  board's result stays and it counts an eviction once the send is accepted.

Links: area retinue transport · applies D15 · refines D9 · plans T3 · summary a refused link request leaves the bridge table as it was, in both drivers

### T4 ruling, D37 (2026-11-19)

Put with the numbers in "T4 inventory of the mirrored tests". It rewrites
T4, which said driver tests "keep only what a driver owns".

- **D37, how far the paired driver tests go:** 32 tests share a name across
  the drivers (950 board lines, 1,139 desktop lines); 26 mix a shared
  decision with what a driver owns, 5 are driver-owned, 1 asserts only a
  shared decision but runs through each driver. Options: keep the pairs and
  fill the gaps; slim the pairs, as the plan said; one body for both drivers
  over a common rig. Answer: "Keep the pairs, fill the gaps". The pairs stay
  as end-to-end checks that each driver calls the shared decision correctly.
  T4 adds the shared tests still missing, runs the shared table tests at the
  real sizes (16 and 4096, beside 4), and gives the mirrors with different
  names one name. No test is removed.

Links: area retinue transport · refines T4 · draws on T4 inventory · plans T4 · summary the paired driver tests stay; T4 fills the shared gaps, tests at real sizes and aligns names

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

### T2a. Route life as stock 1.6.0

Ruled by D25, before T3. Carrying or sending by a route marks it used and no
longer extends its life. A route's expiry moves when it is learned from an
announce and on the events stock treats as proof that the destination is
alive. A never-used route is culled after a linger time. One change to the
shared route entry and operations, with each driver's call sites, failing
test first, the same rule asserted in both drivers.

**Forks, returned and ruled (D28 to D30):** expiry moves on an announce, on
our own link being established and on a verified carried link proof; the
desktop culls never-used routes with stock's linger times and the board does
not; a full table drops expired routes, then never-used ones, then the least
recently used.

**Done when:** a route that only carries traffic expires at the time it was
due when learned, in both drivers, under same-named tests; the Rust suite,
the live suite and `peer_rnsrs.py` pass; the firmware images build.

### T3. Share the forwarding decisions

Move bridge admission, type-2 forwarding, reverse-proof handling and the
announce relay gate into the shared module as functions that return a decision.
Drivers keep locking, queues, `Actions`, statistics and policy selection.

Row 1 lands here (D13): the core takes the list of attached, permitted
interfaces as an input. The board learns its interfaces as the shell names
them (D21), up to four (D33, D34), and carries the unsent remainder of a relay
across polls (D22).
The functions return a decision with a reason and each driver keeps its
counters (D23). With carriage off a packet is not transit (D24). The announce
gate remembers the hash first (D26).

**Done when:** neither `node/` nor `endpoint/` decides whether or where a
carried packet goes; each only supplies inputs and carries out the decision.

### T4. Test the shared core once

One behaviour suite runs against the shared module at the board and desktop
capacities. The paired driver tests stay as end-to-end checks of each driver
(D37); mirrors under different names take one name.

**Done when:** every shared decision a paired test asserts has a test in the
shared module, or the gap is named here; the shared table tests pass at 4, at
16 over the array and at 4096 over the map; and the counts of tests added,
renamed and kept are recorded here.

### T5. Qualify

**Done when:** the Rust suite, the live suite and `peer_rnsrs.py` pass at the
final revision; the firmware images build, and their flash and RAM figures are
recorded beside the figures from before T2.

**Result (2026-11-19).** The final revision is `bc017a2`. The Rust suite ran
there: nextest 920 of 921 (the known timing test,
`reliable_exchange_survives_loss_including_identify`), no-alloc tests 69 of
69, thumb `alloc` check builds. The live suite (35 of 35 against stock RNS
1.6.0), `peer_rnsrs.py` (8 of 8 against rns-rs `663e5e3`) and both firmware
builds ran at `00da22d`, the last revision that changed non-test code; T4
after it changed tests and named the desktop's table sizes. The owner
accepted those runs for the final revision.

| T114 image | text | bss |
|---|---|---|
| Before T2 (`5befe56`) | 374,614 | 138,668 |
| After T2 (`b4cf3b7`) | 376,670 | 138,812 |
| After T2a (`e6c0cec`) | 377,630 | 139,772 |
| After T3 (`00da22d`), final | 380,670 | 139,844 |

Over the whole plan: 6,056 bytes more flash, 1,176 bytes more RAM. The V4
image builds (with `--ignore-rust-version`: its toolchain is 1.97.0 against a
required 1.97.1) and has no figure here because it holds none of this code:
it never enables `retinue/alloc`, which `Node` and the transport need
(`firmware/heltec-v4-phy/Cargo.toml:20`).

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
- The seven statistics the drivers count differently (T3 inventory rows d
  to k), left as they are by D23.
- What each driver still decides after T3 and T4, each covered by a paired
  driver test and by no shared test: the order "bridge, then type-2 hop, then
  return-path proof, handled once"; a short link request's id and a link
  request recording no return path; what makes a packet "addressed" to this
  hop; which packet contexts skip the duplicate memory; which side of a
  return path an interface removal purges. Lines are in the 2026-11-19
  progress entries. Also left with the drivers in T3: the board's check that
  a send will fit before it records a bridge, the desktop send point's own
  egress and hop-ceiling checks, and the size and MTU steps.
- `reliable_exchange_survives_loss_including_identify` fails at its 20 s
  limit in every run of this plan; it predates the plan and was left alone.
- The hop-count check on a carried link's packets (D27, candidate row 27).
- A carried link request is stamp-checked by both drivers; stock checks only
  at the destination. Noticed in T3's stock reading; not ruled.
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

### T3 inventory of the forwarding decisions (2026-11-19)

A read-only lane listed, step by step with file and line, how each driver
decides the four things T3 shares, at `d90113e`: carrying a link request and
crossing a bridge; type-2 forwarding; carrying a proof back; and the announce
relay gate at receipt and at release. Checked here against the source: the
board's release loop and `poll`, both bridged paths, the desktop's proof arm,
its announce gate and release, and its forward tail. The rest is the lane's
reading. Paths start at `crates/retinue/src/`.

The rows T1 converged read as ruled in both drivers (2 to 8, 11, 13, 20, 22,
23). Row 1 is still open in code: the board sends a relay only on
`rebroadcast.interface` (`node/rebroadcast.rs:112`).

What the board knows about interfaces, for row 1 (D13):

- It keeps no list of attached interfaces. It has three per-interface tables
  of four entries each: non-default modes (`interface_modes`), airtime
  allowances (`first_hop_airtime`) and announce caps (`announce_caps`).
- `ingest(interface, packet, now)` and `poll(now, interface, blob)` each take
  one interface (`node/ingest.rs:19`, `node/poll.rs:32`).
- Every caller outside the crate's tests passes one interface: `radio-hand`
  (`RADIO = 0`, and it ignores the interface of a `Send`,
  `channel/node.rs:215`), `Instance`, `retinue-sim`, the fuzz target. Only the
  `path_request_oracle` example ingests on several. No caller outside tests
  sets an interface mode on a Node. About 320 test call sites name an
  interface without declaring it first.
- A relay that does not fit in one `poll` stays in the rebroadcast table and
  is taken by the next: the release loop pops only while `Actions` has room
  (`node/rebroadcast.rs:89`), and relays run last in `poll`. `ACTIONS` is 8 on
  the boards and 1 in one test (`tests/node_pause/forced.rs:88`).
- A carried data packet, bridged packet or proof is sent in the same `ingest`
  or lost; nothing re-offers it.

Differences the T0 table does not list. Those marked "outcome" change what
happens to a packet; the others change only which statistic moves.

| # | Difference | Node | Endpoint | Kind |
|---|---|---|---|---|
| a | Carriage switched off while a bridge or return path exists | not transit: the packet goes on to local handling and the tables are untouched (`node/transit.rs:85`, `178`, `292`) | dropped and counted; a proof's return path is consumed before the policy is asked (`endpoint/router.rs:132`, `325`) | outcome |
| b | One read of the route per carried packet | one live route gives interface, next hop and hop count | the interface is read live, then the next hop and the hop count are read again without a liveness check (`endpoint/transit.rs:279`, `362`) | outcome, only if the route changes between reads |
| c | A new bridge whose request is not sent | recorded only if the action list has room; never removed (`node/transit.rs:227`) | recorded, then removed if the send is refused (`endpoint/transit.rs:232`) | same result by different means |
| d | Order of the announce relay gate | hop ceiling, then duplicate memory, then size (`node/transit.rs:345`) | duplicate memory, then rate block, then hop ceiling (`endpoint/announces.rs:441`) | statistics: a repeat of a refused announce is counted again on the board, once on the desktop |
| e | Packet for a bridge on a third interface | dropped, not counted (`node/transit.rs:99`) | dropped, `policy_rejected` (`endpoint/router.rs:109`) | statistics |
| f | Traffic ahead of the link proof, or a bad proof | `unvalidated_link_packets` | `policy_rejected` | statistics |
| g | A duplicate carried packet | not counted | `filtered_packets` | statistics |
| h | Bridge evicted, bridge refused | `evicted_bridges`, `refused_bridges` | eviction not counted; refusal is `policy_rejected` | statistics |
| i | Link-request stamp refused | `refused_stamps`, checked after the path-request branch | not counted, checked before it | statistics and order |
| j | Relayed announce counted | once per accepted send | once per transmission if any interface took it | statistics; matters once the board fans out |
| k | Lapsed bridges pruned | every packet and poll, `expired_bridges` | every link request, not counted | D18; statistics |

Stock, read here for (a): with transport off, stock skips its whole transport
block (`Transport.py:2271`) and consults neither the link table for a link
proof (`2884`) nor the reverse table for a proof (`3009`; the entry is not
popped), so the packet goes on to local handling. The board matches. Row 24
(stock marks a link's destination path as used when carrying bridged traffic)
is still a candidate, not ruled.

### Stock 1.6.0 against the T3 differences (2026-11-19)

A second read-only lane read stock RNS 1.6.0 for seven of the differences
above. Read again here: the lines for the path "used" mark and expiry. The
rest is the lane's reading. Lines are `Transport.py` in the 1.6.0 wheel.

- **A route's life when it carries traffic (corrects candidate row 24).**
  Both drivers extend a route's life each time it carries a packet
  (`transport/tables.rs:97`, called at `node/transit.rs:273` and
  `endpoint/transit.rs:231`); the comments cite lines of an older RNS. Stock
  1.6.0 does not. Carrying by path (`2387`), carrying across a link (`2440`)
  and sending (`1618`) set only the path's "used" mark. Expiry is set when the
  path is learned and moved only by `mark_path_responsive` (`3633`), whose
  comment reads "Marking a path live must only occur on events that
  cryptographically prove recent destination liveness". The "used" mark keeps
  a path out of the never-used cull (`1145`; 1.75 h on a transport node,
  12 min on a leaf, `228`). Retinue has no "used" mark and no never-used cull.
  Neither driver marks anything when carrying across a bridge.
- **A link packet whose hop count is wrong.** Stock carries a packet across
  its link table only if its hop count equals the count recorded for that
  side (`2408` to `2424`). Neither driver records or checks hop counts on a
  bridge. On a third interface all three drop the packet and remember
  nothing.
- **Order of the announce relay gate (inventory row d).** Stock remembers the
  announce's hash before any relay check (`2235`), limits hops early (`2022`,
  `2489`), suppresses repeats by the freshness of the random blob (`2499` to
  `2578`), then rate-limits (`2584`), then inserts (`2607`). The desktop
  remembers before its rate and hop checks, as stock; the board checks hops
  first.
- **A carried link request that is not sent (row c).** Stock writes the link
  entry before transmitting and never removes it (`2374`, `2386`);
  `Transport.transmit` returns nothing. The desktop removes a new bridge on a
  refused send; the board records one only when the send will fit.
- **Hop count for the proof timeout (row b).** Stock uses the path entry's
  hops, floored at one, plus the first-hop extra (`2298`, `2333`). Both
  drivers use the route's hops plus one and the same extra
  (`node/params.rs:140`). The desktop reads the route a second time for it.
- **Stamp check on a carried link request (row i).** Stock checks a link
  request's stamp only at its destination (`Link.py:195`), never when carrying
  one. Both drivers check carried requests against the route's announced cost
  when their policy enforces stamps (`node/ingest.rs:63`,
  `endpoint/router.rs:69`). Noticed; not part of this plan.
- **Own receipt after carrying a proof.** Stock carries a proof and then also
  tries its own receipts (`3009` to `3036`). Both drivers stop after
  carrying, as D8 ruled.

### T2a inventory: route life in stock 1.6.0 and in Retinue (2026-11-19)

A read-only lane listed every place stock and both drivers set, extend or
remove a route. Read again here: stock's liveness callers, the carried-over
"used" mark and the linger selection. The rest is the lane's reading. Stock
lines are `Transport.py` unless named.

Stock 1.6.0:

- A path entry holds a timestamp, an absolute expiry and a "used" time (0 is
  never used) (`4866` to `4873`).
- An accepted announce writes a new entry: timestamp now, expiry now plus the
  lifetime for the receiving interface's mode (7 days, 1 day for access
  point, 6 hours for roaming), and "used" carried over from the entry it
  replaces (`2494`, `2510`, `2726`). A rate-blocked announce moves none of
  the three (`2717`).
- Sending by a path (`1618`, before the send, direct paths included),
  carrying by a path (`2387`) and carrying across a link (`2440`, on the
  link destination's path) set "used" only.
- `mark_path_responsive` sets timestamp, expiry and "used" (`3632`). It is
  called when this node, as initiator, validates a link proof (`Link.py:513`)
  and when a transport node validates the signature of a link proof it is
  carrying (`2938`), plus two link-rebalancing cases (`2907`, `2983`). A
  proved packet receipt and a concluded resource call nothing.
- The cull runs every 5 s (`1014`): a path past expiry goes; a path whose
  interface is gone goes; a never-used path goes once its timestamp is older
  than the linger time (`1137` to `1148`). Linger is 1.75 h on a transport
  node and 12 min on a leaf (`228`, `451`), ten times that on an interface
  under 25 kbit/s (`232`). Every 5 min, never-used paths beyond 2048 are
  culled, fast interfaces first, soonest expiry first (`1150`).
- No lookup checks expiry; a path is usable until the cull removes it.
- A separate path state (unknown, unresponsive, responsive) lets a
  same-emission announce with more hops replace an unresponsive path
  (`2573`). Not part of D25.

Retinue:

- `RouteEntry` holds `learned` and the learning interface's mode; a route is
  live while `now - learned` is under the mode's lifetime
  (`transport/entries.rs:9`). There is no "used" mark.
- `touch_route` sets `learned = now`. Callers: the board when opening a link
  through a relay, before the send (`node/links.rs:91`), and when carrying a
  type-2 packet, after acceptance (`node/transit.rs:273`); the desktop when
  addressing any of its own packets through a relay, before the send
  (`endpoint/shared.rs:477`), and when carrying, after acceptance
  (`endpoint/transit.rs:231`). A send by a direct route touches nothing.
  Carrying across a bridge touches nothing.
- Routes are learned only from announces (and on the desktop from a gravity
  repoint and a tunnel restore, which keeps the mirrored `learned`).
- At capacity both drivers drop expired routes, then the one with the oldest
  `learned`.
- Nothing distinguishes a transport node from a leaf in route life. The board
  does not originate path requests; the desktop does.
- Where a link is established as initiator, the destination hash is in scope
  on the desktop (`endpoint/link_setup.rs:325`) and is not stored with the
  board's pending link (`node/links.rs:243`). Where a carried link proof's
  signature is checked, the bridge holds the destination
  (`node/transit.rs:109`, `endpoint/transit.rs:53`); both drivers also pass a
  proof on the side check alone when the destination's identity is unknown.
- Tests that assert traffic extends a route's life: `a_route_in_use_is_refreshed`
  in `node/tests/routes.rs:225` and `endpoint/tests/routes.rs:278`,
  `return_paths_are_transactional_with_output` in both `transport_core.rs`
  files, and `routes_touch_only_when_live` in `transport/tests.rs:154`.

### T4 inventory of the mirrored tests (2026-11-19)

A read-only lane read the shared module's tests and the tests that carry the
same name in both drivers, at `22e7592`. Checked here: the counts, the
`both!` macro and its capacity, and one pair's body. The per-pair verdicts
are the lane's reading. Paths start at `crates/retinue/src/`.

- **Counts.** `node/` has 146 tests, `endpoint/` 170. The shared module has
  46 test bodies (20 in `transport/tests.rs`, 3 in `transport/tests/memory.rs`,
  10 in `transport/tests/forward.rs`, 13 in `rebroadcast/tests.rs`); 26 of
  them run twice, over a bounded array and over `HashMap`, so 72 tests.
- **Capacities.** Every shared table test runs at 4 entries (`CAP`,
  `transport/tests.rs:14`). Board fixtures hold 2 to 4 routes. The desktop's
  tables compile to 4 under `cfg(test)` (`endpoint/paths.rs:29`). Nothing in
  the crate runs at 16 routes or at 4096.
- **The 32 same-named pairs** total 950 lines on the board and 1,139 on the
  desktop. The lane's verdicts: 1 asserts only a shared decision
  (`an_announce_that_replaces_a_route_keeps_its_used_mark`), 5 assert only
  what a driver owns (link MTU floor, a key conflict in the address book, the
  forwarded count, link-interface binding, the relayed header), and 26 are
  mixed. Even the one "shared" pair drives each driver end to end (`ingest`,
  then the stored route).
- **What the mixed pairs keep for the driver:** the counters, which differ by
  name or by value in 6 pairs (`evicted_routes` against `paths_evicted`;
  `forwarded_announces` 2 against 1; `policy_rejected` 3 against unchanged);
  the packet as sent; that a decision is applied only on accepted output; and
  how a refusal is produced (a full `Actions` on the board; a full queue, an
  offline interface or a selector on the desktop).
- **Shared tests already assert the decision** for 27 of the 32 pairs. None
  for: the announce cap's purge on interface removal (pair 20) and the order
  "bridge before return path, handled once" (pair 29). Partial for the local
  link request (pair 2) and the short link request's bridge (pair 27).
- **Mirrors under different names:** 10 more pairs the lane is confident of
  (five in `node/tests/bridges.rs` against `endpoint/tests/transit.rs`), and
  about 8 likely by name. About 25 board and 17 desktop transit tests have no
  counterpart.
- **One body for both drivers** would need a common rig over: time (`u64`
  milliseconds per call against `Instant` and the paused tokio clock), how a
  relay is released (`poll` against timers), how a send is refused, two
  counter structs, and two ways of building packets. All 32 desktop tests
  are `#[tokio::test]`; the board's are plain.

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
- 2026-11-19: T3 assessed in two read-only lanes: an inventory of the four
  decision sequences and of what the board knows about interfaces, and a
  stock 1.6.0 reading of seven differences the T0 table did not list. Both
  were checked here in part (see their Findings sections). Two rounds went to
  the owner: D21 to D24, then D25 to D27. The route-life finding (D25) adds
  phase T2a ahead of T3; a third read-only lane is listing stock's and
  Retinue's route-lifetime code for it. No code is written.
- 2026-11-19: T2a's inventory came back and was checked in part; its three
  forks were put to the owner and ruled (D28 to D30). T2a's code started in an
  opus lane on branch `transport-core-t2a` in
  `Code/worktrees/murm-transport-core-t2a`, cut from `d90113e`: four changes,
  failing test first, the same rule asserted in both drivers.
- 2026-11-19: the T2a lane committed its four changes on `transport-core-t2a`
  (`ecef260`, `53d734f`, `3f242e6`, `1bb946a`), no fork. Read here against the
  rulings: the shared rule, the board, the desktop, and the desktop's lock
  order (route-life checks read interface state while holding the path table;
  nothing takes the path table while holding the interface, policy or routing
  locks). `run_live.py` at `1bb946a`: 35 of 35. Two questions went to the
  owner (D31, D32). The lane's other readings stand as built: a
  shared-instance client that forwards still proves a route from a carried
  proof (stock has no reachable case); a board link refused by a full link
  table moves nothing; an announce hands on the used mark only from a live
  route; the own-send mark excludes announces. The lane is adding one commit
  for D32 and for test names that still state the old eviction rule.
- 2026-11-19: the lane's fifth commit, `e6c0cec`, decides a slow interface by
  `Shared::first_hop_airtime` (the configured bitrate, else the airtime
  allowance) against the airtime of one MTU at 25 kbit/s
  (`endpoint/paths.rs:84`), and renames two tests and one helper whose names
  stated the old eviction rule. Read here. T2a is 32 files, 1193 lines
  added and 209 removed; the test count rises by 22. 
  - Existing tests changed under D25 and D28: `a_route_in_use_is_refreshed`
    in both drivers (renamed `a_route_in_use_is_marked_not_extended`; the
    route is now gone at its lifetime, and a direct send marks it too);
    `return_paths_are_transactional_with_output` in both (asserts the used
    mark, not `learned`); `routes_touch_only_when_live` (renamed
    `routes_are_marked_used_only_when_live`);
    `open_link_addresses_the_first_relay` on the board (expiry moves from
    1 + lifetime to 5 + lifetime, because the link established at 5 proves
    the route); and the setup of
    `routes_live_a_week_unless_their_interface_mode_shortens_them` on the
    desktop (marks its routes used so the linger does not cut them).
  - Run here at `e6c0cec`: nextest 856 of 857 (the known timing test);
    no-alloc tests 58 of 58; thumb `alloc` check builds; `peer_rnsrs.py` 8 of
    8 against rns-rs `663e5e3`, rebuilt on the Unix host and removed after;
    T114 image builds; V4 image builds with `--ignore-rust-version`.
    `run_live.py` passed 35 of 35 at `1bb946a` and was not rerun for the
    fifth commit, which changes only the linger's slow test and names.
  - T114 image against T2 (`b4cf3b7`): text 376,670 to 377,630 bytes (960
    more), bss 138,812 to 139,772 (960 more).
  - Public surface: unchanged. The board's pending link holds its destination
    (private).
  T2a's done-condition is met on the branch. Not merged.
- 2026-11-19: the owner was shown the gates and three readings the lane built
  without a ruling (a forwarding shared-instance client extends a route from a
  carried, verified proof; a board link refused for a full link table moves
  nothing; a path response we send does not mark the route used) and said
  "Commit, merge, push." `main` fast-forwarded from `d90113e` to `e6c0cec`,
  the tree the gates ran on. The lane's worktree, branch and build target
  were removed.
- 2026-11-19: T3's code started in an opus lane on branch `transport-core-t3`
  in `Code/worktrees/murm-transport-core-t3`, cut from `113232d`. Part A
  converges in place, one commit each: the board's announce gate order (D26),
  carriage off on the desktop (D24), the board's interface table and relay
  fan-out (D2, D13, D21, D22, D33, D34). Part B moves the decisions: the
  shared functions alone, then `Node`, then `Endpoint`. The lane stops after
  Part A's first two commits, after the fan-out, after the shared functions,
  and at the end.
- 2026-11-19: the lane stopped with two commits and one fork. Read here
  against the rulings; nothing sent back.
  - `074b586`: the board's announce gate is duplicate memory, hop ceiling,
    size (`node/transit.rs:355`). No existing test changed.
  - `0f3be58`: the board's interface table (`KNOWN_INTERFACES = 4`,
    crate-private; learned in `ingest`, `poll`, `set_first_hop_airtime` and
    `set_interface_mode`) and relay fan-out. The owed interfaces live on the
    relay's entry in `crate::rebroadcast`; the desktop never sets them. Two
    board tests changed under D2: the ingress-interface relay test, renamed,
    now expects both interfaces and four counted sends; and
    `the_next_hop_is_the_route_at_release` expects two sends.
  - Consequences of keeping the remainder on the entry, as built: neighbours
    that end a relay end its remainder; a newer announce or the relay's own
    retry replaces it; a full table can evict it; it passes the mode rule and
    the live-route check again when offered. A relay heard on an interface
    with no table slot leaves only on the table's interfaces.
  - Lane gates at `0f3be58`: nextest 863 of 864 (the known timing test);
    no-alloc 58 of 58; thumb check builds.
  - The fork, carriage off on the desktop, was ruled as D35.
  - Found: `a_slow_interface_is_known_by_bitrate_else_airtime`
    (`endpoint/tests/route_life.rs`, from T2a, on `main`) learns five routes
    into the test table of four, so one is evicted by map order. Reproduced
    here: 3 failures in 10 runs, each "case 0". The lane fixes the test.
- 2026-11-19: the lane stopped after three more commits, with one fork.
  Read here against the rulings and against each driver's present order of
  checks; nothing sent back.
  - `1937d08`: the flaky test checks each case alone; 20 of 20 in the lane.
  - `2b818c2`: carriage off on the desktop (D24, D35), at
    `endpoint/router.rs:100` and the proof arm, with `Shared::peek_reverse`.
    No existing assertion changed; the two public tests pass as they are.
    Four relay tests moved unchanged to `endpoint/tests/transport_relay.rs`
    for the line ceiling.
  - `3a1930a`: the shared decisions in `transport/forward.rs` (386 lines),
    no driver changed: `admit_crossing`, `cross_gate`, `crossed`,
    `forward_hop`, `live_route`, `admit_request`, `unsent_request`,
    `reverse_proof`, `relay_gate`, `release_from`, `relay_targets`, returning
    `Transit` with a `Refusal`, or a `RelayRefusal`. Each function holds at
    most one table; a step on another table (the duplicate memory, the route,
    the egress selector) comes in as a closure, so the desktop never holds
    two table locks in an order it does not use today.
  - As built, not ruled: the size check and link-request MTU clamp on the
    board, and the link-MTU clamp and queue refusal on the desktop, stay with
    each driver as part of sending, between the decision and the send.
  - Lane gates at `3a1930a`: nextest 881 of 882 (the known timing test);
    no-alloc 68 of 68; thumb check builds.
  - The fork, an unsent link request with the bridge table full, was ruled
    as D36.
- 2026-11-19: the lane finished: eight commits on `transport-core-t3`, tip
  `00da22d`, 27 files under `crates/retinue/src/`, 2009 lines added and 548
  removed, no public item changed. Read here: the board's transit file, the
  desktop's bridged path and forward tail; nothing sent back.
  - `650422f` (D36): `record_bridge_undoably` returns what recording
    displaced and `undo_bridge` puts it back in place; the desktop's refused
    link request no longer loses the bridge it evicted or leaves a
    retransmission's replacement. Same-named test in both drivers,
    `a_refused_link_request_leaves_the_bridges_as_they_were`; the board's
    passed before and after.
  - `907a6f7`: `Node` onto the shared decisions. `evicted_bridges` counts
    once the send is accepted.
  - `00da22d`: `Endpoint` onto them. One route read per carried packet
    (`Shared::path_entry`, row b). The desktop's own `LinkBridge::admit`,
    `Crossing`, `accepts_transit_from` and `peek_reverse` are gone.
  - Where each decision is made and called. Bridge crossing: `admit_crossing`,
    `cross_gate`, `crossed`; `node/transit.rs:103`, `endpoint/router.rs:113`.
    Type-2 forwarding: `forward_hop`, `admit_request`, `unsent_request`;
    `node/transit.rs:197`, `endpoint/transit.rs:211`. Reverse proofs:
    `reverse_proof`; `node/transit.rs:342`, `endpoint/transit.rs:145`.
    Announce gate: `relay_gate` at receipt (`node/transit.rs:392`,
    `endpoint/announces.rs:438`), `release_from` and `relay_targets` at
    release (`node/rebroadcast.rs:121`, `endpoint/rebroadcast.rs:232`).
  - Left with the drivers, as built and not ruled: the board checks that the
    send will fit before recording a bridge, so a full action list with a
    table full of validated links still counts no `refused_bridges` (D23);
    the desktop's single send point keeps its egress and hop-ceiling checks,
    which the shared gates now run first and which one test calls directly;
    the size and MTU steps noted above.
  - Run here at `00da22d`: nextest 883 of 884 (the known timing test);
    no-alloc tests 68 of 68; thumb `alloc` check builds; `run_live.py` 35 of
    35 against stock RNS 1.6.0; `peer_rnsrs.py` 8 of 8 against rns-rs
    `663e5e3`, rebuilt on the Unix host and removed after; T114 image builds;
    V4 image builds with `--ignore-rust-version`.
  - T114 image: text 380,670 bytes, bss 139,844. Against T2a (`e6c0cec`):
    3,040 more text, 72 more bss. Against the tree before T2 (`5befe56`):
    6,056 more text, 1,176 more bss.
  T3's done-condition is met on the branch, with the three steps above left
  to the drivers. Not merged.
- 2026-11-19: the owner was shown the gates and the three steps left with
  the drivers and said "Merge, commit, push, proceed." `main` fast-forwarded
  from `113232d` to `00da22d`, the tree the gates ran on. The lane's
  worktree, branch and build target were removed.
- 2026-11-19: T4's inventory came back and was checked in part; its one
  fork was put to the owner and ruled (D37). T4's tests started in an opus
  lane on branch `transport-core-t4` in
  `Code/worktrees/murm-transport-core-t4`, cut from `22e7592`: the shared
  table tests at the real sizes, the shared tests still missing, then one
  name for each mirror. It changes tests only.
- 2026-11-19: the T4 lane committed three changes on `transport-core-t4`
  (`815bf54`, `e769e56`, `087950e`) and set one choice aside. Read here: the
  scope of each commit, the non-test diff, and the sized test macro.
  - Real sizes: 14 table bodies now run over the array at 4 and 16 and over
    the map at 4 and 4096; two array-only bodies at 4 and 16; the memory's
    full-generation body at 2, 32 and 32,768. The board's 16 is read from
    `Node`'s default parameters. 32 real-size tests; none failed. The bridge
    and return-path bodies moved to `transport/tests/bridges.rs` for the line
    ceiling.
  - The only non-test change: the desktop's three capacities and
    `PACKET_HASHES` gained names the shared tests can read
    (`DESKTOP_PATH_TABLE_CAPACITY` and two like it, re-exported under
    `cfg(test)`). Production values are unchanged.
  - Added shared tests: the announce cap's purge on interface removal
    (`rebroadcast/tests.rs:195`); a packet not addressed to this hop skips
    every transit check (`transport/tests/forward.rs:206`); and the neighbour
    count of two carrying across our first send (`rebroadcast/tests.rs:183`),
    a partial the lane found among the pairs the inventory called covered.
  - Named gaps, decided in each driver and left untested in the shared
    module. The order "bridge, then type-2 hop, then return-path proof,
    handled once": `node/ingest.rs:79`, `endpoint/router.rs:111` to `181` and
    `349`. A short link request's id falling back to the packet hash, and a
    link request recording no return path: `node/transit.rs:256`, `282`;
    `endpoint/transit.rs:248`, `261`. What makes a packet "addressed" to this
    hop: `node/transit.rs:190`, `endpoint/router.rs:168`. Which packet
    contexts skip the duplicate memory, and that admission precedes it:
    `node/transit.rs:422`, `endpoint/dedup.rs:135`. Which side of a return
    path an interface removal purges: `node/routes.rs:122`,
    `endpoint/transit.rs:169`. Minor: `reverse_proof` is not tested at the
    hop boundary itself.
  - Renames: 13 mirrors took one name each (8 files, name lines only). Names
    shared across the drivers went from 32 to 45. Twelve likely mirrors were
    left because one body asserts more than the other.
  - Run here at `087950e`: nextest 918 of 919 (the known timing test);
    no-alloc tests 69 of 69; thumb `alloc` check builds.
  - The choice set aside: the memory's equivalence test
    (`turning_over_late_answers_as_turning_over_early`) runs 500 inserts over
    6 keys at capacity 2 against a list model. Options: scale it with a
    set-based model; board size only; leave it at 2. The owner: "Scale it,
    set-based model" (applies D37). The lane is adding that as a fourth
    commit.
- 2026-11-19: the lane's fourth commit, `bc017a2`, scales the equivalence
  test: keys are three times the capacity; inserts are 250 per slot up to
  256 keys and 25 per slot beyond (250 per slot took 17.9 s at the desktop
  size); the reference model holds two hash sets and is written apart from
  `TransitMemory`; every run asserts at least eight turn-overs. At capacity 2
  it is the same 500 inserts as before. Runs: array 2 and 32, set 2 and
  32,768 (819,200 inserts, 12 turn-overs, 2.1 s here). The lane broke the
  model's turn-over condition by one insert and all four runs failed, then
  restored it.
  - Run here at `bc017a2`: nextest 920 of 921 (the known timing test);
    no-alloc tests 69 of 69; thumb `alloc` check builds.
  - Counts for T4's done-condition. Removed: none. Kept: every driver test
    (`node::tests` 144, `endpoint::tests` 125 by the lane's listing, which
    leaves out test modules inline in source files). Renamed: 13. Added to
    the shared module: 3 bodies, 46 to 49; expanded over storage and size, 72
    to 109 tests.
  T4's done-condition is met on the branch: four commits, tip `bc017a2`. Not
  merged. It changes no non-test code beyond naming the desktop's table
  sizes, so the board image and the wire behaviour are those T3's gates ran
  on at `00da22d`.
- 2026-11-19: asked whether to close on T3's interop results or rerun the
  live suite, the peer gate and the firmware builds on the T4 branch first,
  the owner chose the first. `main` fast-forwarded from `22e7592` to
  `bc017a2`. T5 is recorded under its phase. The lane's worktree, branch and
  build target were removed. The plan is done.
