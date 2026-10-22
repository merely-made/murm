# I2P2: retinue over an embedded emissary

**Date:** 2026-10-22, Windows 11 host. **Plan:**
[I2P over SAMv3](../../../design_docs/2026-10-21_i2p_sam_seam_plan.md), phase I2P2.

**Established:** two retinue endpoints, each speaking through its own SAM session
on one in-process emissary router, exchanged announces both ways, opened a link
each way that echoed, and moved a 12,000-byte Resource each way. They did this
twice, on the public I2P network. In the same run, a loopback TCP control did
the same exchange at the same 500-byte protocol MTU.

**Not established:** which tunnels carried the traffic. Routing through tunnels
rather than local delivery is inferred from emissary's code: a destination
looks up its peer's lease set through the netDb and has no local-delivery path
(`destination/mod.rs` 294-330 in emissary-core 0.4.0). Two runs say nothing
about time-to-connect distribution beyond its spread. Not tested: an external
router, cross-router traffic or stock RNS (I2P3, I2P4).

## Setup

- **Router:** `emissary-core` / `emissary-util` 0.4.0 from crates.io, through
  `murm-i2p` (`native/crates/murm-i2p`).
  - **Transit:** off.
  - **Publish:** off. NTCP2 and SSU2 still bind on `0.0.0.0` with
    OS-assigned ports, as emissary binds any enabled transport, but advertise
    no address. No UPnP, no forwarded port.
  - **SAM:** `127.0.0.1` on an OS-assigned port.
- **Storage:** `Code/testing/retinue/emissary/`, persistent (656 KB after
  run 2). Reseed: emissary-util's built-in HTTPS servers, IPv4 only, below 25
  stored routers.
- **retinue:** feature `i2p`, at the branch's tree.
  - The hub listens on a destination generated for the run (`DEST GENERATE`,
    signature type 7) and has no session options.
  - The leaf dials it by base64 destination, with default `I2pTimers`.
- **Test:** `native/crates/murm-i2p/tests/live.rs`, `#[ignore]`d. The command
  was:

  ```sh
  MURM_I2P_STORAGE=C:/Users/mark_/Code/testing/retinue/emissary \
  MURM_I2P_RECEIPT=<run>.json \
  cargo test -p murm-i2p --test live -- --ignored --nocapture
  ```

## Results

Times are from the start of each exchange; the link and Resource figures are
cumulative.

| | Run 1 ([json](run-1.json)) | Run 2 ([json](run-2.json)) |
|---|---|---|
| Started (UTC) | 19:59:02 | 20:00:06 |
| Router storage at start | empty | 137 routers |
| Reseed | 137 routers | skipped |
| Router start | 4,587 ms | 1,360 ms |
| Destination | `zmupx56e…ta.b32.i2p` | `q2nxw5up…nq.b32.i2p` |
| TCP control, all five steps | 2,085 ms | 2,091 ms |
| I2P: both announces heard | 26,077 ms | 58,216 ms |
| I2P: link a to b | 27,304 ms | 59,909 ms |
| I2P: link b to a | 28,666 ms | 61,591 ms |
| I2P: Resource a to b | 30,614 ms | 64,029 ms |
| I2P: Resource b to a | 32,663 ms | 66,577 ms |

Once announces have crossed, each link round-trip takes 1.2 to 1.7 s and each
12 KB Resource 1.9 to 2.6 s, in both runs. The first-contact time (tunnel
builds, lease-set publication, the first `STREAM CONNECT` after
`CANT_REACH_PEER` retries) is most of the total, and it doubled between runs.

## A failed run, kept

The first attempt (19:57:49Z) failed in the TCP control before the router
started: `left: Data, right: Resource`. Over loopback TCP, retinue autoconfigures
a large link MTU, so 12,000 bytes went as a single data packet. The control
was then pinned to `HwMtu::Protocol` to match I2P's link MTU. That is a test
fix, not a code change, and nothing contacted the network in that attempt.

## Windows firewall

The owner chose to cancel or ignore a Defender prompt if one appeared. I did
not watch for one during these runs.
