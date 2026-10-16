# Browser callback lifetime qualification

A failed browser join can discard a connecting initiator while its JavaScript channel and peer remain alive. The old destructor closed those objects with seven callbacks still referring to Rust closures being dropped. A later close event could invoke a dead closure.

The destructor now detaches all five channel handlers and both peer handlers before closing the objects. Explicit live `close()` retains its callback route and terminal notification behavior. Two browser regressions exercise retained JavaScript objects after destruction and exactly one terminal notification across repeated explicit close callbacks.

Qualification on the frozen `b49df894635bac95823284efee6c445885eaf62a` supplier base passed 44 portable unit tests and 8 transcript vectors under Rust 1.99.0. All 20 browser unit tests passed in headed Chrome 155, including both regressions, through the official wasm-bindgen 0.2.129 interactive runner with `NO_HEADLESS=1` on a loopback origin. The original page screenshot was exported through Chrome's visible Capture screenshot command. Browser output reported 20 passed, 0 failed, 0 ignored. The runner emitted an initialization deprecation warning and a favicon 404; neither was a failed test or uncaught callback exception.

The receipt hashes the changed source, test WASM, runner and original screenshot. This is bounded callback lifetime qualification. It does not qualify consumer admission, default-Chrome mDNS signaling, WAN/TURN connectivity, the broader Graphshell page wall, or Cleromancy C1. Those remain separate consumer receipts. The branch intentionally preserves the frozen supplier family; later upstream Signalman and URL publication changes remain on main.
