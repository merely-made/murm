# I2P over SAMv3: retinue's I2P interface and the embedded router

**Date:** 2026-10-21. **Status:** retained I2P3/I2P4 reconciled (2026-10-26).
The retained I2P3/I2P4 implementation, harnesses and October 23-24 receipts
are restored from cohort `543a2a014c2`. Current `truchman` 0.1.1 pins and
its explicit Tokio macros dependency remain. The refreshed main had omitted
session-bound lookup, fresh-session redial, router termination reporting and
the interop harnesses despite its status note claiming I2P4 was present.
Current-tree qualification is recorded in the progress entry below; the
restored live receipts establish their original Windows cohort only.
The ephemeral identity parity fork remains open.

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
I2P2 round, 2026-10-22, after I2P0. The CI fact behind the location answer:
murm's CI runs `cargo deny check licenses` only (`.github/workflows/ci.yml`
125-127), on the root workspace. Advisories are not gated, and `native/` is
not license-checked. (An earlier statement in this session that the advisory
would fail `cargo deny` was wrong.)

- **`rsa` 0.9 advisory (RUSTSEC-2023-0071):** "Accept, recorded".
  emissary-util uses `rsa` only to verify su3 reseed signatures and parse
  reseed certificates (`su3.rs` 265, `certificates.rs` 19). Those are
  public-key operations the timing attack does not reach. Revisit when
  emissary moves past rsa 0.9.
- **Location:** "native/, add license gate". The crate is
  `native/crates/murm-i2p`, and CI gains a license check for `native/` with
  the root allow list.
- **Publish for the receipt run:** "Off, client-only". No ports opened, no
  UPnP, transit off.
- **Router storage:** "Persistent testing dir". `Code/testing/retinue/emissary/`
  is kept between runs (one reseed, a stable router ID) and deleted when the
  I2P phases close.
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
- **Ephemeral identity parity (open, unnumbered, raised 2026-10-23).** A stock
  non-transport RNS node gets a new I2P b32 at every start (see the upstream
  ledger). retinue's `attach_i2p_section` always names the keyfile from the
  endpoint's stable identity, so a retinue node keeps its b32. The options:
  - Keep retinue stable, as now.
  - Mirror RNS: an ephemeral transport hash unless the host asks for a
    static one.
- **F4 — keyfile compatibility.** Mirror RNS's `storagepath/i2p/<hash>.i2p`
  layout and its old-format preference, so a node keeps its b32 across RNS
  and retinue. The alternative is a retinue-owned layout.

## Upstream ledger

These are findings about upstream projects, recorded here first. Nothing is
filed upstream until the owner has read the entry and decided.

- **emissary: a remote peer can panic a debug build** (found 2026-10-22,
  I2P3). The cross-router run had a hub on i2pd 3.0.0 and a leaf on
  emissary-core 0.4.0. During it, the emissary leaf received a garlic
  message whose tag matched no tag-set entry and failed as NSR, and hit
  `debug_assert!(false)` at `destination/session/session.rs` 915. The router
  task panicked.
  - In release builds the assert compiles out and the message is rejected
    with `SessionError::InvalidState`.
  - The line is unchanged on upstream `master` (`9b43484`), and emissary-core
    has 108 `debug_assert!(false)`s in all.
  - The condition itself may be ordinary: a late or retransmitted message
    after a session reset, which emissary's own comment anticipates.
  - **Mitigation here:** `native/Cargo.toml` turns `debug-assertions` off
    for emissary-core in dev and test, and `murm-i2p` reports a router that
    ends (`stopped`, `RouterEnd::Panicked`).
  - **Upstream status:** not filed.
- **A kept SAM session cannot reach a destination that restarted, on emissary
  and on i2pd** (found 2026-10-23, I2P4 runs 2 and 5-9).
  - **What happened:** stock RNS on i2pd was killed and restarted on the
    same key and b32, and retinue kept its SAM session across the outage.
  - **Through emissary-core 0.4.0:** every `STREAM CONNECT` from that
    session got `CANT_REACH_PEER`, 34 attempts over about 15 minutes and 89
    over 40 minutes.
  - **Through i2pd 3.0.0:** `STREAM CONNECT` answered `RESULT=OK` and the
    interface came online (run 7). The restarted RNS logged no incoming
    connection, retinue's counters showed packets sent and none received,
    and two link attempts 30 s apart both timed out. i2pd reports OK without
    waiting for the far end.
  - **Shown by runs 8 and 9:** with a fresh session per redial
    (`fresh_session_per_redial`), both routers reached the restarted node on
    the first redial. RNS logged the accepted connection, and the
    post-restart link worked: back online 123.3 s after the kill on i2pd,
    135.6 s on emissary.
  - **So the stale state lives in the session, not in retinue:** each
    router keeps per-session remote state (the old lease set or garlic
    session) for the destination and does not rebuild it.
  - **Not tested:** whether stock RNS as the dialer, which also keeps one
    session per client tunnel, suffers the same.
  - **Upstream status:** not filed. Whether this is a defect or expected
    router behaviour is unsettled; on i2pd, the optimistic `OK` makes it
    silent.
- **RNS: one failed lookup kills a stock I2P peer for good** (found 2026-10-23,
  I2P4 run 1). RNS 1.5.7 was configured with `peers =` a fresh retinue b32.
  i2pd answered its first lookup `INVALID_KEY`, before emissary had
  published the lease set. RNS then logged "Exception on event loop ...
  invalid state" and, from then on, only "Event loop is closed", every 8 s,
  for the life of the process.
  - **Cause, by reading the code:** `I2PController.start` runs the asyncio
    loop and closes it in `finally` on any exception (`I2PInterface.py`
    94-112). Nothing restarts it, so every later tunnel attempt fails.
  - **Effect:** a stock RNS node whose I2P peer is briefly unresolvable at
    start (a new destination, or a router not yet integrated) never connects
    until restarted.
  - **Workaround here:** the I2P4 orchestrator starts RNS only once i2pd can
    resolve retinue's b32 on a fresh connection.
  - **Upstream status:** not filed.
- **RNS: a non-transport node's I2P b32 changes on every start** (found
  2026-10-23, I2P4 run 1).
  - **Mechanism:** with `enable_transport = No` (the default) and no
    `static_transport_identity`, RNS loads the stored transport identity and
    then replaces it with a fresh ephemeral one (`Transport.py` 332-335).
    `I2PInterface` names its keyfile from `RNS.Transport.identity.hash`
    (`I2PInterface.py` 234), so a connectable non-transport node writes a new
    key, and gets a new b32, at each start. Any `peers =` entry naming it goes
    stale on its first restart.
  - **This looks deliberate on RNS's side** (privacy for non-transport
    nodes), so it is recorded as behaviour, not as a defect.
  - **The parity question it raises** is in Open forks ("Ephemeral identity
    parity").
  - **Upstream status:** not filed.
- **RNS: I2P `peers` with IFAC and no explicit `ifac_size` fail on every
  frame** (found 2026-10-23, I2P4 run 2).
  - **What happened:** with `networkname`/`passphrase` and no `ifac_size`,
    a stock RNS 1.5.7 node dialing through `peers =` logged
    `unsupported operand type(s) for +: 'int' and 'NoneType'` at `len(raw) >
    2+interface.ifac_size` on every inbound frame, plus `bad operand type for
    unary -: 'NoneType'` on transmit, and no traffic passed.
  - **Cause, by reading the code:** `I2PInterfacePeer.__init__` copies
    `parent_interface.ifac_size` (`I2PInterface.py` 366). For `peers`, that
    runs inside `I2PInterface.__init__` (812-820), before `Reticulum.py`
    assigns the parent its default size, so each peer holds `None`. Accepted
    connections copy it later (847), so the connectable side works.
  - **Confirmed by the workaround:** with `ifac_size = 128` the same variant
    passed (run 4).
  - **Upstream status:** not filed.
- **i2pd: a SAM `HELLO` split across TCP segments drops the connection**
  (found 2026-10-23, I2P4 run 3).
  - **Probe:** against i2pd 3.0.0, `HELLO VERSION MIN=3.1 MAX=3.3` sent whole
    is answered `HELLO REPLY RESULT=OK VERSION=3.3`. Sent as `HE` then the
    rest 0.5 s later, the connection is aborted, and i2pd logs "SAM: Handshake
    mismatch".
  - TCP gives no segment boundaries, so this is a robustness defect. Real
    clients, truchman included, send each line in one write and are
    unaffected.
  - It broke this lane's byte-wise SAM logging proxy, which now forwards
    whole lines.
  - **Upstream status:** not filed.

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
- 2026-10-22: I2P1 merged into `main` as `cb0c91e` (not pushed). The merged
  tree is identical to the tested branch tree (`37fcc77`). `native/` and
  `apps/signalman-desktop/` still resolve with `--locked`. The lane's
  worktree, build output and branch were removed.
- 2026-10-22: **`truchman` 0.1.0 published** to crates.io from `cb0c91e`, on
  the owner's authorization. The dry run packaged 17 files (80.0 KiB) and
  verify-built. The release shows on the sparse index (cksum `2ed91932…`)
  and the API (created 2026-10-22T19:14:12Z). This completes I2P1; I2P0's
  venv re-sync and I2P2-I2P4 remain.
- 2026-10-22: **I2P0 done.**
  - **Venv:** before touching the oracle venv, I confirmed no running
    process used it. `rns-1.5.7-py3-none-any.whl` (644,289 bytes) came from
    PyPI into its own scratch directory, with sha256 `0e4aaf8c…41faddf`. That
    is the same wheel retinue's `NOTICE` records for the 1.5.7 review. The
    venv was installed from that file, then `requirements.txt`. It now
    reports RNS 1.5.7 and LXMF 1.2.0, and `pip check` is clean.
  - **Diff:** `Interfaces/I2PInterface.py` and the vendored `i2plib` are
    byte-identical between 1.5.6 and 1.5.7 (sha256 `47bc0fe4…` both), so
    I2P1's parity holds at the current pin. `NOTICE` is updated.
- 2026-10-22: **I2P2 passed** on branch `i2p-host` (worktree
  `Code/worktrees/murm-i2p-host`). Details follow.
  - **`murm-i2p`** (`native/crates/murm-i2p`, `publish = false`) embeds
    `emissary-core` / `emissary-util` 0.4.0 from crates.io.
    - `RouterConfig::new(storage, transit, publish)` takes transit and
      publish as required arguments, with no defaults.
    - Reseeding follows emissary-cli 0.4.0: below 25 stored routers, from
      the built-in HTTPS servers, IPv4 only (`config.rs` 329-330, `main.rs`
      137-153).
    - `EmbeddedRouter::start` returns the loopback SAM `Bridge` and a
      `StartReport`. `shutdown` is graceful, with ten seconds' grace.
  - **Receipt:** `testing/receipts/i2p2-embedded-emissary/`, two live runs,
    both passing.
    - Two endpoints on one router exchanged announces, links and a 12 KB
      Resource each way. A loopback TCP control, held at the protocol MTU,
      did the same in the same run.
    - Run 1 started from empty storage: it reseeded 137 routers, the router
      started in 4.6 s, and the I2P exchange took 32.7 s in all.
    - Run 2 was warm: no reseed, a 1.4 s start, 66.6 s in all.
    - First contact dominates both runs (26 s, then 58 s). After it, each
      link round-trip takes 1.2-1.7 s and each Resource 1.9-2.6 s.
    - An earlier attempt failed in the control (the TCP link MTU made the
      payload a single packet) before any network contact. It is kept in the
      receipt.
  - **Findings:**
    - emissary binds NTCP2 on `0.0.0.0` and SSU2 likewise whenever IPv4 is
      on, published or not (`transport/ntcp2/mod.rs` 225-227). "Client-only"
      therefore means unadvertised listeners. The owner chose to run anyway
      and cancel any firewall prompt.
    - 0.4.0's `Ntcp2Config` has `publish`, where the online docs (master)
      have `publish_ipv4`/`publish_ipv6`, and its reseeder is fixed to
      reqwest `default-tls` (native-tls). Both are tracked as released, not
      as master.
    - emissary has no local-delivery path between destinations on one
      router (`destination/mod.rs` 294-330). The receipt does not observe
      tunnels directly.
  - **License gate:** CI job `licenses-native` runs `cargo deny --all-features
    --config deny.toml check licenses` on `native/`. Per the owner's choice,
    `deny.toml` gains per-crate exceptions for the nine permissive crates the
    native graph has outside the allow list: async_io_stream, pharos and
    ws_stream_wasm (Unlicense), foldhash (Zlib), spez (BSD-2-Clause),
    webpki-roots and webpki-root-certs (CDLA-Permissive-2.0), and tiny-keccak
    (CC0-1.0, via emissary). Both gates pass locally with `--all-features`.
  - **Locks:** `native/Cargo.lock` adds 68 packages and changes no existing
    version.
  - **Not done:** I2P3 (i2pd, and across routers) and I2P4 (stock RNS).
- 2026-10-23: **I2P3 passed** on branch `i2p-ext` (worktree
  `Code/worktrees/murm-i2p-ext`). The receipt is
  `testing/receipts/i2p3-external-router/`. Details follow.
  - **i2pd:** 3.0.0 ran client-only from the staged copy, configured to
    follow the owner's I2P2 answers (`notransit`, transports unpublished,
    no UPnP, no console, proxies or address book; SAM on `127.0.0.1:7656`).
    It reseeded once, and was stopped gracefully after the runs.
  - **Results:** both endpoints on i2pd passed (140.4 s in all, first
    contact 110.5 s). Across routers, both directions passed in one run: hub
    on emissary with its leaf on i2pd took 56.9 s, hub on i2pd with its leaf
    on emissary 54.1 s. The refactored I2P2 test passed again (38.7 s).
    Every run had its TCP control.
  - **Failure, kept:** the first cross run panicked emissary at
    `destination/session/session.rs` 915 (`debug_assert!(false)` on an
    unknown garlic tag from the i2pd peer). See the upstream ledger.
  - **The owner's two answers, applied:**
    - `[profile.dev.package.emissary-core] debug-assertions = false` in
      `native/Cargo.toml`.
    - `EmbeddedRouter::is_running`, `stopped() -> RouterEnd` (Exited,
      Panicked with message, Aborted), and `shutdown` now returns it.
      `watch_end` is unit-tested for all three outcomes.
    - The live tests race each emissary leg against `stopped()`.
    - The live tests were refactored into `control`, `emissary`,
      `external` and `over_i2p` helpers.
  - **Not established:**
    - whether the unknown-tag condition recurred in the passing cross run;
    - that the profile override took effect, beyond the panic's absence;
    - more than one run per configuration.
  - **Not done:** I2P4 (stock RNS 1.5.7 `I2PInterface` on i2pd against
    retinue on emissary).
- 2026-10-23: **I2P4** on branch `i2p-rns` (worktree
  `Code/worktrees/murm-i2p-rns`). The receipt is
  `testing/receipts/i2p4-stock-rns/` (six runs). Details follow.
  - **Owner's answers:** session-bound lookup; an oracle script plus a
    murm-i2p example; the keyfile proven by path and destination match;
    reconnect judged on 15 s redials and kept routes, with recovery recorded
    but not bounded.
  - **truchman 0.1.1 (unpublished):** `StreamSession::lookup` and `resolve`
    run on the session's own control connection. The control task
    interleaves lookups with PING/PONG and matches replies by `NAME`. A test
    against a bridge that accepts only one connection proves the lookups
    went over the session.
  - **retinue:** I2P peers resolve through their session. emissary answers
    b32 lookups only there (`sam/pending/connection.rs` 391-402,
    `sam/session.rs` 1160). The fake router now behaves the same way, and
    putting back the old fresh-connection lookup fails the hub test.
  - **murm-i2p:** example `rns_i2p_peer`: retinue on emissary, or on an
    external router with `--sam`. It has a SAM logging proxy that forwards
    whole lines, and a `keyfile` subcommand. Oracle: `interop_i2p.py` with
    `--only`, `--retinue-sam` and `--recovery-window`.
  - **Results:** see the receipt. Every exchange passed in both directions,
    IFAC off and 16 (one variant needed an RNS-defect workaround); the
    keyfile layout passed; redials came 15.0 s apart with routes kept; the
    interface came back online on i2pd in 107.7 s.
  - **Open:**
    - Through emissary, a restarted RNS is never reached (upstream ledger).
    - On i2pd, the first link after the restart timed out, cause
      undiagnosed. It could be retinue's.
  - **Upstream ledger:** five entries in all. This phase added three for
    RNS (first-lookup death, ephemeral identity, IFAC peers), one for i2pd
    (split `HELLO`) and one for emissary (restart), alongside I2P3's panic.
    The new open fork: ephemeral identity parity.
- 2026-10-23: **Post-restart link timeout diagnosed** (owner: "Diagnose the
  post-restart link timeout on i2pd first (it may be retinue's); keep the
  branch unmerged until then"). Runs 7-9 are in the I2P4 receipt.
  - **Instruments:**
    - the restarted RNS logs at level 7;
    - the example gained `STATS` (interface counters);
    - the harness echoes again 30 s after a failure.
  - **Run 7 (i2pd, session kept):**
    - reconnect passed: redial 15.0 s after the stream ended, online 119.1 s
      after the kill, route kept;
    - both link attempts timed out;
    - the restarted RNS logged no accepted connection;
    - retinue's counters went `tx` 26 → 28 → 30 with `rx` steady at 18.
    `establish` never reuses a link, so stale-link reuse is ruled out.
  - **Cause:** the session retinue kept across the outage could not reach the
    restarted destination. On i2pd the stream looked open (an optimistic
    `STREAM STATUS OK`); on emissary every connect was refused.
  - **Test (runs 8, 9):** experimental `I2pPeer::fresh_session_per_redial`,
    default off, sets session and target to `None` after a stream ends or a
    connect is refused. With it, both routers recovered on the first redial
    and the post-restart link passed:
    - i2pd: redial began 14.8 s after the stream ended, session ready
      +26.8 s, online +30.2 s;
    - emissary: 14.9 s, +23.2 s, +26.1 s.
  - **Harness:** a redial is now timed from its first SAM command.
  - **Open:** whether `fresh_session_per_redial` should become the default.
    That question is for the owner.
- 2026-10-23: **`fresh_session_per_redial` is now the default** (owner: "Make
  fresh_session_per_redial the default (keep the field so a host can keep one
  session), rerun the full I2P4 orchestrator to confirm, then I'll decide on
  merging").
  - **Code:** the field's doc now carries the reason, and the
    `attach_i2p_peer` doc describes the behaviour. The fake-router test
    covers both arms: by default one session per attempt; kept, exactly
    one. The example's opt-out is `--keep-session`. `NOTICE` records the
    difference from RNS's one-session client tunnel.
  - **Run 10, the full orchestrator on emissary:** everything passed,
    including reconnect (redial 15.0 s after the stream ended, back online
    135.1 s after the kill, route kept) and the post-restart link. The one
    exception was RNS's Resource to retinue in RNS-dials no-IFAC: RNS's
    sender reported `FAILED` and cancelled. The new default does not touch
    that variant, since retinue is the listener there.
  - **Runs 11-13** repeated that variant three times with RNS at log level
    7, and all passed. It stands at 1 failure in 5, intermittent, cause
    unknown.
  - **Harness fixes this round:** `--rns-loglevel`, and UTF-8 output in both
    processes. A cp1252 encode error in verbose RNS output had killed the
    line-relay thread; the three runs that hit it were discarded and rerun.
- 2026-10-23: **Run 10's Resource failure diagnosed** (owner: "Hold the merge;
  run the RNS-dials no-IFAC variant more times with RNS at log level 7 until
  the Resource failure recurs, and diagnose it").
  - **Runs 14-33:** 20 more runs of the variant at RNS level 7, all passed.
    The variant now stands at 1 failure in 25 (runs 2 and 10-33). No run
    logged a single RNS Resource watchdog retry.
  - **The cause, from i2pd's log of run 10:** RNS's Resource to retinue
    began at about 18:55:21 local. From 18:55:27 the variant's only I2P
    stream (RNS's peer stream to retinue) stalled both ways:
    - i2pd resent its outgoing messages three times on other leases and
      tunnels;
    - from 18:55:44 to 18:55:58 incoming message 15 was missing while 16-21
      queued behind it, so RNS received nothing from retinue;
    - tunnel tests failed at about 2 a minute through the variant's window,
      against about 0.9 a minute across runs 14-33. Runs 14-33 logged no
      stream resends and one single-message gap that recovered.
  - **Why RNS gave up:** an RNS sender's shortest patience is the advertised
    stage, 5 waits of 6 × RTT + 1 s. That is 20-107 s at the link RTTs these
    runs measured (0.5-3.4 s). The stall was at least 31 s. Retinue had the
    advertisement (its error names the Resource), so its part request was
    probably what sat behind message 15. Which watchdog path fired is
    inferred, not logged: run 10 logged RNS at level 3.
  - **Verdict:** I2P network loss on the path between the two routers,
    longer than RNS's Resource sender tolerance. Nothing points at retinue,
    and nothing is ledgered.

- 2026-10-26: **Reconcile retained I2P3/I2P4 with current Murm main.**
  Restore the source, tests, harnesses, NOTICE entry, router profile override
  and 72 historical receipt files from `543a2a014c2`, while retaining current
  supplier revisions and `truchman` 0.1.1. The prior refresh's status claim
  was incorrect: the session lookup/redial and router-end changes were absent.
  Recovery refs and Windows dirty work are preserved separately.
  - Current-tree checks pass: 14 focused SAM/I2P tests on macOS, and the
    native router termination test plus example compilation on macOS and
    Windows. The three standalone public-network native tests remain ignored.
  - The Windows stock-RNS live check passes for **Retinue dials RNS, no
    IFAC**, with a TCP control: bidirectional links and 12,000-byte Resources,
    matching keyfile/b32, same destination on restart, route retention and
    a fresh working link. Recovery took 60.7 s; redial gaps were 14.8 and
    15.0 s. The run completed in 265.4 s within a 30-minute cap.
  - Other dial/IFAC variants were not rerun. The current receipt records
    the initial Python launcher failure and shutdown observations separately
    from the successful exchange. See [qualification](../testing/receipts/i2p-reconciliation-20261026/qualification.json)
    and [live measurements](../testing/receipts/i2p-reconciliation-20261026/live-interop.json).
