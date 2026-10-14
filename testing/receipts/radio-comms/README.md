# Signalman LXMF-to-Comms software receipt

**Status, 2026-10-13:** software path qualified. Real-radio, headed and fresh
custody-daemon acceptance remain separate. The canonical plan is
`murm/design_docs/2026-10-13_murm_family_move_plan.md`, R1/R2.

The concrete Signalman adapter and its attended face share one durable message
journal. Comms success records a queued intent; carrier handoff is not recipient
delivery or a read receipt. Current station operations remain behind Mere's
Djinn-authorized lease. Saved public identity can support offline authoring,
but ambiguous station history is not selected automatically.

`qualification.json` records the exact supplier refs, changed Rust/manifest/lock
input hashes, log hashes and selected dependency graph identities. The fresh
scope passes 30 portable tests on Rust 1.99.0, 20 portable tests on 1.97.1,
38 Signalman tests on each compiler, and 41 desktop/headless cases on 1.99.0
(28 library, six management-shell, seven network-face). One existing isolated
custody receipt is ignored, not counted as a pass. Bare Comms Wasm compilation,
desktop all-target compilation and both implementation libraries' strict
Clippy checks pass. MSRV desktop compilation is not claimed.

The main receipt uses the actual Comms router, Signalman's message writer,
Outrider signing/verification, Retinue's in-process packet carrier, authenticated
Postilion receive projection and Redb reopen. It preserves message identity,
carrier IDs/mode, direct receive, duplicate suppression and explicit contact
saving. Additional cases cover unsupported drafts before mutation, station
scope, serialized concurrent queueing, bounded/deduplicated handoff, durable
acknowledgement, matched-peer retry and offline authoring after reopen.

Reproduce with the repository toolchain and its declared MSRV:

```sh
cargo +1.99.0 test -p signalman --features comms --lib --locked
cargo +1.97.1 test -p signalman --features comms --lib --locked
cargo +1.99.0 test --manifest-path apps/signalman-desktop/Cargo.toml --lib --test management_shell --test network_face --locked
cargo +1.99.0 check --manifest-path apps/signalman-desktop/Cargo.toml --all-targets --locked
cargo +1.99.0 clippy -p signalman --features comms --lib --locked -- -D warnings
cargo +1.99.0 clippy --manifest-path apps/signalman-desktop/Cargo.toml --lib --locked -- -D warnings
```

The portable Comms checks run in Mere at the supplier/source revision above.
The profile is Signalman's versioned application text carried by LXMF. Wider
stock-LXMF bodies, Comms voice presentation, other radio adapters and automatic
multi-station routing are not covered. The diagnostics summarize journal facts;
radio/route captures retain their own authority, age and qualification.
