# WebRTC evaluation

An independent, non-shipping upstream API/dependency probe for
`genet/design_docs/2026-11-01_webrtc_conformance_plan.md`, E0. Supplier
ownership and extraction remain in
`murm/design_docs/2026-10-13_murm_family_move_plan.md`.

Exact roots: `rtc = 0.21.1` with `crypto-ring`, `str0m = 0.23.1` with
`rust-crypto` and no examples/default crypto, and `webrtc = 0.21.1` with
`runtime-tokio,crypto-ring`. The committed lock fixes transitive versions.
They are resolved separately by `dependency_report.py`; enabling both core
features for the native tests does not establish either independent footprint.

From the Murm checkout in PowerShell:

```powershell
$env:CARGO_TARGET_DIR = 'C:\t\cargo-targets\retinue'
$env:CARGO_BUILD_BUILD_DIR = 'C:\t\cargo-targets\retinue\build'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
cargo test --manifest-path testing/webrtc-evaluation/Cargo.toml --locked --no-default-features --features rtc-core,str0m-core -j 1
python testing/webrtc-evaluation/dependency_report.py
```

The target is Murm's existing pre-rename reusable target. The explicit build-dir
setting prevents the inherited Cargo configuration from creating a second
build tree in this probe. There is no isolated Cargo home or worktree.

The API probes check data-only SDP offer/answer, current/pending descriptions,
rollback, invalid channel options and closed-connection refusal; two negotiate
SDP across the different engines. They do not run ICE/DTLS/SCTP, send data,
drive a browser, run Genet's JS bindings or establish WPT conformance. The
separate async graph is a dependency measurement only, not a runtime test.

Retained receipts: **1 MB total cap**, compact JSON/text only. No full Cargo
metadata or compiler logs. `dependency_report.py --output <new-path>` creates
an immutable receipt and refuses to overwrite one. Counts include reachable
normal and build dependencies on the recorded host target and exclude this
probe; they are not the old comparison's build-byte measurements.

E0 complete locally on Windows/Rust 1.99.1: six API/SDP probes pass, including
offer/answer across engines in both directions. `receipts/native-tests.json`
records source/lock/binary hashes. `receipts/dependency-report.json` records
133/165/189 transitive package versions for str0m/rtc/async webrtc respectively.
`receipts/existing-carrier-crypto.txt` verifies that the actual locked Murm
carrier also brings AWS-LC through dimpl's certificate-generation feature.
This directory adds no dependency to a shipping workspace. Browser/WPT,
packet/data, relay, media and actual MSRV execution remain unqualified here.
