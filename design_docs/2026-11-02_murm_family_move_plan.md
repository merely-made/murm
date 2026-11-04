# Murm family integration and RTC

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-13_murm_family_move_plan.md`; the original is archived whole.

## Open phases and done conditions

- **R1–R3 application integration:** qualify the actual Signalman LXMF path on its named physical/headed profile and extend operator diagnostics to peer, path, sync and standards-profile facts without inventing delivery. Sennet and Tucket have distinct native semantics; broader LXMF bodies, voice and multi-station selection remain separate capabilities. TinySSB verification does not itself provide chat.
- **LXMF interoperability and radio diagnostics:** qualify desktop Signalman with ordinary stock LXMF/MeshChatX content, since its versioned titles are narrower than generic chat. Surface held messages while sender identity is unresolved without asserting authenticated identity, and investigate remaining V4 frame loss, the unexplained RNode error 04 and post-timeout path discovery on an identified bench. The terminal single-send failure was repaired; delivery in each direction did not establish reliable RF.
- **R4–R5 companions:** complete selected serial/RF, BLE and headed/pixel acceptance, including the immediate MeshCore reconnect variant separately from the passing 500 ms fixture, plus journal power-loss behavior. Name stock versions and device identities and restore profiles. Stock-pair functional acceptance is not full application or platform parity.
- **R6–R8 native radio:** retain scoped Tucket contact-restore and Sennet public/directed text/ACK passes; qualify installed personal-daemon custody/UI, private-channel custody, 2.8 ACK proofs, broader mesh routing and remaining visual cleanup under their own profiles. Protocol ACK correlation is not authenticated read receipt. T114 endpoint and broader roles belong to [protocol parity](2026-11-02_protocol_parity_plan.md).
- **Board names:** preserve labels and current identity/preferences when restoring or changing images. Physical screen pixels were not independently observed, and the SB2 five software-reboot recovery does not establish battery-disconnected power-cycle recovery; either is a separate qualification if needed.
- **Supplier and receiving gates:** registry releases require their own decision and qualification. Resolve the filtered killed-peer shutdown test with bounded evidence and a positive control; Keep physical/live-network/headed gates and the Isocosm gameplay compiler/wing gates with their owners; source adoption is not a gameplay pass. Recheck current owning plans rather than assume the historical failure persists.
- **RTC provider continuation:** qualify the fixed-server ICE restart slice below; add and qualify host DNS, mDNS, TCP/TLS TURN and server replacement as separately selected capabilities. Keep relay-only from direct candidates; host budgets and I/O remain caller-owned. Complete application-carrier adoption only when the real consumer passes binding, admission, backpressure, refusal and reconnect with the shared provider. Genet owns its browser/WPT/media gates and its selected headed direct/UDP-relay matrix does not prove cross-machine or application-carrier acceptance. Registry publication waits for an engine release containing the pinned DCEP and restart fixes.

## Decisions and boundaries

### RTC ICE restart slice

**Status (2026-11-03): provider qualified with late-check repair; Genet receiving in progress.**

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
and browser lifetime. Server replacement, interface rebinding, DNS/mDNS, TCP/TLS,
media and carrier adoption retain their own gates. Append at most **20 KB** to
the existing family receipt; its total stays below **100 KB**. Genet's receiving
plan owns its separate 1 MB total receipt budget and source/lock/binary evidence.

Murm supplies communication engines; Dramatis/Personae and Djinn retain identity and custody, Gemot membership authority, and applications own their facts. The later foundation promotion places Stickleback in Eidetic; Genet owns Fetch/WebSocket policy. The generic RTC provider has no application admission, invitation, Luggage, radio or conversation dependency; the existing application carrier retains its own framing and trust. Keep original crate identities, ancestry and immutable historical evidence. Sennet retains its independent-source boundary: public documentation and identified black-box captures, excluding upstream firmware implementation and schemas. A host carrier handoff or unauthenticated protocol ACK does not establish recipient read or sender authentication. The stock companion queue read can consume device data before host persistence; do not claim exactly-once or power-loss safety.

## Findings, 2026-11-02

`native/Cargo.toml` pins Stickleback and related foundation packages to Eidetic; `native/crates/transport/src/p2panda_transport/tests.rs` retains the killed-peer test. `apps/signalman/src/message/observe.rs` still rejects other wire titles, `crates/postilion/src/held.rs` holds unverified senders until announce, and `crates/outrider/src/direct.rs` retains bounded fresh-link retries. `apps/signalman/src/radio_comms.rs` contains the radio application owner; `wire/crates/rtc/src/udp.rs` contains the caller-driven UDP provider. MOVE0–MOVE5 and named source adoption are recorded as published at their bounded scopes. R5 stock functional, R6–R8 native and E1a/E1c local results have distinct hardware and peer scopes. Automatic contact restoration was subsequently qualified for selected host, V4 and T114 profiles; do not carry the earlier blanket assertion as current.

## Progress

- 2026-11-03: provider requalified against core `479995928668d0db1f8cfb2aa26f818b9f03919f`:
  58 core ICE tests, nine ordinary checks on working Rust and MSRV, three
  controlled coturn checks and strict all-target wire Clippy pass. The delayed-answer
  regression checks recovery only after fresh authentication. Genet receiving is open.
- 2026-11-03: fixed-server restart provider qualified at `377b9ca1b59f48b0c491e25b7d70bb57a23faedd`,
  using core `235f02199a23f00df2c554ed669218eee9627a8e`. Provider working/MSRV,
  controlled coturn, strict Clippy and 57 core ICE tests pass. Genet's native
  and selected WPT receiving gates pass; full headed restart qualification is
  in progress in its owning receipt.
- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
