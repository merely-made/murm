# Murm family integration and RTC

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-13_murm_family_move_plan.md`; the original is archived whole.

## Station Custody extraction, 2026-11-04

**Status: landed.** Murm `a34a783fa2808071188883122adc42beb18dfd55` receives
the standalone `crates/station-custody` workspace and Signalman's desktop path
dependency. Mere `3a5e2f1286d7a397f6051cb11b3e0d631df696e8` removes the donor.
The receiver was pushed and all twelve Rust blobs verified before donor removal.
MPL notices, API, derivation salts, control encodings, first-owner behavior and
lease expiry/revocation are unchanged. The three existing source-size exceptions
move with their files. Dramatis/Personae and Djinn retain credential and grant
authority; Murm owns the radio-specific adapter, outside its default radio build.

The standalone all-target suite passes 22 tests; Signalman's station and Djinn
filters pass 10 tests. Two broker-dependent fixtures remain ignored. The adapter's
Rust 1.97.1 library check, both Clippy checks, Signalman's locked all-target check,
Mere's locked whole-workspace/all-target check and both repositories' hooks pass.
Mere has 87 members; its default native normal/build graph excludes the station
and radio family. Retinue remains in its lockfile for optional resident NomadNet.
Signalman's existing Mere/Genet GUI source identities are preserved.

Required evidence is retained under
`Code/testing/murm/station-custody-extraction-20261104`, at most **100 MB**:
source equality, commands/results, lock/source identities and pushed commits.
Both Cargo output directories use stable Murm/Mere targets; no isolated Cargo
home was created. Existing physical/headed receipts remain frozen. No board
claim, provisioning, serial/RF or installed-daemon qualification was run. Mere's
app-host build warns that its guest build failed; no guest-load acceptance is
claimed. The legacy multi-application DR-C harness still needs its earlier
Castellan/resident path migration; only its Station Custody paths changed here.

## Open phases and done conditions

- **R1–R3 application integration:** qualify the actual Signalman LXMF path on its named physical/headed profile and extend operator diagnostics to peer, path, sync and standards-profile facts without inventing delivery. Sennet and Tucket have distinct native semantics; broader LXMF bodies, voice and multi-station selection remain separate capabilities. TinySSB verification does not itself provide chat.
- **LXMF interoperability and radio diagnostics:** qualify desktop Signalman with ordinary stock LXMF/MeshChatX content, since its versioned titles are narrower than generic chat. Surface held messages while sender identity is unresolved without asserting authenticated identity, and investigate remaining V4 frame loss, the unexplained RNode error 04 and post-timeout path discovery on an identified bench. The terminal single-send failure was repaired; delivery in each direction did not establish reliable RF.
- **R4–R5 companions:** complete selected serial/RF, BLE and headed/pixel acceptance, including the immediate MeshCore reconnect variant separately from the passing 500 ms fixture, plus journal power-loss behavior. Name stock versions and device identities and restore profiles. Stock-pair functional acceptance is not full application or platform parity.
- **R6–R8 native radio:** retain scoped Tucket contact-restore and Sennet public/directed text/ACK passes; qualify installed personal-daemon custody/UI, private-channel custody, 2.8 ACK proofs, broader mesh routing and remaining visual cleanup under their own profiles. Protocol ACK correlation is not authenticated read receipt. T114 endpoint and broader roles belong to [protocol parity](2026-11-02_protocol_parity_plan.md).
- **Board names:** preserve labels and current identity/preferences when restoring or changing images. Physical screen pixels were not independently observed, and the SB2 five software-reboot recovery does not establish battery-disconnected power-cycle recovery; either is a separate qualification if needed.
- **Supplier and receiving gates:** registry releases require their own decision and qualification. Resolve the filtered killed-peer shutdown test with bounded evidence and a positive control; Keep physical/live-network/headed gates and the Isocosm gameplay compiler/wing gates with their owners; source adoption is not a gameplay pass. Recheck current owning plans rather than assume the historical failure persists.
- **RTC provider continuation:** the fixed-server restart, explicit-port DNS, UDP service discovery and bounded remote IPv4 mDNS receiving slices below are qualified; native Firefox responder acceptance, local mDNS advertisement, IPv6 multicast, TCP/TLS TURN and server replacement retain separate gates. Keep relay-only from direct candidates; host budgets and I/O remain caller-owned. Complete application-carrier adoption only when the real consumer passes binding, admission, backpressure, refusal and reconnect with the shared provider. Genet owns its browser/WPT/media gates and its selected headed direct/UDP-relay matrix does not prove cross-machine or application-carrier acceptance. Registry publication waits for an engine release containing the pinned DCEP and restart fixes.

## Decisions and boundaries

### Remote mDNS candidate continuation

**Status (2026-11-04): E2m supplier and bounded Genet receiving qualified.**
Genet's current mDNS plan owns remote-candidate receiving. Provider
`bd8b6659a29c5e52ebe09b028ec3ef5a455d7e7f` pins core
`896ae75de1c2673e98a7ebbf7058b2733786276e`. Its bounded, caller-polled
one-shot resolver uses a selected IPv4 interface and actual multicast packet
metadata; ordinary DNS keeps refusing `.local`. All 34 ordinary and three
controlled coturn checks pass per working/MSRV toolchain, with strict Clippy
and a host-only normal/build graph excluding Tokio. The core preserves names
and IDs separately from numeric routes, excludes local TURN pairs with remote
mDNS names, and waits for mutual restart credentials before replacing a retained
full-peer route. Genet owns web-visible privacy, SDP/trickle ingestion and
lifetime/generation cancellation. The receiving matrix keeps Chrome names
untouched and explicitly uses controlled aliases for observed Firefox IPv4
sockets. Firefox's own responder, local advertisement, IPv6 multicast and
application carriers remain open. Receiving source
`993fb879f3139bcd70048e277989f698c41caffc` passes 44 native checks and all
48 controlled headed cases, with unchanged selected WPT maps. Its closed
external E2m packet records 96 restarts, 48 interrupted cleanups, owned fixture
shutdown and permission restoration; failed earlier cohorts remain distinct.
New E2m supplier evidence is capped at 6 KB in the existing family receipt,
within its 100 KB total; Genet's separate fresh packet is capped at 244 KB,
including core gates. Earlier E2s and other RTC receipts remain closed.

### UDP service discovery continuation

**Status (2026-11-04): provider and controlled Genet receiving qualified.**

E2s in `genet/design_docs/2026-11-04_webrtc_mdns_plan.md` owns the new
portless-URL receiving gate. Murm extends its independent resolver with bounded
SRV/S-NAPTR endpoint batches and lazy backtracking, retaining caller-owned I/O,
URI omission and existing TURN allocations. Keep the previous E2d evidence
closed. New supplier evidence is capped at **30 KB** in the existing family
receipt, under its 100 KB total; Genet has a fresh external 220 KB packet.
UDP-only service resolution does not qualify mDNS, TURN TCP/TLS or carriers.
Provider `ef2d18fadbbdca3847ad99040af14f2bc7693a47` passes 29 ordinary and
three controlled coturn checks on Rust 1.99.1 and MSRV 1.97.1, strict wire
all-target Clippy, and the host-only DNS compile/tree without Tokio. The
source/commands and limits are in the appended E2s family receipt. Genet product
source `ff958976f2a904d250d00ee10335696c62f749b1` passes 38 native checks;
recorder `7abebdda633bac2d91bd05b4976f2bf65ea525cb` passes all 48 headed
Chrome/Firefox restart cases. Native URLs are portless and discover nondefault
UDP ports; browsers use equivalent numeric endpoints on fixed-interface IPv4.
The seven selected WPT maps remain unchanged. Genet's external Nov 4 packet is
closed after fixture shutdown and permission restoration. This is not full
TURN transport/authentication, public NAT, media or carrier acceptance.

### Hostname ICE-server resolution

**Status (2026-11-04): provider and controlled Genet receiving gates qualified.**

Genet's `genet/design_docs/2026-11-04_webrtc_mdns_plan.md`, E2d, owns the
authorized DNS receiving slice. Murm supplies an optional caller-polled resolver
and bounded per-URL address fallback while keeping the core and host-only
numeric path runtime-free. Pending requests cancel on restart/close/drop;
successful TURN allocations retain their endpoint and socket. Retain at most
**30 KB** of new supplier evidence in the existing family receipt (under its
100 KB total); Genet owns the separate external receiving packet. A/AAAA with
explicit UDP ports is the qualified address-lookup gate; SRV/NAPTR, mDNS and TURN TCP/TLS remain
separate. Prior E2r receipts stay frozen at their original boundaries.

### RTC ICE restart slice

**Status (2026-11-03): provider and Genet receiving qualified for fixed numeric UDP servers and unchanged interfaces.**

Mark authorized established data-channel ICE restart with configured STUN/TURN
and automated native/browser tests. Retain the same physical socket and TURN
allocations while credentials and connectivity checks move to a new generation;
STUN restarts with fresh bounded transactions. Retired ICE checks must not enter
the fresh generation, and relay-only keeps its no-direct-route guarantee.
Close/drop releases the retained allocations once. A bounded previously nominated
data path stays usable during negotiation and is retired on success/failure or
its deadline. Authenticated old consent is answered only on that route, without
changing fresh checks/roles or extending its expiry. The core repairs live in
the pinned `merely-made/rtc` fork, including the same-identity DTLS answer role.
After fresh remote credentials arrive, the known nominated address is checked
as a fresh candidate without inheriting prior nomination or transactions.
Authenticated current-generation requests also schedule a fresh check when
the controlling pair exhausted its checks before the answer arrived. Broader
triggered-check scheduling and requests before remote credentials remain separate gates.

Done: ordinary provider checks on working Rust and MSRV, controlled coturn,
strict Clippy and Genet's unchanged WPT/native/headed receiving gates pass.
Genet owns event ordering, sticky restart requests, candidate generation checks
and browser lifetime. Server replacement, interface rebinding, SRV/NAPTR/mDNS, TCP/TLS,
media and carrier adoption retain their own gates. Append at most **20 KB** to
the existing family receipt; its total stays below **100 KB**. Genet's receiving
predecessor packet is frozen under its separate 1 MB cap; new E2d evidence uses
its external 220 KB receiving cap and source/lock/binary audit.

Murm supplies communication engines; Dramatis/Personae and Djinn retain identity and custody, Gemot membership authority, and applications own their facts. The later foundation promotion places Stickleback in Eidetic; Genet owns Fetch/WebSocket policy. The generic RTC provider has no application admission, invitation, Luggage, radio or conversation dependency; the existing application carrier retains its own framing and trust. Keep original crate identities, ancestry and immutable historical evidence. Sennet retains its independent-source boundary: public documentation and identified black-box captures, excluding upstream firmware implementation and schemas. A host carrier handoff or unauthenticated protocol ACK does not establish recipient read or sender authentication. The stock companion queue read can consume device data before host persistence; do not claim exactly-once or power-loss safety.

## Findings, 2026-11-02

`native/Cargo.toml` pins Stickleback and related foundation packages to Eidetic; `native/crates/transport/src/p2panda_transport/tests.rs` retains the killed-peer test. `apps/signalman/src/message/observe.rs` still rejects other wire titles, `crates/postilion/src/held.rs` holds unverified senders until announce, and `crates/outrider/src/direct.rs` retains bounded fresh-link retries. `apps/signalman/src/radio_comms.rs` contains the radio application owner; `wire/crates/rtc/src/udp.rs` contains the caller-driven UDP provider. MOVE0–MOVE5 and named source adoption are recorded as published at their bounded scopes. R5 stock functional, R6–R8 native and E1a/E1c local results have distinct hardware and peer scopes. Automatic contact restoration was subsequently qualified for selected host, V4 and T114 profiles; do not carry the earlier blanket assertion as current.

## Progress

- 2026-11-04: E2d provider `c018fee13f417867cd3bbfe3ebca167257e34dc0`
  passes 20 ordinary checks on working Rust 1.99.1 and MSRV 1.97.1, three
  coturn checks, strict wire Clippy and host-only compile. Native DNS uses an
  established wire codec with caller-polled UDP/TCP and owned cancellation;
  the production graph has no Tokio. Immediate refusal now drains terminal
  errors before gathering completes. Genet source
  `d9f402446c7651ddad37014799fdb13c65c9f243` passes 30 native checks,
  unchanged selected WPT maps and all 48 headed hostname/direct restart cases
  (96 completed and 48 interrupted restarts). Its external packet owns the
  exact binary/lock identities, 16 pending-DNS cleanups, permission restoration,
  fixture shutdown and prior retained-cache exception. This does not qualify
  portless service discovery, mDNS, TURN TCP/TLS, media or application carriers.
- 2026-11-03: provider requalified against core `479995928668d0db1f8cfb2aa26f818b9f03919f`:
  58 core ICE tests, nine ordinary checks on working Rust and MSRV, three
  controlled coturn checks and strict all-target wire Clippy pass. The delayed-answer
  regression checks recovery only after fresh authentication. Genet source
  `69c926551546fe5d7cdd4efaeb6b89e18e045adc` passes 22 native checks, the unchanged
  selected WPT comparison and all 48 headed restart cases on provider
  `2ff146629e17ce8e77fc56ffa26f7b13b8b8f83a`. Its receipt owns the fixture,
  permission-restoration and retained-cache audit.
- 2026-11-03: fixed-server restart provider qualified at `377b9ca1b59f48b0c491e25b7d70bb57a23faedd`,
  using core `235f02199a23f00df2c554ed669218eee9627a8e`. Provider working/MSRV,
  controlled coturn, strict Clippy and 57 core ICE tests pass. Genet's native
  and selected WPT receiving gates passed on that pin; the late-check repair
  and final headed receiving gate are recorded above.
- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.

- 2026-11-04: local IPv4 candidate registration is an opt-in, caller-owned
  provider API using the core UUID responder at ba8e3c4. It owns selected-link
  multicast receive and explicit-source UDP/5353 output, bounded polling and
  best-effort goodbye on drop. Consumer peer quotas bound registration count.
  Direct-unicast queries and IPv6 remain open. Genet owns exposure/projection
  and the registration lifetime; its E2a plan and shared 20 KB evidence budget
  own native acceptance, including failure privacy and cleanup.

## Pipit consolidation, 2026-11-04

Pipit 0.2.0 moves from standalone revision `1281de0901bf47bf3e9edc8f78b122ce60a44389` to `crates/pipit`. Outrider's optional voice feature and Signalman use the local codec. The independent workspace preserves no_std, optional allocation, zero dependencies, codec/frame/clip bytes and MIT/Apache notices. Original source blobs are enumerated in `crates/pipit/source-import.json`; the receiving commit retains donor ancestry. Registry publication remains a separate release gate.

Done conditions: original and receiving 48 codec tests agree; Rust 1.97.1 allocation-free compilation passes; Outrider voice and Signalman voice tests and all-target checks pass with one codec source; line ceiling and Clippy pass. These are native in-process gates, not radio, audio-device or headed qualification. Bounded summaries share `Code/testing/subsystem-extractions-20261104` and its aggregate 100 MB cap.

Receiving result: 48 codec, 14 Outrider voice and 2 Signalman voice tests pass; both consumer all-target checks, allocation-free MSRV, codec Clippy and root hooks pass. The only root lock package change is Pipit registry-to-local source; published codec version and clip identities stay 0.2.0.

Signalman Desktop receiving follow-up: its independent lock now selects the same local Pipit 0.2.0. The locked offline all-target desktop check passes; the only lock change removes Pipit's registry source/checksum. Existing Mere/Genet source cohorts are preserved. This is compilation, not headed/audio-device qualification.
