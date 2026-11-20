# Wall node and carrier-agnostic management: remaining work

**Status (2026-11-20): in progress; WN2's software half (WN2a to WN2c) is built and host-tested, and the node image boots and announces on one V4 with nothing on the air to hear. The heap figure and bench run (WN2d, WN2e) are open.** Adopted as a plan on 2026-11-02. The [original wall-node plan](archive_docs/2026-11-02/2026-08-30_wall_node_management_plan.md) retains its decisions, WN0–WN8 history and scoped evidence. The [coordination agreement](2026-08-09_retinue_work_lanes.md#coordinated-appliance-listening-and-observation-work-2026-09-06) governs shared interruption and observation; the magnetic pocket-node plan reuses the control contracts without replacing this V4 appliance work.

## Phases and done-conditions

1. **WN0 contract.** Execute the same versioned bounded semantic fixtures on host and firmware target. Unknown versions and oversized input fail closed, authority matches actual dependencies, and no carrier/UI detail enters the semantic payload. Cross-compilation is not target execution.
2. **WN1 durable control and first owner.** Physical Claim and Resume have a bounded receipt in the original; Abandon was not run. Preserve and qualify safe Abandon, corrupt/pending recovery and status-only uncertainty boundaries without repeating the completed claim merely to close this plan. On the claimed V4 use the granted controller to apply and commit a legal candidate, power-cut and verify it; separately revert a candidate and prove autonomous expiry without a host frame. Require replay refusal across power loss, a fresh counter succeeding, effective clamped PHY and one provenance-bound receipt. Complete board credential vault/sealing, cross-carrier commit and target quiet-witness proofs. Choose and protect the V4 vault range, add an authoritative build-time application/partition overlap guard, and check T114 package preservation of its separate control pair. Reconcile the first-owner controller public-scope create-new write interruption issue with its current owner. Every erase, write, apply and cleanup cut must recover last-known-good; replay, bad controller/node, stale generation, expiry and inadequate recovery quorum must refuse. Same-transaction retry returns the durable cached result under a fresh signed outer counter without repeated work. Secrets and commit tokens stay out of public evidence.
3. **WN2 standalone V4 Reticulum.** Make interruption one board-owner lifecycle using the existing quiet guard; stop/drain, guarded work and a single RX restore or reset-required outcome. Assemble a named native node with bounded allocation, durable identity and reservation before emission; retain modem and RNode recovery. Done when a three-party direct-PHY path and payload work in both directions with the controller absent, again after power cycle, with a relay-absent negative control, forwarding facts, allocator peak and refusal figures. Do not infer transit from three devices merely being present.
4. **WN3 WiFi.** Add configured TCP client/server, sealed credentials, lifecycle and a distinct Auto-discovery receipt. Done when a V4 transports between stock WiFi and LoRa peers without its controller, rejoins without exposing credentials, tolerates either interface's loss/return, and records loop/path behavior plus exact coexistence memory, interrupt, power and packet budgets.
5. **WN4 carriers.** Route the same semantic requests through USB, IP and authenticated Reticulum Link/Resource without a second grammar. Done when equivalent fixtures pass, a change starts on one carrier and commits on another, unconfirmed changes revert, and two-site repair succeeds while a routed path survives. Credential-bearing operations require confidentiality; raw broadcast and plain TCP refuse them. The optional iroh host probe is only a simulated-board result: before device use choose an evidenced device profile, bind a device endpoint key without a Persona root, expose a prepared-request/retry seam if needed, and prove loss, deadline, power, RF coexistence and resource limits. Gateway authentication is not board authentication.
6. **WN5 BLE bootstrap.** On a factory-reset V4, claim and join WiFi from a laptop and real phone; report join result over the live BLE session and reconnect. Done when loss, permission denial, bonding deletion, Bluetooth-off and restart have explicit results, removing BLE does not stop the node, and combined BLE/WiFi/LoRa capacity is measured.
7. **WN6 image selection.** Linkboy authenticates packages and approves immutable plans; bearer delivery does not become activation authority. Done when a candidate reaches an inactive V4 slot through two carriers, trial-boots and commits on health, while wrong identity, silence, timeout, replay/downgrade and power loss return to confirmed image, with physically proven ROM cable recovery.
8. **WN7 resident adapters.** Retinue, Tucket and Sennet retain separate protocol/routing authority; RNode remains exclusive. Done when at least Reticulum and one exact-wire foreign adapter share a measured lease/scan plan, malformed input cannot extend a lease, off-time/miss rates are measured, and a second radio or board proves continuous coverage where one radio cannot. UI distinguishes bearer, exclusive mode, native route and semantic gateway.
   *Withdrawal note, 2026-11-14 ([protocol_parity D1](2026-11-02_protocol_parity_plan.md#protocol_parity-d1--foreign-meshes-through-stock-radios)):* Mark ruled "Stock radios only"; "the resident-protocols firmware image and WN7's foreign-adapter condition are withdrawn." The "one exact-wire foreign adapter" condition no longer applies; foreign meshes are reached through a stock companion radio, one per mesh. What remains of WN7 without a foreign adapter is not decided here, and its status is unchanged.
9. **WN8 profiles.** Qualify standalone claim/join, unattended LoRa-to-WiFi, restart, remote inspection and rollback as WN8a; image trial/rollback/power-cut/recovery as WN8b; multiplex and coverage as WN8c. Each exact artifact claims only its earned profile. WN8a does not depend on asserting WN8b/c.
   *Note, 2026-11-14:* WN8c's multiplex was to be qualified on the withdrawn resident image ([protocol_parity D1](2026-11-02_protocol_parity_plan.md#protocol_parity-d1--foreign-meshes-through-stock-radios)). Whether "multiplex" here meant foreign protocols or runtime personalities in general is not settled by the ruling; the status is unchanged.

## WN2 execution (2026-11-20)

WN2 runs in the order below. The shape follows the archived plan's
[smallest-appliance sequence](archive_docs/2026-11-02/2026-08-30_wall_node_management_plan.md#current-smallest-appliance-sequence-2026-09-06),
steps 2 to 4; its step 1 (WN1's apply, commit and revert with the granted
controller) is not part of this work. Kept from that sequence: the node is a
named build on the USB image and the modem image stays without a heap; the node
is `Node<32, 8, 4>` behind the shared `NodeChannel`; modem and RNode stay as
recovery channels; no second flash guard; no change to durable bytes, control
wire bytes or the T114 loop. The bench run uses stock RNS 1.6.0, the current
pin, where the older text says 1.5.2.

- **WN2a, the node build.** A `node` feature on the USB image adds the
  allocator, `radio-hand/node` and `retinue/alloc`. The boot takes the announce
  lease before any radio service, builds the node from the stored identity, and
  enters a node loop on `V4RadioOwner`. `channel node` is accepted in this build
  and still refused in the modem build. Status reports heap in use, heap peak,
  failed allocations, unsent frames and the node's forwarding figures.
  *Done when* the modem image still links no allocator, the node image builds
  for the board, new pure code has host tests, and the T114 image is unchanged.
  *Built 2026-11-20*, with relay from the config at boot (WN2c's first half);
  see Progress. Still owed here: the failed-allocation figure in the `heap`
  reply, and a measured stack peak while the node is constructed.
- **WN2b, control in the node loop (D1).** The node loop answers the status
  request and serves signed commands through the existing `QuietWindow`, and
  waits on the provisional deadline as the modem loop does. *Done when* a host
  model shows no radio action during guarded work and exactly one receive
  restore or reset on every exit and cancellation point. A live relay change
  takes effect on the next packet (D6); after a live profile change the
  first-hop allowance is recomputed from the profile then running. Both loops
  share one carrier in `radio-hand`, with host tests of its ordering (D7); the
  node reads the flags in force from the radio owner (D9). *Built 2026-11-20*;
  see Progress for what the model covers.
- **WN2c, relay from the config (D2).** The node build accepts
  `relay_announces` and `relay_packets`, applies them at boot and on a live
  change, and relays both on an unclaimed board. *Done when* each flag alone
  and both together are tested against the node's transport settings. The
  modem-only image keeps refusing a committed config with a relay flag on and
  names the cause (D4). The V4 node does not echo resources (D5) and refuses
  resources addressed to itself (D8). *Built 2026-11-20.*
- **WN2d, the heap figure.** Build with a provisional heap, run the flood on
  the bench, and choose the heap size and its refusal boundary from the
  measured peak. The number is the owner's.
- **WN2e, the bench run (D3).** The archived plan's physical procedure, steps
  5 and 6, with the [transit receipt spec](2026-08-14_transit_link_receipt_spec.md)'s
  relay-off control before and after the success. This closes WN2's
  done-condition above. The unclaimed V4 runs it first and the commissioned
  one second (D12); the payload and hosts are D13 and D14.

### wall_node_management D1 to D3 (2026-11-20)

- **D1, control while the board is a node (new):** signed commands are served
  only by the modem loop today (`firmware/heltec-v4-phy/src/main.rs:903`); the
  RNode loop serves none. Asked whether node mode keeps serving them, the
  owner: "Commands and status". What a relay change does to carried links is
  D6.
- **D2, where the relay setting comes from (new):** the T114 turns relay on in
  code (`firmware/t114-phy/src/main.rs:134`); the control config already
  carries two flags the V4 refuses (`radio_owner.rs:180`). The owner:
  "Config; on when unclaimed". A claimed board's committed config has both
  flags off, because the image refused anything else, so the bench relay run
  needs a controller apply and commit first.
- **D3, keeping the end radios apart on the bench (new):** the owner:
  "Attenuation". The other ways in the transit receipt spec, distance and a
  labelled logical filter, are not used for this run.

### wall_node_management D4 to D6 (2026-11-20)

- **D4, the modem-only image and a relay-on config (new):** flashed onto a
  board whose committed config has a relay flag on, the modem-only image fails
  its boot-time apply (`radio_owner.rs:583`, through
  `crates/radio-hand/src/control/runtime.rs:312`) and serves only the
  status-only loop with fault 12; the way out is reflashing the node image.
  Inside the node image `channel modem` still works. The owner: "Refuse, say
  why". The status line names the cause and the fix; the image still never
  runs a config it cannot honour.
- **D5, the echo service on the V4 node (new):** the shared node channel
  echoes each completed inbound resource back on its link
  (`crates/radio-hand/src/channel/node.rs:221`), which on a wall relay lets
  any peer that opens a link spend the relay's airtime and heap. The owner:
  "Off on V4, per-board setting". The T114 keeps it on. Refusing the inbound
  transfer itself is D8.
- **D6, a live relay change and carried links (refines D1):**
  `set_transport_config` swaps the policy and keeps routes and bridges, and
  the three forwarding paths check the flag per packet
  (`crates/retinue/src/transport/forward.rs:116,200,290`). The owner: "Stop at
  once". Carried links stop on the next packet and their endpoints time out; a
  provisional apply reverted inside the bridge lifetime resumes them. No
  Retinue change.

### wall_node_management D7 to D9 (2026-11-20)

- **D7, where the signed-command carrier lives (new):** the carrier that
  verifies and serves signed commands is 162 lines in the V4 firmware
  (`firmware/heltec-v4-phy/src/control_carrier.rs`), which no host test
  reaches; the quiet window under it has eight host tests in `radio-hand`.
  Moved into `radio-hand`, it cost the modem image 388 to 428 bytes of
  374,267 in the lane's two drafts. The owner: "Move it to radio-hand". Both
  loops call the one carrier, and its ordering is tested on the host with a
  fake board, including drops at entry, apply and finish.
- **D8, resources addressed to the V4 node (refines D5):** with echo off the
  node still reassembled any resource sent to it; neither board sets a limit,
  so Retinue's default of 64 MiB applied (`crates/retinue/src/node/setup.rs:75`)
  against a 96 KiB heap. `set_max_inbound_resource`
  (`crates/retinue/src/node/resources.rs:127`) refuses an offer before
  reassembly and counts it. The owner: "Refuse all, per-board limit". The
  limit is a per-board setting beside echo, 0 on the V4; WN4's Reticulum
  carrier raises it when it needs resources. The T114 is unchanged.
- **D9, where the node reads the config in force (new):** Seneschal keeps a
  provisional candidate's config private
  (`crates/seneschal/src/control/durable/model.rs:167`); the V4 radio owner
  already keeps the profile of the last config it applied, and boot, apply,
  revert and expiry all pass through its apply. The owner: "The radio owner".
  In the node build it also keeps that config's relay flags. No Seneschal
  change.

### wall_node_management D10 and D11 (2026-11-20)

- **D10, how the node image is flashed (new):** the package rules make a
  Linkboy plan the only path that may write a board
  (`firmware/packages/README.md`), and the one V4 template builds the modem
  image. The owner: "Own template, not in the public index". A
  `retinue.heltec-v4-node` template with the same preserved range, built by
  the staging script, listed in a bench index until the bench run passes.
- **D11, the bench boards (new):** asked whether the three attached boards'
  serial ports could be opened for read-only probes, the owner: "Consider
  those free game to change. Nothin' sacred about them, they're the testing
  boards. I plan to give one to a friend later but you can do a week of tests
  with them both and the t114, at least." Both V4s (USB parents ending FB:28
  and FA:64) and the T114 may be flashed, reset and reconfigured for this
  work. Secrets and commit tokens still stay out of public evidence.

### wall_node_management D12 to D14 (2026-11-20)

- **D12, which V4 relays in the bench run (new):** the archived procedure
  names the commissioned board (FB:28, SB1), whose committed config has relay
  off, so it needs the controller's apply and commit first, never yet run on
  hardware; the other (FA:64, SB2) is unclaimed and relays under D2. The
  owner: "SB2 first, then SB1". SB2 gives forwarding, the power cycle and the
  relay-off control; the SB1 run with the controller is the one that closes
  WN2 as the procedure words it.
- **D13, the held-out payload (new):** the owner: "Random packet each way".
  256 random bytes generated at run time in each direction over a link
  through the relay, with the sha256 logged by sender and receiver.
- **D14, where the two endpoints run (new):** the owner: "Both on this
  laptop". Two stock RNS 1.6.0 instances with separate config directories, no
  shared instance, no AutoInterface or IP interface, one RNode each. The
  relay-off control before and after the success is what shows they cannot
  reach each other another way.

### wall_node_management D15 (2026-11-20)

- **D15, who the endpoints are (refines D13 and D14):** D13 and D14 were put
  as two stock RNS endpoints on RNode-channel radios. That cannot reach the
  node as built: the RNode channel transmits with sync word 0x12 and an
  8-symbol preamble (`crates/radio-hand/src/rnode/pending.rs:12`), and the
  node runs the board's default profile, Meshtastic LongFast with sync 0x2B
  (`firmware/heltec-v4-phy/src/main.rs:372`). On the bench, RNS 1.6.0 through
  SB1 sent three announces; the node on SB2 logged one bad frame and decoded
  none, and RNS heard none of the node's. The owner: "Retinue on SB2, stock
  RNS on SB1". The SB2 run uses Retinue desktop endpoints through direct-PHY
  modems, the [transit receipt spec](2026-08-14_transit_link_receipt_spec.md)'s
  fixture; in the SB1 run the controller's apply sets the RNode air profile
  together with relay-on, and stock RNS 1.6.0 on RNode-channel radios is the
  endpoint. D13's payload and D14's single host stand for both runs.

### wall_node_management D16 (2026-11-20)

- **D16, how the payload crosses (refines D13):** D13's 256 random bytes do
  not fit one link packet on direct PHY: a radio frame is at most 255 bytes
  (`crates/selvage/src/wire.rs:35`) and a link packet carries about 175 after
  its header and encryption. Asked whether they cross over the link's stream,
  the owner: "Yes, the stream". 256 random bytes each way over the endpoint's
  stream, two frames per direction, sha256 logged by sender and receiver.

### wall_node_management D17 and D18 (2026-11-20)

- **D17, the controller for the commissioned-board run (new):** no host tool
  can send relay-on or a chosen radio profile.
  `apps/signalman/src/bin/signalman-control-configure.rs:116` builds its
  candidate from `postilion::profile` with relay off, and reads the raw key
  from an environment variable; the granted Mere controller key is released
  by djinn inside `station-custody` and never exported, and
  `crates/station-custody/src/bin/station-claim.rs:284` has only `init`,
  `claim` and `status`. The library is complete
  (`crates/postilion/src/control/verified/client.rs:171`). The owner: "Add it
  to station-claim now". Apply, commit and revert under djinn custody, with
  the relay flags and every radio field as arguments. This is WN1's open
  applied-and-committed receipt, taken up here because the SB1 run needs it.
- **D18, the commit token between apply and commit (new):** the procedure
  keeps it in protected controller state and out of receipts; today's tool
  prints it. The owner: "In memory, one invocation". One command applies,
  shows the status evidence and commits, reverts or lets the candidate
  expire before it exits. The holder is a seam so a store can be added when
  a commit on another carrier needs one.

## Decisions and constraints

Transport authentication does not confer board-local authority. Grants bind actual keys; durable outer counters and mutation-sequence retirement are separate from semantic transaction identity. `Changed`, `Replayed`, `Ready` and reset-required preserve distinct meanings. Board-owned recovery facts cannot be asserted by candidate configuration. A signature does not encrypt credentials. A quiet witness must cover live flash and apply; once stopping starts, cancellation or uncertain hardware state fails to a reset boundary. The first useful appliance is USB lifecycle then native LoRa forwarding, with WiFi afterward; WN0 target fixture and later carriers are real debts but are not asserted as blockers for that integration.

## Findings (2026-11-02)

`crates/radio-hand/src/control/runtime.rs:24,116` defines the control runtime and provisional deadline; `crates/radio-hand/src/channel/node.rs:62` defines a node channel. The original records a physical claim/resume and durable signed Status including replay refusal across power loss; these are scoped evidence, not lifecycle mutation, native forwarding, additional carriers, vault or appliance-profile acceptance.

## Deferred and follow-on

Choose V4 heap from measured high-water and refusal behavior. Decide unsigned, bounded public diagnostics without consuming signed status journal endurance. Establish target behavior when expiry overlaps TX, RX or lease handoff. Select the held-out three-party fixture before flashing. T114 owner/stop-and-drop, BLE/phone, WiFi/IP/Reticulum, and full credential sealing remain separate. Production iroh device integration and a prepared semantic-request retry seam are optional follow-ons; the host probe does not establish them. Reconcile the current stock RNS pin at execution rather than silently changing historical receipt identity.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
- 2026-11-20: WN2 started after the shared transport core landed. Assessment: the
  V4 already has the identity store, announce-lease storage, one radio owner
  with the quiet guard and an RNode loop on the shared channel code; missing
  are the allocator, a node loop, the boot lease and the status figures. D1 to
  D3 ruled. Attached to the dev machine: one V4 and one T114; the second V4 is
  not attached. No code yet.
- 2026-11-20: WN2a and the boot half of WN2c built on a lane, checked in its
  worktree and merged. The tracking heap is shared through `radio-hand`'s
  `node` feature; the V4 `node` build takes the announce lease before receive
  is first armed, keeps the node in static memory (25,304 bytes) and bounds
  every host write at 100 ms, closing the host side after one that runs out.
  Sizes, text bytes: V4 modem 374,107 (16 fewer), T114 380,662 (8 fewer), V4
  node 609,767. The modem image's dependency tree has no allocator and
  `retinue` without `alloc`; the node build's has both. `radio-hand` tests:
  140, and 157 with `replay`. Not run: the V4 firmware's own tests (esp-hal
  does not build for the host) and anything on a board. Unmeasured: the stack
  peak while the node is constructed, against about 155 KiB of main stack. An
  unclaimed board waits out the 20 s claim window at each boot before the node
  starts. `validation/security/unsafe-policy.toml` does not yet list the moved
  heap. D4 to D6 ruled.
- 2026-11-20: three pieces of the second checkpoint built, checked and merged.
  The modem-only image's status-only line names a relay-on config and the fix
  (D4; 160 bytes). Echo is a per-board setting, on for the T114 and off for
  the V4 (D5; the T114 image is 24 bytes smaller). The V4 `heap` reply ends
  `failed=<count>`. The unsafe audit's entries for the four heap files are
  current; it still prints three exceptions with no unsafe left
  (`firmware/stack_watermark.rs` and the two `protocol_capacity.rs` examples)
  and `crates/truchman` without `#![forbid(unsafe_code)]`, which this work did
  not touch. Sizes, text bytes: V4 modem 374,267, T114 380,638, V4 node
  609,903. `radio-hand` tests: 140, and 160 with `replay`. Not built yet:
  signed commands in the node loop and the live relay change, which stopped
  at the fork D7 settles. Not tested: the new status line (the V4 firmware's
  tests do not run on the host). D7 to D9 ruled.
- 2026-11-20: WN2b and the rest of WN2c built on the lane and checked in its
  worktree. The carrier is `radio_hand::control::carrier` behind a
  `ControlBoard` trait (quiet readiness, commit entropy); the V4 file is its
  adapter, and the modem image grew 392 bytes. The node loop takes a KISS FEND
  at a line boundary as the start of a control frame, answers the status
  request from the boot snapshot, serves commands through the carrier, and
  waits on the provisional deadline as a fourth branch of its wait. After each
  served command or expiry it follows the owner's last applied config (relay
  flags and first-hop allowance); boot reads the same place. The inbound limit
  is a per-board setting, 0 on the V4. Host tests added: six on the carrier
  (preflight before the verifier advances, token before verification, a
  runtime error discards the verifier, silent refusals, one reset on a drop at
  entry, apply and finish, expiry waiting for a quiet boundary), four on node
  work around the quiet window, three on a running node following the flags
  and a revert, two on the inbound limit with its positive control. In the
  node tests the send, arm and collect are fakes that follow the V4 owner's
  rule; the carrier and runtime are the real code. What the loop does with a
  deadline that falls inside a send or a collect: nothing races either, so the
  rollback runs on a later turn, late by that operation, by any host read or
  frame already waiting, and by 200 ms for each attempt the radio was not
  quiet for; no reset. Sizes, text bytes: V4 modem 374,659, T114 380,750 (112
  more, the explicit default limit), V4 node 612,411; the UART image builds.
  `radio-hand` tests: 150, 175 with `replay`. Not run: the V4 firmware's own
  tests, and anything on a board. Both V4s and the T114 are attached to the
  dev machine. The node image has no Linkboy package yet, which the package
  rules require before a write.
- 2026-11-20: the node image has a Linkboy package (D10):
  `firmware/packages/heltec-v4-node.template.toml`, a bench index and a
  recovery page; the staging script builds it with `--features node`, and the
  public index and the modem package's staged bytes are unchanged. Staged from
  `90ae4030342` (11,132,452-byte ELF, sha256 `30bd2647...77e4`, 682,576 bytes
  written) and flashed through Linkboy to the second V4 (USB parent ending
  FA:64, name SB2) with the pinned espflash 4.5.0: write range
  `0x0..0x3f0000`, preserved `0x3f0000..0x400000`. First run on a board:
  status reports `build=90ae403... image=v4-node-usb` with the identity, name
  and reservation it had before; after `channel node` and a boot with no host
  attached it reports `channel=node`, the reservation advanced by one lease
  (1,179,648 to 1,245,184), one announce sent, `transport=1` (an unclaimed
  board, D2), and `heap=0/98304 highwater=332 failed=0`, unchanged four
  minutes later. Nothing else was on the air, so nothing was received or
  forwarded: this is a boot, not a relay result. Found on that boot and fixed:
  the `node` status line was cut off at `timebase=` on every board, because
  its names alone are 239 bytes and its buffer was 224; the buffer is 384 and
  the T114 image is 264 bytes larger. The board still runs the image without
  that fix. The first V4 (FB:28, SB1) is on the modem image in the RNode
  channel; the T114 is on its native image in the modem channel and reports a
  watchdog reset with one crash record, not looked into.
- 2026-11-20: the board was reflashed from `ebb91e2bd44` (sha256
  `2fad0183...bde11`) and reports the whole `node` line. Each boot as a node
  takes one 65,536 lease: the reservation stood at 1,507,328 after five.
  The comment at `crates/radio-hand/src/rnode/pending.rs:9` says every
  direct-PHY profile uses sync 0x12; the boards report 0x2B. D15 ruled.
- 2026-11-20: two-radio check, not a relay result.
  `crates/retinue/examples/node_link.rs` on the desktop, through SB1 in the
  modem channel, heard the SB2 node's boot announce (167 bytes at -39 dBm),
  sent a link request and received the proof (118 bytes at -38 dBm): one run
  of one passed in 35.3 s. The node then reported `tx=2 rx=1 links=1`,
  `heap=100/98304 highwater=404 failed=0` and the face event `link up`.
  At -39 dBm across the desk, 60 dB on each end radio leaves the relay about
  -99 dBm from each end and puts the ends 120 dB further apart. The owner has
  the antenna cables the boards came with and no attenuators yet.
- 2026-11-20: the direct-PHY fixture exists on the lane branch (`7ba04ada971`,
  not yet merged): `crates/retinue/examples/transit_peer.rs`, a responder and
  a requester aimed at the responder's destination, and
  `testing/wn2_transit_bench.py` with `control`, `run` and `collect` phases.
  Run on the three boards with no attenuation, T114 and SB1 as modems. None of
  it is a relay result. The relay-off control failed as it should with the
  ends in earshot: link in 2.1 s on `hops=0 next_hop=direct`, the responder
  accepting one link, 256 bytes each way over the stream with matching sha256
  at both ends (D16). With the relay on, the node heard the traffic and
  rebroadcast the responder's announces (`fwdannounce` 0 to 3, `peers=1
  routes=1`), `fwdpacket` stayed 0, and the requester again linked directly.
  The requester never reported a relayed copy of the announce; why is not
  established. No test anywhere runs an endpoint through the bounded node to
  another endpoint: `endpoint_multi.rs` has endpoint, transport endpoint,
  endpoint, and `node/tests/routes.rs` has three nodes. A host test of that
  path is being written before the attenuators arrive.
- 2026-11-20: the via-relay path has a host test on the lane branch
  (`f47ade7037e`, `crates/retinue/tests/transit_relay.rs`, three tests, 25 s):
  a requester that cannot hear the responder learns it only from the bounded
  node's rebroadcast, links through it and exchanges the D16 payload with
  matching hashes while the node's forwarded-announce and forwarded-packet
  counters move; with both relay flags off it fails and the responder accepts
  nothing; and a requester that also hears the responder directly drops the
  node's copy as a replay (`crates/retinue/src/endpoint/announces.rs:361`) and
  stays direct, which is what the bench showed. The lane reads stock RNS 1.6.0
  as ignoring that copy too, and reports one thing not run down: one
  `announce()` call on the responder put two announce frames on the air.
- 2026-11-20: first heap figures for WN2d, from `node_stress` through SB1
  against the SB2 node, 96 KiB heap. Three 40-announce waves (96 of 120
  heard) filled the tables, `peers=32 routes=16` with 80 route evictions:
  27,016 bytes in use, high water 27,312, no failed allocation. Four links
  held and the rest refused, on a fresh boot: 400 bytes in use, high water
  764. Not measured: carried links and forwarded packets, which need the two
  ends isolated. The heap size is not chosen.
- 2026-11-20: the fixture's double announce was its own: `Endpoint::register`
  announces by design and the responder announced again at once. Fixed in
  `transit_peer.rs` and the host test (`390217f0996`); not rerun on the
  boards. `link_peer.rs` has the same pattern and was left.
- 2026-11-20: read-only assessment of the SB1 run, spot-checked against the
  code. The board side is ready: a public configuration carries frequency,
  bandwidth, spreading factor, coding rate, preamble, sync word and power
  (`crates/selvage/src/profile.rs:10`), nothing restricts the sync word, and
  an apply reprograms all of it (`crates/radio-hand/src/service.rs:42`). Our
  RNode channel puts host data on the air verbatim and drops anything over
  255 bytes (`crates/radio-hand/src/channel/rnode.rs:198`), so its frames
  decode in the node once the profiles match; stock RNode hardware, which
  adds a leading byte, would not. Open for that run: the profile itself; the
  status banner and `sync` probe report `2b` whatever is applied
  (`firmware/heltec-v4-phy/src/channels.rs:273`); signed Status carries no
  radio fields; a refused profile earns silence; and stock RNS on a radio
  interface keeps a 500-byte link MTU, so D16's 256-byte write may exceed one
  255-byte frame there. D17 and D18 ruled.
