# Murm family move receipts

## 2026-11-01: shared RTC nonblocking host

The source hashes below qualify the provider at the following commit.
Earlier E0/E1a receipts remain in their original, cited location.

```json
{
  "schema": 1,
  "scope": "Shared nonblocking host and async driver; native loopback only",
  "recorded_at_utc": "2026-11-01T21:34:43.922417+00:00",
  "prior_source_revision": "c488754b4dc",
  "gates": [
    {
      "command": [
        "cargo",
        "fmt",
        "--manifest-path",
        "wire/Cargo.toml",
        "-p",
        "murm-rtc",
        "--",
        "--check"
      ],
      "exit_code": 0,
      "summary": [
        ""
      ]
    },
    {
      "command": [
        "cargo",
        "test",
        "--manifest-path",
        "wire/Cargo.toml",
        "-p",
        "murm-rtc",
        "--features",
        "host,udp",
        "--locked",
        "--offline",
        "-j",
        "1"
      ],
      "exit_code": 0,
      "summary": [
        "running 1 test",
        "test host_drives_encrypted_data_without_an_async_runtime ... ok",
        "",
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s",
        "",
        "",
        "running 4 tests",
        "test unspecified_routes_and_zero_budgets_are_refused ... ok",
        "test oversize_input_is_refused_before_protocol_parsing ... ok",
        "test cancellation_and_drop_release_the_socket_without_a_background_owner ... ok",
        "test encrypted_udp_preserves_multiple_channel_modes_and_message_boundaries ... ok",
        "",
        "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s",
        "",
        "",
        "running 0 tests",
        "",
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        "",
        "",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "   Compiling murm-rtc v0.1.0 (C:\\Users\\mark_\\Code\\repos\\murm\\wire\\crates\\rtc)",
        "    Finished `test` profile [unoptimized] target(s) in 1m 39s",
        "     Running unittests src\\lib.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\murm_rtc-39fa67fa852af1ad.exe)",
        "     Running tests\\host.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\host-ff8659dde866c103.exe)",
        "     Running tests\\udp.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\udp-9f321baf9c60242e.exe)",
        "   Doc-tests murm_rtc"
      ]
    },
    {
      "command": [
        "cargo",
        "+1.97.1",
        "test",
        "--manifest-path",
        "wire/Cargo.toml",
        "-p",
        "murm-rtc",
        "--features",
        "host,udp",
        "--locked",
        "--offline",
        "-j",
        "1"
      ],
      "exit_code": 0,
      "summary": [
        "",
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        "",
        "",
        "running 1 test",
        "test host_drives_encrypted_data_without_an_async_runtime ... ok",
        "",
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s",
        "",
        "",
        "running 4 tests",
        "test unspecified_routes_and_zero_budgets_are_refused ... ok",
        "test oversize_input_is_refused_before_protocol_parsing ... ok",
        "test cancellation_and_drop_release_the_socket_without_a_background_owner ... ok",
        "test encrypted_udp_preserves_multiple_channel_modes_and_message_boundaries ... ok",
        "",
        "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s",
        "",
        "",
        "running 0 tests",
        "",
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        "",
        "",
        "   Compiling murm-rtc v0.1.0 (C:\\Users\\mark_\\Code\\repos\\murm\\wire\\crates\\rtc)",
        "    Finished `test` profile [unoptimized] target(s) in 14.56s",
        "     Running unittests src\\lib.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\murm_rtc-1f069e397f3ed93e.exe)",
        "     Running tests\\host.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\host-14df2e66d13774c7.exe)",
        "     Running tests\\udp.rs (C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\udp-30d28788cef31f43.exe)",
        "   Doc-tests murm_rtc"
      ]
    },
    {
      "command": [
        "cargo",
        "clippy",
        "--manifest-path",
        "wire/Cargo.toml",
        "-p",
        "murm-rtc",
        "--features",
        "host,udp",
        "--all-targets",
        "--locked",
        "--offline",
        "-j",
        "1",
        "--",
        "-D",
        "warnings"
      ],
      "exit_code": 0,
      "summary": [
        "",
        "    Checking murm-rtc v0.1.0 (C:\\Users\\mark_\\Code\\repos\\murm\\wire\\crates\\rtc)",
        "    Finished `dev` profile [unoptimized] target(s) in 5.26s"
      ]
    },
    {
      "command": [
        "cargo",
        "check",
        "--manifest-path",
        "wire/Cargo.toml",
        "-p",
        "murm-rtc",
        "--features",
        "host",
        "--locked",
        "--offline",
        "-j",
        "1"
      ],
      "exit_code": 0,
      "summary": [
        "",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "    Blocking waiting for file lock on package cache",
        "    Checking murm-rtc v0.1.0 (C:\\Users\\mark_\\Code\\repos\\murm\\wire\\crates\\rtc)",
        "    Finished `dev` profile [unoptimized] target(s) in 1m 14s"
      ]
    }
  ],
  "host_dependency_boundary": {
    "command": [
      "cargo",
      "tree",
      "--manifest-path",
      "wire/Cargo.toml",
      "-p",
      "murm-rtc",
      "--no-default-features",
      "--features",
      "crypto-ring,host",
      "-e",
      "normal,build",
      "--prefix",
      "none",
      "--locked",
      "--offline"
    ],
    "exit_code": 0,
    "forbidden_hits": []
  },
  "source_sha256": {
    "wire/Cargo.toml": "52f2647ed9e011b55294128796aad27f2c15475461f702ac0eb8515598f6ae5e",
    "wire/Cargo.lock": "2cfc1f6ee40947aa1d106fd2fd80b05c8f57389855eec3cbbe05f5e6cf1adcae",
    "wire/crates/rtc/Cargo.toml": "3ce48f9368e8db33f83d49fe84e5290bcdbd0f32aed3332238076e33c6703373",
    "wire/crates/rtc/README.md": "48cf3dda19fc323f3d7690dfa99b1a2292210d13ec3dfdfd0fcaae2f3f56901b",
    "wire/crates/rtc/src/lib.rs": "feedea43f44336e69ae197618707c8053c9f6a9145309fb551712127a294cba8",
    "wire/crates/rtc/src/udp.rs": "1ddef5b2dae0e4e369134927f7d99a63526e6daacb6b2d5f7eef3a93b8843ec1",
    "wire/crates/rtc/tests/host.rs": "bd1bd81e59a431a28ee70cfff63727c0dd70f4504ee38e9cbef3de6af3ca118c",
    "wire/crates/rtc/tests/udp.rs": "3a8ad00f6cf093c7c59d929fd5856388dfed9fd3afdf5e735be7d1689f6dec8d"
  },
  "rustc": "rustc 1.99.1 (58cf8f9c3 2026-10-13)\nbinary: rustc\ncommit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67\ncommit-date: 2026-10-13\nhost: x86_64-pc-windows-msvc\nrelease: 1.99.1\nLLVM version: 23.1.1\n"
}
```

## 2026-11-01: earlier RTC provider notes

Moved from the plan under the documentation policy; cited E0/E1a files remain
in their original location.

### E1a: selected RTC provider

Mark accepted continuing with RTC after the comparison. `wire/crates/rtc`
now supplies `murm-rtc`, independently selectable from the HTTP/QUIC package.
Its default surface re-exports the one pinned Sans-I/O engine; the optional
`host` adapter owns one concrete nonblocking local socket with caller-selected
packet, datagram and timer budgets. The optional `udp` feature adds Tokio waits
over the same packet driver. It spawns no task or runtime. The host can pause
data-channel reads while still servicing I/O, protocol timers and media.
Application framing, invitations and admission stay outside this package.

The real UDP test exposed the published 0.21.1 release's loss of accepted
data-channel reliability settings. Upstream already fixed both the SCTP
stream's behavior and the public channel attributes in
[`70901c3`](https://github.com/webrtc-rs/rtc/commit/70901c3df6ad92dcb87d97af3d1e2fa8a2e2ae38),
so this provider uses that exact Git revision, including its workspace
protocol crates. It does not patch the registry cache or depend on a root
Cargo patch that downstream consumers would fail to inherit. Registry
publication is disabled until a released engine contains the fix. E0's
published-release source and receipts remain unchanged.

Native qualification covers real ICE/DTLS/SCTP over loopback UDP, two channel
modes including accepted `maxRetransmits: 0`, UTF-8 and a fragmented 48,000-byte
binary message, reverse traffic while application reads are paused, certificate
fingerprints derived from the handshake, local/remote DTLS closure, cancellation
and socket release, invalid route/budget refusal and oversize datagrams.
No induced-loss, saturated-window, media, STUN/TURN, browser or WPT result is
inferred. The first compile failure, the release reliability failure and a
transport-close observation repair retain compact receipts in
`testing/webrtc-evaluation/receipts/`; success qualification is recorded there
separately. All receipts share the existing 1 MB cap and reusable target.

E1a passes all four native cases on working Rust 1.99.1 and shared MSRV
1.97.1, the default core-only compile, strict Clippy and formatting. A
normal/build dependency audit proves one exact RTC source and excludes Tokio
from the core as well as HTTP/QUIC, the peer stack and all application owners.
The source and binary hashes are in `provider-qualification.json`; the
independent graph is in `provider-boundaries.json`. The exact workspace
manifest used for those package-scoped gates is preserved alongside them.
Afterward, `default-members = ["crates/wire"]` preserves the wire workspace's
previous ordinary build selection; the final selection/source audit records
that adjustment without implying an application cutover. No isolated target,
Cargo home or worktree was created. Genet binding and replacement-carrier
qualification remain E1b/E2.

The receiving Genet integration required nonblocking polling without installing
an async runtime on its JS thread. `bind_host`, `poll` and `next_deadline` now
provide that path; the existing async API uses the same packet/timer machinery.
All five native cases pass on Rust 1.99.1 and 1.97.1, including actual encrypted
transfer without an async runtime. Strict Clippy and formatting pass, and the
host-only normal/build graph excludes Tokio and upper-layer consumers. The
shared provider qualification remains native loopback, with STUN/TURN and
application-carrier cutover open.


## 2026-11-01: E1c resolved UDP STUN/TURN

Windows host polling through the WSL-local coturn 4.6.1 fixture at numeric
172.28.40.199:34790. Bind selected by an ordinary UDP route probe; this is
Windows/WSL virtual-network evidence, not cross-machine or headed acceptance.
The fixture uses source-controlled test credentials only. The installed coturn
system service was stopped and disabled; the foreground fixture is task-owned.
No additional target, Cargo home or worktree was created. Reused
C:\t\cargo-targets\retinue and its build subdirectory, debug=0, jobs=1.

- Working and MSRV ordinary gates: 7 passed, 3 controlled-server cases ignored.
- Explicit controlled gate on both toolchains: 3 passed. STUN reflexive candidate;
  relay-only encrypted 48 KB delivery after 2.2 seconds with a 1-second allocation
  refresh cap; bad credentials complete gathering without direct candidates.
- Ordinary regressions include exact endpoint/transaction demux preserving ICE
  requests, a bounded blackhole deadline, DTLS close, backpressure and socket
  release. Core and host normal/build dependency trees contain none of Tokio,
  Genet, murm-wire, mere-transport, Personae, Notochord or Luggage.
- Strict all-target Clippy and core with no built-in crypto compile passed.
- Initial exploratory attempts failed on the test's assumption that the core
  emitted an empty candidate (it emits gathering Complete), a BytesMut test
  conversion, and close flushing suppressed by the first draft. Those were
  corrected before acceptance. Clippy also required moving the unit module.

Commands from wire/: cargo test -p murm-rtc --features udp -j 1;
cargo test -p murm-rtc --features udp --test turn -j 1 -- --ignored --test-threads=1;
cargo +1.97.1 test -p murm-rtc --features udp -j 1 and the same controlled gate;
cargo clippy -p murm-rtc --features udp --all-targets -j 1 -- -D warnings;
cargo check -p murm-rtc --no-default-features -j 1.
Tree SHA256 core=4909790b4fa6bc37cfb0e24f3d80dacbcc44a07ec9e48d74ac30041b24fd84dd;
host=5436ba0614ddae801da2de382f46708fe8d986eaa40614dd5e143070c6dc904a.

DNS, TCP/TLS, mDNS, server replacement, restart, permission-lifetime expiry,
allocation-loss recovery, packet loss, saturation, public deployment, media and
application-carrier acceptance remain open. Release is best effort; the test
asserts socket release, not server receipt of Refresh(0).

Source and lock hashes at qualification (working-file bytes):

```json
{
  "base": "81495e4c46f992e488d6432076fba77b173cc420",
  "files": {
    "wire/crates/rtc/src/udp.rs": "b64fe6b1be75b798509859e6ca081b90bc538224eff925fe124acf00ec176232",
    "wire/crates/rtc/src/udp/discovery.rs": "976e780022d345afe61fb348ced020118ab774eea641d4884a57dc69919df096",
    "wire/crates/rtc/tests/turn.rs": "9c649f5a75ec218bddc16e5b9bb34ab066b89b447fda59bae181f95588811259",
    "wire/crates/rtc/tests/coturn.sh": "cc4a9eae722d8011c14ac2a2ee9c4770fce25a4a15a91c07d2f8890158f38e31",
    "wire/Cargo.lock": "2cfc1f6ee40947aa1d106fd2fd80b05c8f57389855eec3cbbe05f5e6cf1adcae"
  },
  "toolchains": [
    "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
    "rustc 1.97.1 (8bab26f4f 2026-07-14)"
  ]
}
```


## 2026-11-01: relay-policy related-address correction

Review against [JSEP section 7.3](https://www.rfc-editor.org/rfc/rfc9429.html#section-7.3) found that the first E1c draft exposed the physical address in relay candidates. Relay-only candidates now use an unspecified related address and zero port, retained by core SDP/transport views. The forced-relay assertion checks this before exchanging candidates. Working ordinary 7/7, controlled 3/3, strict all-target Clippy, and MSRV controlled 3/3 pass after this correction. Earlier acceptance above remains evidence of its named source; Genet must consume the correction before final relay acceptance.

```json
{
  "wire/crates/rtc/src/udp/discovery.rs": "c0b897e57d9ba48850a4eeed3824518add4fa690bfff43710ca02833049d76f8",
  "wire/crates/rtc/tests/turn.rs": "8c238abe5633f78d816b1eabd97497bb1c10a868675c5935d777f8af07e22d9c"
}
```


## 2026-11-01: local gatherer metadata retained

Genet relay intake caught a local-URL omission: the core deliberately strips the URL in signaling to_json. The driver now supplies the URL explicitly when registering local candidates. The core exposes it on the candidate event, separately from the protocol candidate; receivers retain it as local metadata. Controlled STUN/TURN 3/3 pass on working Rust and MSRV, now asserting event URL presence and signaling URL absence, with strict all-target Clippy passing. Initial new assertions caught the omission; no prior receipt is changed.

```json
{
  "wire/crates/rtc/src/udp/discovery.rs": "f1b0ba5d1d1f8129d34cf275412ac33d9232e80f285bd36a3ea144a3b83d4414",
  "wire/crates/rtc/tests/turn.rs": "fc3840a2f78e3d022839354a60e62e3400adf037e7faba3b9b9de2c98f12991e"
}
```


## 2026-11-01: E1c final receiving and fixture audit

Genet receives the exact supplier revision below. Both engines pass encrypted relay transfer and local metadata/privacy checks; the verbose controlled coturn fixture independently confirms authenticated allocation renewals and lifetime-zero release. The selected server lines and full receiving gates are appended to `genet/design_docs/archive_docs/receipts/2026-11-01_webrtc_conformance_plan_receipts.md`. Adding verbose fixture output changes no provider protocol source. Final ordinary gates on the supplier code pass on both toolchains. All fixture processes were stopped, with no isolated target, Cargo home or worktree retained. The installed WSL coturn package remains available for repeating the fixture; its automatic service is disabled.

```json
{
  "supplier_source": "ebafed19bf68edcfa0c276f0693a2bf45c858174",
  "Genet_receiving_source": "0f38cba4fbe9d5a8158170f18469743b8b7c4433",
  "fixture_sha256": "745ad015cf1c9c8703698794a7c43f8da92c622c5f6d1f1812d3ad7282db9875",
  "controlled_server_confirmations": {
    "allocations": 4,
    "renewals": 22,
    "lifetime_zero_releases": 4
  },
  "final_ordinary_tests": {
    "working": 7,
    "MSRV": 7
  },
  "fixture_stopped": true,
  "coturn_system_service": "inactive and disabled"
}
```


## 2026-11-02: browser interop route diagnostics

Route refusals now carry the requested TransportContext; no protocol payload or credential is included. This exposed the core QueryOnly mDNS default trying to send from 0.0.0.0:5353 to the multicast group through the ordinary peer socket. Genet disables that undriven core path and refuses unresolved mDNS candidates until it has a dedicated resolver route. UDP routing refusal remains strict. The application carrier stays unchanged. Concurrent native session-lane work was merged before pushing, preserving its security-setup changes.

Commands from wire/, target=retinue with its build subdirectory, debug=0, jobs=1, locked/offline: `cargo test -p murm-rtc --features udp`; the same test command with `+1.97.1`; `cargo test -p murm-rtc --features udp --test turn -- --ignored --test-threads=1`; `cargo clippy -p murm-rtc --features udp --all-targets -- -D warnings`. All pass. The controlled fixture was still live for the receiving browser matrix; its stop is recorded at completion in Genet.

```json
{
  "supplier_revision": "01449d2822a8219f7531b2ea8ed59989535a86eb",
  "integrated_main": "85bd99a5c34534c958460c9cbc14545e4983ebb5",
  "sourceSHA256": {
    "wire/crates/rtc/src/udp.rs": "718980f4f0ed5500bfbed830c321ab731b371988313329e812e6b1bc62cdeb73",
    "wire/crates/rtc/src/udp/discovery.rs": "7e7d2e41e88e95cef952bfad38651a07c979788c7c32d72727ab84a2bfb46b14"
  },
  "wireLockSHA256": "2cfc1f6ee40947aa1d106fd2fd80b05c8f57389855eec3cbbe05f5e6cf1adcae",
  "ordinary_tests": {
    "working": 7,
    "MSRV_1_97_1": 7
  },
  "controlled_coturn_tests": 3,
  "strict_all_target_clippy": "pass"
}
```


## 2026-11-02: headed receiving closure

Genet's receiving source `73a2f0ae4be1e9da10b3acc410d910aa355559cb` retains supplier `01449d2822a8219f7531b2ea8ed59989535a86eb`. Its full headed Chrome/Firefox transport matrix passed 32 cases before the subsequent stats-privacy correction; eight targeted browser/engine/route privacy cases and a final cold-start case also pass. Final native ordinary/controlled checks, strict Clippy, selected historical WPT identities and Ortet receiving checks are recorded with their exact source/binary boundaries in `genet/design_docs/archive_docs/receipts/2026-11-01_webrtc_conformance_plan_receipts.md`. The Genet receipt owns the network frontend, scoped browser permissions and retained-profile exception; this does not qualify a carrier cutover.

The verified task-owned foreground coturn process was stopped after the receiving checks; no turnserver remains. Its automatic service remains inactive and disabled. No isolated target, Cargo home or worktree was created by this follow-up. Supplier source is unchanged since its ordinary tests on working Rust/MSRV, controlled coturn checks and strict all-target Clippy recorded above.


## 2026-11-03: E2r provider final gates

```json
{"gate":"E2r-provider","source":"e46a6163a036da07b20cfaa2c7746c530f38d7a4","core":"9767c0f6e3c2b6be7ccba93595ca7e44d3e85334","utc":"2026-11-03T23:06:40.475960+00:00","wireLockSHA256":"256ac71591a845da12addc62fd865b89b1ed6ba5e37d8965c0cf385572fe4fa6","commands":[{"command":["cargo","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1"],"exit":0,"seconds":11.41,"results":["test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","test result: ok. 1 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.05s","test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s","test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","    Finished `test` profile [unoptimized] target(s) in 7.54s"]},{"command":["cargo","+1.97.1","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1"],"exit":0,"seconds":47.2,"results":["test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s","test result: ok. 1 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.05s","test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s","test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","    Finished `test` profile [unoptimized] target(s) in 42.85s"]},{"command":["cargo","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1","--test","turn","--","--ignored","--test-threads=1"],"exit":0,"seconds":4.58,"results":["test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 2.39s","    Finished `test` profile [unoptimized] target(s) in 2.02s"]},{"command":["cargo","clippy","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1","--all-targets","--","-D","warnings"],"exit":0,"seconds":18.38,"results":["    Finished `dev` profile [unoptimized] target(s) in 18.10s"]}],"scope":"same physical socket, resolved UDP STUN/TURN; fixed-server restart, no carrier/interface-rebind qualification"}
```


## 2026-11-03: bounded old-consent responder

```json
{"gate":"E2r-core-consent","utc":"2026-11-03T23:56:21.402824+00:00","source":"3e03d99e5226952faf547b35b07c30ca2fdbaaa4","compiler":"rustc 1.97.1 (8bab26f4f 2026-07-14)","binarySHA256":"632c169f931951006313101a5f90acd16b871f10bc0fa4ab5c6bb71e2563be51","build":"cargo test -p rtc-ice --lib --no-default-features --features crypto-ring --locked --offline -j 1 restart_answers_old_consent","run":"C:\\t\\cargo-targets\\retinue\\build\\debug\\deps\\rtc_ice-b6503882a16b4e84.exe","passed":56,"failed":0,"scope":"Built focused test and directly ran the same complete unit binary to avoid repeating the global Cargo cache wait. Exact old route and credentials, wrong key/source refusal, authenticated response, fresh table separation, repeated restart deadline, expiry and close."}
```


## 2026-11-03: final restart supplier qualification

```json
{"gate":"E2r-provider-final","utc":"2026-11-04T00:10:03.083423+00:00","source":"0138711d9505b2ff14bd881094b7345ca7efa9a3","core":"3e03d99e5226952faf547b35b07c30ca2fdbaaa4","wireLockSHA256":"8d227f6ec12bb41d136b36eaccbc5b35f73cb17926310d4df3082b2d0f2bb75c","commands":[{"command":["cargo","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1"],"exit":0,"seconds":155.23,"results":["    Finished `test` profile [unoptimized] target(s) in 2m 30s","test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s","test result: ok. 1 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.09s","test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s","test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"]},{"command":["cargo","+1.97.1","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--locked","--offline","-j","1"],"exit":0,"seconds":187.72,"results":["    Finished `test` profile [unoptimized] target(s) in 3m 03s","test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s","test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s","test result: ok. 1 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.09s","test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s","test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"]},{"command":["cargo","test","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--test","turn","--locked","--offline","-j","1","--","--ignored","--test-threads=1"],"exit":0,"seconds":72.19,"results":["    Finished `test` profile [unoptimized] target(s) in 1m 09s","test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 2.68s"]},{"command":["cargo","clippy","--manifest-path","wire/Cargo.toml","-p","murm-rtc","--features","udp","--all-targets","--locked","--offline","-j","1","--","-D","warnings"],"exit":0,"seconds":49.89,"results":["    Finished `dev` profile [unoptimized] target(s) in 48.12s"]}],"scope":"9 ordinary tests on working Rust 1.99.1 and MSRV 1.97.1, 3 controlled coturn tests, strict all-target Clippy. Genet owns established-channel restart receiving tests."}
```

The fork retains the nominated data route and authenticated old consent under the same fixed deadline, and preserves the continuing DTLS answer role only for the same remote identity. Public provider regression checks both continued and changed DTLS identities; the relay blackhole check completes two gathering generations without direct candidates. Root radio Clippy passed before this independent wire-only pin; its repeated hook was skipped for that pin, and the explicit wire all-target strict gate above passes. Previous supplier rows remain historical.
