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
