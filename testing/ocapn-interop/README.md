# Independent OCapN wire-subset check

This runner invokes fifteen original tests from the unchanged upstream
[`ocapn/ocapn-test-suite`](https://github.com/ocapn/ocapn-test-suite) at
`31f0b806108a55d76a6ede5af7e413818dfb9221`. It does not modify or replace the
supplier's tests, protocol types, serialization, or network implementation.
Run it with a Python environment containing the supplier's documented
dependencies (`cryptography` and `stem`). The tests communicate with a separate
native `murm-ocapn` example process.

Build the explicit fixture feature, then run this harness using the resulting
example binary (Cargo configuration may change its location):

```sh
cargo build --locked --manifest-path native/Cargo.toml \
  -p murm-ocapn --features suite-fixture --example suite_server
python testing/ocapn-interop/run.py \
  --supplier /path/to/immutable/ocapn-test-suite \
  --server-binary native/target/debug/examples/suite_server \
  --output /tmp/murm-ocapn-interop
```

The source checkout must match the pinned commit and remain clean before and
after the test process. The output directory must be new. It contains structured
per-test results, complete process logs, the exact selected test identifiers,
supplier/runtime source hashes, runtime source snapshots, workspace lock and
binary hash, and Python dependency versions. Runtime source, lock and binary
must also remain unchanged during execution. A partial run, skip, error,
failure, unexpected server exit, or input mutation leaves `overall_passed:
false` and a failing process exit.
The runner adds no retries. The supplier's existing network-related `setUp`
retry decorator remains unchanged and is recorded in the summary.

The fifteen tests cover start-session validation (three), abort (one), delivery
and promise pipelining (four), promise listening (three), export GC (three), and
answer GC (one). The two crossed-hello tests and all third party handoff tests
are explicitly excluded because they are unsupported by the bounded initial
profile. Tests are not removed from this list in response to a failing run.

This is interoperability evidence for the pinned suite's CapTP `1.0` wire
subset over its **testing-only plaintext TCP netlayer**. That upstream netlayer
has no transport security or production authority guarantees. It establishes
neither full OCapN/current-draft conformance nor Tor, Noise or native-iroh
netlayer interoperability. Secure endpoint integration and application
capability authority have their own tests and acceptance gates. The fixture
objects are the supplier's declared test behaviors, not product exports.
