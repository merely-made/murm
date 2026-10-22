# I2P over SAMv3: retinue's I2P interface and the embedded router

**Date:** 2026-10-21. **Status:** in progress (2026-10-22). I2P1 is
implemented and software-qualified on branch `i2p-sam`, not merged. The
`truchman` 0.1.0 publish and I2P0's venv re-sync remain. I2P2-I2P4 have not
started.

Related: Mere's reachability rungs and privacy lanes,
`mere/design_docs/mere_docs/implementation_strategy/2026-08-03_reachability_rungs_and_privacy_lanes_plan.md`
(R3),
[RNS 1.5.7 interface review](2026-10-09_rns_157_interface_review.md) (A2, A5),
[Reticulum License adoption](2026-10-05_reticulum_license_adoption.md),
[protocol parity](2026-10-16_protocol_parity_plan.md).

## Why

Mere's R3 names three emissary consumers. The third is "Retinue over I2P":
upstream RNS ships an `I2PInterface`, so a retinue I2P carrier makes WAN
links private and also interoperable with existing Reticulum-over-I2P
transport nodes. Since the family move, that consumer lives here. R3's other
two consumers (the netDB rendezvous receipt and turnstone's `i2p://` lane)
stay in their own repos. They are named here only because the SAM client
chosen below should be one they can reuse.

Today `[[I2PInterface]]` in an RNS config is refused with
`RnsConfigError::UnsupportedType` (`crates/retinue/src/endpoint/iface_config.rs`
65-67).

## Owner answers (2026-10-21)

These came from a question round. They are answers, not numbered rulings.

- **Shape:** "Build the SAMv3 interface seam in murm first, with emissary
  embedded as one provider behind a feature flag and an external i2pd/Java
  router as the other. Arti stays deferred."
- **SAM client:** "any reason to fork yosemite or build on it? if not, we have
  the option if it becomes not maintained, right? let's examine it and if it
  suits our needs and is maintained, we can take it as is, and if not, sounds
  like a good start to a samv3 + datagram crate of our own." The examination
  is under Findings. Whether it passes is fork F1.
- **First live router:** embedded emissary first, "but feel free to install
  i2pd as well. unless it's prohibitively big".
- **Embedded router transit:** the host must choose. Transit on/off, its cap,
  and publish on/off are explicit host inputs, not inherited defaults.
- **RNS source:** the owner confirmed model training is off, and
  `I2PInterface.py` may be read and adapted into retinue under the 10-05
  license rules.

Second round, same day, after the Findings below:

- **F1, SAM client:** "Own crate now". Our own SAMv3 + datagram crate,
  MPL-2.0, not RNS-derived, with yosemite as a reference. It still needs a
  name and a location.
- **F2, emissary source:** "crates.io 0.4.0". The fork stays available in
  case a patch is ever needed.
- **F4, keyfiles:** "Mirror RNS". Same `storagepath/i2p/<hash>.i2p` layout
  and old-format preference.
- **i2pd version:** "3.0.0" (released 2026-10-20).

Naming round, 2026-10-22. Recorded with the checked registries in the
workspace naming ledger.

- **SAM client:** `truchman`, the old English word for an interpreter or
  go-between. Publish tier, MPL-2.0. Free on crates.io (sparse index and API)
  that day; unclaimed.
- **Provider:** `murm-i2p`, plain, support tier. `legation` is banked in case
  it ever publishes. Owner, same day: "Legation sounds good for a fork name if
  we gotta rename the emissary fork". If F2's crates.io dependency ever has to
  become a published patched fork, the fork takes `legation`.
- **Locations, proposed and not yet confirmed:** `crates/truchman` in the radio
  workspace (retinue takes it behind feature `i2p`), and
  `native/crates/murm-i2p` beside murm and mere-transport.
- **Claiming `truchman`:** with a real 0.1.0 publish once I2P1's SAM client
  passes its fake-bridge tests, the mora precedent, not a stub now. This
  makes the publish an I2P1 done-condition, and it needs the owner's go at
  that point.

## Architecture

The seam is SAMv3 over loopback TCP. It works with either provider because
neither embedded nor external routers expose anything else to applications.

```
retinue (Reticulum License)              provider (host's choice)
  Endpoint::attach_i2p / listen_i2p        ├─ embedded emissary-core: SAM on 127.0.0.1:<os port>
    └─ SAM client: session, stream ───────►└─ external i2pd / Java I2P: SAM on 127.0.0.1:7656
         └─ pump::carry(Framing::Hdlc, read, write)
```

- **Carrier.** The I2P interface is the existing stream carrier with a
  different socket. `pump::carry` already drives any reader/writer pair with
  HDLC framing for TCP and Pipe (`endpoint/pipe.rs` 99, `endpoint/dial.rs`
  `supervise`). The I2P work is the session lifecycle, supervision, keepalive
  and config, not a new framer.
- **Licensing.** The RNS-adapted behaviour (config keys, keepalive, keyfile
  naming, reconnect) lands in retinue only, per rule 3 of the license
  adoption. The SAM client is not RNS-derived. Whichever we take, it stays
  outside retinue's license so mere and turnstone can use it.
- **Provider placement.** The embedded router is a host concern. Embedding
  makes the host an I2P participant, so it never links into retinue itself.
  It lives in its own default-off crate that returns a SAM address. Name and
  location are fork F3.

## Findings (2026-10-21)

**RNS `I2PInterface.py`.** Read at **RNS 1.5.6**: the oracle venv's
installed version, `RNS/_version.py`. File sha256 `47bc0fe460e60ca0e9660816441323bcb507e681bc7946866c192b38bd74fb25`, 906 lines.
`oracle/requirements.txt` pins 1.5.7, but the Windows venv was never
re-synced. The 1.5.7 receipt was taken on macOS. Diffing this file against
1.5.7 is a precondition of I2P3.

- **Framing.** HDLC over a SAM STREAM. `kiss_framing` is hard-coded `False`
  and no config key sets it (355, 553-554). Inbound frames are capped at
  `HW_MTU`; empty frames are ignored (539-540, 661).
- **Constants.** `HW_MTU` 1064 (340, 737). `BITRATE_GUESS` 256 kbit/s (719).
  `DEFAULT_IFAC_SIZE` 16, the same as stream carriers (720; review A2). Mode
  full (359, 744). `supports_discovery` true (762).
- **Config keys.** `peers` (list of b32 destinations), `connectable` (bool,
  default false), `ifac_size`, `ifac_netname`, `ifac_netkey` (728-735). Each
  peer becomes its own interface named `"<name> to <peer>"` (812-820). Each
  accepted stream spawns a peer interface that copies the parent's
  ingress/egress control (822-870).
- **Keepalive and liveness.** These are application-level and over the
  stream, not TCP options. Write `7E 7E` after 10 s without a write (582-584).
  The tunnel goes stale after 20 s without a read (577-580). The socket closes
  after `(9*5+10)*2` = 110 s without a read (331, 590-599). The initiator
  redials every 15 s with unlimited tries (323-324, 507-533). Client tunnel
  setup retries every 8 s and server tunnel setup every 15 s (418, 805). The
  Linux `TCP_USER_TIMEOUT`/keepalive options (443-448) apply to RNS's local
  socket to i2plib, not to anything across I2P.
- **Tunnel synthesis.** An HDLC initiator calls
  `Transport.synthesize_tunnel` on every connect and reconnect (503, 533),
  just as a TCP client does.
- **Destination key.** Generated once with `DEST GENERATE SIGNATURE_TYPE=7`
  and stored base64 at
  `storagepath/i2p/<hex(H(H(name) + H(transport_identity_hash)))>.i2p`. The
  older file `H(H(name))` is preferred when present (229-252). `H` is
  `Identity.full_hash`.
- **SAM traffic,** via vendored i2plib 0.0.14 (MIT, `RNS/vendor/i2plib`).
  `SESSION CREATE STYLE=STREAM` with **no i2cp options** (`aiosam.py` 71-98;
  `tunnel.py` 172-185), so tunnel length and leaseset encryption are the
  router's defaults. `STREAM CONNECT` / `STREAM ACCEPT` use `SILENT=false`
  (`sam.py` 59-64). The SAM address comes from `I2P_SAM_ADDRESS`, default
  `127.0.0.1:7656` (`utils.py` 29-35).

**emissary** (fork `crates/emissary` = upstream `9b43484` + one `Cargo.lock`
relock; upstream has no commits since 2026-07-26; v0.4.0 tagged 2026-04-12;
80 of the last year's 107 commits by one author).

- **Embedding.** Applications reach an embedded router only over SAMv3 or
  I2CP. `SamConfig { tcp_port: 0, udp_port: 0, host: "127.0.0.1" }` binds
  OS-assigned ports, which `Router::protocol_address_info()` reports
  (`docs/embedding-rust.md`). Reseeding is the embedder's job; `emissary-util`
  ships an HTTPS `Reseeder`, `Storage`, and a UPnP/NAT-PMP `PortMapper`.
  Transit and publish are plain `Config` fields.
- **Gaps against Java I2P / i2pd, from a grep of the code (not an audit).**
  I2CP refuses encrypted (blinded, b33) and meta leasesets
  (`i2cp/message/mod.rs` 552-566). There is no router family. ElGamal routers
  are ignored and 0-hop tunnels are unsupported; neither matters here. The
  `todo!` at `i2np/garlic.rs:824` is on the serialize path only, so a remote
  peer cannot reach it.
- **Dependencies.** The published manifests take `ed25519-dalek` /
  `x25519-dalek` at `3.0.0-pre.6` caret requirements, which accept the 3.x
  finals. Cargo ignores a dependency's lockfile, so the fork's relock does
  nothing for a consumer. `emissary-util` brings `rsa` 0.9 (advisory
  RUSTSEC-2023-0071, a private-key timing issue; the reseeder only verifies
  signatures), `sha2` 0.10 alongside murm's 0.11, `reqwest` 0.12, and
  native-tls by default (rustls is selectable). `cargo deny` here will need a
  decision on the advisory.

**yosemite 0.7.0** (eepnet, MIT; the SAM client emissary-cli uses).

- **Fits the stream need.** Tokio `AsyncRead`/`AsyncWrite` streams with
  `split()`. Sessions can be transient or `DestinationKind::Persistent { private_key }`.
  `RouterApi::generate_destination` and `lookup_name` exist. Signature type 7
  is the default. Tunnel length/quantity, `publish`, and `lease_set_enc_type`
  are all settable; the default is `6,4`, ML-KEM hybrid plus X25519. It also
  has repliable/anonymous datagrams and primary sessions.
- **Gaps.** The SAM host is hard-coded to `127.0.0.1`, so only the port can be
  set (issue #16, opened 2026-09-21). There is no Datagram2 or Datagram3
  (issues #6 and #7, open since 2025-04-20). There is an open question on SAM
  version handling (#17). Old majors: `rand` 0.8, `thiserror` 1, `nom` 7.
- **Upkeep.** One maintainer, the same as emissary. The last commit was
  2026-03-31; releases came every one to three months from 2025-07 to
  2026-03, then stopped. Two issues from September 2026 show no visible fix.

**This machine.** No I2P router is installed. Nothing listens on 7656.

## Phases

Each phase is gated on its own done-conditions. Every live phase runs a
positive control in the same run.

### I2P0 — Assemble

- Stage a portable i2pd: the official release archive, no installer, transit
  off, SAM bound to loopback. Record its version, archive name and sha256.
- Re-sync the oracle venv to the 1.5.7 pin, then diff `I2PInterface.py`
  between 1.5.6 and 1.5.7.

**Done when:** `i2pd --version` runs from the staged path. The oracle venv
reports 1.5.7. The I2PInterface diff is recorded here (empty or not).

### I2P1 — retinue I2P carrier, software-qualified

- Feature `i2p` on retinue, with the SAM client from F1.
- `Endpoint::attach_i2p` (one peer: dial, supervise, redial every 15 s) and a
  `connectable` listener (persisted destination, accept loop, spawned
  interfaces that inherit the parent's controls), both over
  `pump::carry(Framing::Hdlc, …)`.
- `HW_MTU` 1064, bitrate guess 256 kbit/s, IFAC default 16
  (`Ifac::for_stream`).
- The keepalive and liveness timings above, and tunnel synthesis on connect.
- `[[I2PInterface]]` parses (`peers`, `connectable`, `ifac_*`) and stops
  returning `UnsupportedType` when the feature is on. The SAM address follows
  the provider; `I2P_SAM_ADDRESS` is honoured as RNS does.
- Keyfile layout is fork F4.
- Adaptations are logged in retinue's `NOTICE` with file, lines and revision.

**Done when:** tests against a scripted fake SAM bridge pass:

- HELLO, DEST GENERATE, SESSION CREATE, and STREAM CONNECT/ACCEPT with the
  remote-destination line.
- Refusals: `CANT_REACH_PEER`, `DUPLICATED_DEST`, `INVALID_KEY`, timeout.
- Frames cross in both directions. Keepalive `7E 7E` is sent on idle, and an
  idle read closes and redials.

The positive control is the same harness passing an ordinary packet
round-trip before the refusal cases run. Strict Clippy passes, and the
default build is unchanged with the feature off.

### I2P2 — embedded emissary provider

- A default-off crate (F3) starts `emissary-core` on tokio with storage under
  a host-given directory. It reseeds on first run, binds SAM to
  `127.0.0.1:0`, and returns the SAM address.
- Its config has no `Default` for transit (on/off and cap) or publish, so the
  host must pass them.
- Dependency source is fork F2.

**Done when:** two retinue endpoints, each on its own destination over one
embedded emissary, exchange an announce, a link and a Resource in both
directions across real I2P tunnels with transit off. The receipt records
time to first tunnel, reseed source and router version. Positive control:
the same two endpoints over a loopback TCP interface in the same run.

### I2P3 — external router and cross-router

**Done when:**

- The I2P2 exchange passes with both endpoints on the staged i2pd (SAM 7656).
- It also passes with one endpoint on emissary and one on i2pd, across two
  routers.

### I2P4 — stock RNS interop

**Done when:** stock RNS 1.5.7 `I2PInterface` on i2pd and retinue on emissary
exchange an announce, a link and a Resource both ways, with IFAC off and
with IFAC 16. Each side's b32 is configured in the other's `peers`. One
retinue node also adopts an RNS keyfile, if F4 says it should.

Reconnect parity is part of this phase: killing the stream or restarting
the router recovers within the 15 s redial cadence and keeps routes.

## Open forks

F1, F2 and F4 were answered on 2026-10-21, and F3's names on 2026-10-22
(see Owner answers). Still open: confirming the locations, and when
`truchman` is claimed on crates.io. The original text of each fork follows.

- **F1 — SAM client.** yosemite meets I2P1's stream needs as published, with
  both providers on loopback. It falls short on a remote SAM host and on
  Datagram2/3, and upkeep is quiet. The options:
  - Take it as is now, with a stated condition for moving to our own crate.
  - Start our own SAMv3 + datagram crate now, with yosemite as reference.
- **F2 — emissary source.** Depend on the crates.io `emissary-core` /
  `emissary-util` 0.4.0, which resolve with the dalek finals, or pin our fork
  by rev. The fork currently differs only in a lockfile consumers never read.
- **F3 — provider crate name and location.** Radio workspace `crates/` or the
  `native/` workspace. The name is the owner's.
- **F4 — keyfile compatibility.** Mirror RNS's `storagepath/i2p/<hash>.i2p`
  layout and its old-format preference, so a node keeps its b32 across RNS
  and retinue. The alternative is a retinue-owned layout.

## Progress

- 2026-10-21: Assessment. RNS 1.5.6 `I2PInterface.py` and i2plib read; the
  emissary fork compared with upstream; yosemite examined. Owner answers
  recorded. Plan written. Second-round answers recorded (F1 own crate, F2
  crates.io, F4 mirror RNS, i2pd 3.0.0). F3 and the SAM crate's name remain open.
- 2026-10-21: I2P0, i2pd half. `i2pd_3.0.0_win64_mingw.zip` (4,867,598 bytes)
  came from the PurpleI2P GitHub release 3.0.0 (published 2026-10-20T19:06Z).
  It matched the release-published sha256 `973abdd5…3ff3fb3` and was
  extracted, after a path-safety check, to `Code/testing/retinue/i2pd/3.0.0/bin/`.
  `i2pd.exe` sha256 is `6f501e1f…7301c2a2`. `--version` prints
  `i2pd version 3.0.0 (0.9.71)`, Boost 1.92.0, OpenSSL 3.6.5. It has not been
  started as a router; no config or ports yet. The venv re-sync to 1.5.7 and
  the I2PInterface diff, the other half of I2P0, are not done.
- 2026-10-22: I2P1 implemented on branch `i2p-sam` (worktree
  `Code/worktrees/murm-i2p-sam`), not merged. Details follow.
  - **`truchman` 0.1.0** (`crates/truchman`, MPL-2.0) has a sans-io core and
    a tokio shell. The owner asked "Surprised not sans-io lol", and the crate
    was restructured before its first build. The core is `proto`: typed
    `Command`s produce validated lines, and `LineBuf` turns bytes into reply
    lines and keeps whatever follows as stream data. Destinations and keys,
    I2P base64/base32, and reply parsing are in the core too. The `tokio`
    feature (default on) adds `Bridge`'s commands, `StreamSession` (which
    answers PING with PONG and notices close) and the `Stream` that replays
    any pre-read bytes.
  - **Unlike yosemite:** any bridge host may be set, and every value written
    onto a SAM line is checked, so a config value cannot inject a command.
    Datagrams are not implemented; they need a phase of their own, scheduled
    when a consumer needs them.
  - **Tests:** 14 core unit tests and 7 against a scripted bridge. Coverage:
    HELLO and NOVERSION, DEST GENERATE, NAMING LOOKUP, transient and
    persistent SESSION CREATE, STREAM CONNECT and ACCEPT (with bytes arriving
    in the same segment as the status line), refusals with their `RESULT`
    and `MESSAGE`, PING/PONG and session close. The injection refusals have
    a positive control in the same test. Clippy is clean with and without
    `tokio`.
  - **retinue, feature `i2p`:**
    - `Endpoint::attach_i2p_peer` and `listen_i2p`, and
      `attach_i2p_section(section, bridge, storage)` for `[[I2PInterface]]`.
    - `i2p_keyfile` and `load_or_generate_i2p_key`, using the RNS layout (F4).
    - The pump gained an opt-in `Liveness` (keepalive bytes after a quiet
      spell, `ReadIdle` after a silent one). TCP, Pipe and Local pass none,
      so their behaviour is unchanged.
    - `NOTICE` records the batch at RNS 1.5.6.
  - **`tests/i2p_interface.rs`, 5 tests** over a fake loopback router that
    pairs CONNECT with ACCEPT:
    - A connectable hub learns a leaf's announce, and a link echoes. The
      leaf dials by b32, so lookup is exercised.
    - A silent far end gets one `7E 7E` keepalive after the synthesis frame,
      is dropped at the read deadline, and is redialed.
    - An unreachable peer stays offline and retries on one session; with no
      bridge at all, the endpoint is unharmed.
    - Keyfile naming, old-format preference and generate-once.
    - Section parsing, with 16-byte IFAC by default.
  - **Mutation check:** with liveness disabled in the dialer, the keepalive
    test fails at its assertion. With the code restored, all five pass.
  - **Pre-existing, not caused here:** retinue's lib test target does not
    compile on Windows. `endpoint/tests/interfaces.rs` 229, 233 call socket2's
    `tcp_keepalive_time`/`tcp_keepalive_interval`, which Windows lacks, and
    line 3 has an unused import. That file is identical to `main`, so the
    pump's own unit tests could not run on this machine. The integration
    suite stands in for them here. The family plan's MOVE notes already
    record this.
  - **Integration suite:** with `i2p,pipe,tulle-radio,serial`, every binary
    passes except one test,
    `reliable_stream::reliable_exchange_survives_loss_including_identify`,
    which failed three of three runs at its 20 s read timeout. It runs over
    the in-memory lossy interface, not the pump. Control: with `main`'s
    `pump.rs` swapped back in and the feature off, it fails identically. The
    family plan records it as flaky, failing at `HEAD`. On this machine,
    today, it fails every run.
  - **Not done in I2P1:** the `truchman` 0.1.0 publish, which needs the
    owner's go; any live router.
