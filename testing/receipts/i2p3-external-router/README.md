# I2P3: retinue over i2pd, and across i2pd and emissary

**Date:** 2026-10-23 UTC (2026-10-22 evening local), Windows 11 host.
**Plan:** [I2P over SAMv3](../../../design_docs/2026-10-21_i2p_sam_seam_plan.md),
phase I2P3.

**Established:**
- Two retinue endpoints, each in its own SAM session on one i2pd 3.0.0
  router, exchanged announces, echo links and a 12,000-byte Resource in both
  directions.
- The same exchange passed across two routers, each way:
  - the hub on the embedded emissary, its leaf on i2pd;
  - the hub on i2pd, its leaf on emissary.
- Every run began with the loopback TCP control at the 500-byte protocol MTU.
- The I2P2 test, refactored in this lane, passed again (run 3).

**Not established:**
- Whether the unknown-tag condition behind the panic below recurred in the
  passing cross run. With the assert compiled out, emissary only logs it,
  and the test installs no subscriber.
- That debug assertions are actually off in the build. That is inferred from
  the profile override and the absence of the panic.
- Stock RNS (I2P4).
- More than one run per configuration.

## Setup

- **i2pd:** `i2pd version 3.0.0 (0.9.71)`, the portable win64 build staged
  in I2P0 (`Code/testing/retinue/i2pd/3.0.0/bin/`).
  - It ran with [`i2pd.conf`](i2pd.conf), a client-only config following the
    owner's I2P2 answers: `notransit`, NTCP2 and SSU2 unpublished, no UPnP;
    no web console, HTTP or SOCKS proxy, or address book; SAM on
    `127.0.0.1:7656`; reseed verification on.
  - It reseeded once on first start from its built-in list; its netDb had 171
    router infos when the tests began and 1,491 at shutdown.
  - It ran from 03:50:52Z to its graceful stop after the last test (05:25Z),
    by `taskkill` without `/F`.
- **emissary:** `murm-i2p` with emissary-core/util 0.4.0, transit off,
  nothing published, storage `Code/testing/retinue/emissary/` (137 stored
  routers, so no reseed).
- **Test:** `native/crates/murm-i2p/tests/live.rs`, with
  `MURM_I2P_EXTERNAL_SAM=127.0.0.1:7656`. The command was:

  ```sh
  cargo test -p murm-i2p --test live <name> -- --ignored --nocapture
  ```

## Results

Times are cumulative from the start of each exchange. The TCP control
finished in 2.08-2.13 s every time.

| Run | First contact | Link a to b | Link b to a | Resource a to b | Resource b to a |
|---|---|---|---|---|---|
| [Both on i2pd](external-run-1.json) (03:58:55Z) | 110,515 ms | 124,375 | 128,261 | 133,764 | 140,404 |
| [Hub on emissary, leaf on i2pd](cross-run-2.json) (05:22:12Z) | 48,204 ms | 49,688 | 51,317 | 54,228 | 56,906 |
| [Hub on i2pd, leaf on emissary](cross-run-2.json) (same run) | 44,235 ms | 46,093 | 48,170 | 51,137 | 54,075 |
| [Both on emissary](emissary-run-3.json) (05:24:27Z) | 28,123 ms | 30,052 | 32,252 | 35,553 | 38,659 |

On i2pd, after first contact, links took 3.9-13.9 s and Resources 5.5-6.6 s.
The both-on-emissary run 85 minutes later took 1.9-2.2 s and 3.1-3.3 s. One
run each cannot say whether that is the router or the hour.

## A failed run, kept: cross-run-1 (04:02:36Z)

- **What passed:** the leg with the hub on emissary and its leaf on i2pd
  (45.6 s in all).
- **What failed:** in the leg with the hub on i2pd and its leaf on emissary,
  an emissary worker panicked at emissary-core 0.4.0
  `destination/session/session.rs` 915. That is a `debug_assert!(false)` on
  the path for a garlic message whose tag matches no tag-set entry and fails
  as NSR. The router task died, and the exchange then timed out at its 600 s
  limit (04:13:28Z). No JSON was written.
- **i2pd's log:** around that time it shows only the aftermath (`Destination
  to connect not found`, `I2P acceptor has been reset`).
- **The owner's response:**
  - The native workspace now builds emissary-core with debug assertions off
    in dev and test, as release does.
  - `murm-i2p` now reports how the router ended (`EmbeddedRouter::stopped`,
    `RouterEnd`).
  - The live tests race each emissary leg against it, so a dead router fails
    them at once.
  - The finding is in the plan's upstream ledger and is not filed upstream.

The cross run was then repeated as cross-run-2, above.

## Windows firewall

The owner chose to cancel or ignore Defender prompts for `i2pd.exe` and the
test binary. I did not watch for them.
