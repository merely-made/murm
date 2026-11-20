# Wall node and carrier-agnostic management: remaining work

**Status (2026-11-20): in progress, WN2 software half; the node image builds and is on main, nothing has run on a board.** Adopted as a plan on 2026-11-02. The [original wall-node plan](archive_docs/2026-11-02/2026-08-30_wall_node_management_plan.md) retains its decisions, WN0–WN8 history and scoped evidence. The [coordination agreement](2026-08-09_retinue_work_lanes.md#coordinated-appliance-listening-and-observation-work-2026-09-06) governs shared interruption and observation; the magnetic pocket-node plan reuses the control contracts without replacing this V4 appliance work.

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
  node reads the flags in force from the radio owner (D9).
- **WN2c, relay from the config (D2).** The node build accepts
  `relay_announces` and `relay_packets`, applies them at boot and on a live
  change, and relays both on an unclaimed board. *Done when* each flag alone
  and both together are tested against the node's transport settings. The
  modem-only image keeps refusing a committed config with a relay flag on and
  names the cause (D4). The V4 node does not echo resources (D5) and refuses
  resources addressed to itself (D8).
- **WN2d, the heap figure.** Build with a provisional heap, run the flood on
  the bench, and choose the heap size and its refusal boundary from the
  measured peak. The number is the owner's.
- **WN2e, the bench run (D3).** The archived plan's physical procedure, steps
  5 and 6, with the [transit receipt spec](2026-08-14_transit_link_receipt_spec.md)'s
  relay-off control before and after the success. This closes WN2's
  done-condition above.

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
