# I2P4: stock RNS 1.5.7 against retinue over I2P

**Date:** 2026-10-23, Windows 11 host. **Plan:**
[I2P over SAMv3](../../../design_docs/2026-10-21_i2p_sam_seam_plan.md), phase I2P4.
**Harness:** `crates/retinue/oracle/interop_i2p.py` drives stock RNS 1.5.7
(I2PInterface on i2pd 3.0.0, SAM `127.0.0.1:7656`) against murm-i2p's
`rns_i2p_peer` example (retinue on an embedded emissary 0.4.0, or on i2pd with
`--retinue-sam`). A loopback TCP control runs first in every run.

## What is established

- **Retinue dials stock RNS, no IFAC: passed in runs 1, 2, 5 and 6.** Each
  side heard the other's announces; retinue linked to RNS and got its echo;
  RNS linked to retinue and got its echo; a 12,000-byte Resource went each
  way with matching sha256, and RNS's Resource reached COMPLETE.
- **Retinue dials stock RNS, IFAC 16: passed in runs 1 and 2.**
- **Stock RNS dials retinue, no IFAC: passed in run 2.**
- **Stock RNS dials retinue, IFAC 16: passed in run 4,** with an explicit
  `ifac_size = 128` in RNS's config, the workaround for an RNS defect (below).
- **Keyfile layout (F4): passed in runs 2, 5 and 6.** With stock RNS on
  `static_transport_identity = Yes`, retinue's `i2p_keyfile(storagepath,
  "I2P", transport hash)` named the exact file RNS wrote, and loading it gave
  the b32 RNS reported.
- **Redial cadence (owner's reconnect condition), on both routers.** After
  RNS was killed and restarted on the same storage and b32:
  - the first redial came 14.8 s, 15.0 s and 15.0 s after the stream ended
    (runs 2, 5, 6);
  - on emissary, each further redial came 15.0 s after the previous refusal
    (34 and 89 attempts);
  - the route to RNS's destination was kept in every run.
- **Back online on the same interface: on i2pd only.** In run 6, retinue on
  i2pd came back online 107.7 s after the kill (one redial; the old stream
  had ended at 265.2 s, the redial at 280.2 s).

## Post-restart failures: diagnosed (runs 7-9)

With the session kept, two things failed after the RNS restart:
- **On emissary,** retinue never got back online: every `STREAM CONNECT`
  got `CANT_REACH_PEER`, in runs 2 and 5.
- **On i2pd,** retinue came back online, but the first link timed out, in
  runs 6 and 7.

**Run 7** instrumented the i2pd case:
- the restarted RNS logged at level 7, and **logged no incoming connection
  at all**;
- retinue's interface counters went `tx` 26 → 28 → 30 with `rx` steady at
  18, so retinue sent and nothing came back;
- a second link attempt 30 s later failed the same way, so this was not a
  startup race.

i2pd had answered `STREAM CONNECT` with `RESULT=OK`, but the stream never
reached the new RNS process. retinue's `establish` never reuses a link, which
rules out stale-link reuse.

**Runs 8 and 9** tested the hypothesis that the kept SAM session holds stale
state for the restarted destination. They used `I2pPeer::fresh_session_per_redial`
(experimental, default off), which drops the session after a stream ends or a
connect is refused. On both routers the first redial reached the restarted
node, RNS logged the accepted connection, and the post-restart link passed:

| Run | Router | Redial began | Session ready | Online | Post-restart link |
|---|---|---|---|---|---|
| 8 | i2pd | +14.8 s | +26.8 s | +30.2 s (123.3 s after kill) | passed |
| 9 | emissary | +14.9 s | +23.2 s | +26.1 s (135.6 s after kill) | passed |

All times are measured from the old stream's end. The cause is stale per-session router
state, not retinue's link handling. Whether retinue should recreate the
session by default is open for the owner. The harness's `reconnect` FAIL in
runs 8 and 9 came from timing the redial from `STREAM CONNECT`, which here
follows the session build. It now times a redial from its first SAM command.
- **Runs 2, 5 and 6 scored `reconnect` as FAIL** under the scoring then in
  the script: it timed only gaps after refused connects, and included the
  echo. The numbers above re-derive the owner's condition from the same logs.
  The script now scores it that way.

## Confirmation with the new default (runs 10-13)

At the owner's direction, `fresh_session_per_redial` is now on by default
(the field stays, so a host can keep one session). The full orchestrator then
ran once, as run 10, with retinue on emissary and no flags:

- **Passed:**
  - the TCP control;
  - retinue dials RNS (exchange, keyfile, **reconnect**, and the
    **post-restart link**);
  - retinue dials RNS with IFAC 16;
  - RNS dials retinue with IFAC 16.
- **On reconnect,** retinue started a redial 15.0 s after the stream ended,
  came back online 135.1 s after the kill, and kept its route; the link to
  the restarted RNS worked. This is the case that never recovered in 40
  minutes with the session kept (run 5).
- **Failed, in one step:** RNS dials retinue with no IFAC. Everything up to
  RNS's own Resource to retinue passed: announces, both echoes, and
  retinue's Resource to RNS. Then RNS reported `RES_OUT 7` (`FAILED`) and
  retinue got `resource canceled by sender`. The new default does not touch
  this variant: retinue is the listener there, not the dialer.
- **Reruns 11-13:** the same variant three times with RNS at log level 7
  passed all three. That makes this variant 1 failure in 5 (runs 2, 10, 11,
  12, 13). Run 10 logged RNS at level 3, so nothing records why its sender
  gave up.

## Run 10's Resource failure: diagnosed (runs 14-33 and i2pd's log)

- **Runs 14-33:** the same variant 20 more times at RNS level 7, all passed.
  That makes it 1 failure in 25. No run logged a single RNS Resource watchdog
  retry (advertisement, part request or proof).
- **i2pd's log of run 10** (`Code/testing/retinue/i2pd/3.0.0/data/i2pd.log`,
  local time) holds the failing window. The variant ran from about 18:54 to
  19:09:31, its deadline. RNS's Resource to retinue began at about 18:55:21.
  The variant's only I2P stream (sSID 2605253764, RNS's peer stream to
  retinue) then stalled in both directions:
  - 18:55:27-43: i2pd resent its outgoing messages (#2 to #4) on another
    remote lease and another outbound tunnel;
  - 18:55:44-58: incoming message 15 was missing while 16 to 21 queued behind
    it, so RNS got nothing from retinue on that stream;
  - i2pd's tunnel tests failed 35 times between 18:53 and 19:10 (about 2 a
    minute). Across runs 14-33 they failed 48 times in 54 minutes (about 0.9
    a minute), with no stream resends and one single-message gap that
    recovered.
- **RNS's tolerance:** an RNS 1.5.7 sender waits longest for proofs and part
  requests (over 170 s here). Its shortest patience is the advertised stage:
  5 waits of 6 × RTT + 1 s, which is 20-107 s at the link RTTs these runs
  measured (0.5-3.4 s). The stall lasted at least 31 s. Retinue had received
  the advertisement, since its error names the Resource, so its part request
  was probably the message stuck behind message 15. Run 10 logged RNS at
  level 3, so which watchdog path fired is inferred, not logged.
- **Verdict:** I2P loss on the path between the two routers outlasted RNS's
  sender tolerance. Nothing points at retinue.

## Defects found in other software

All are in the plan's upstream ledger, and none is filed.

- **RNS:** one failed first lookup closes an I2P peer's event loop for good
  (run 1). The harness now waits until i2pd resolves retinue's b32 before
  starting RNS.
- **RNS:** a non-transport node gets a new I2P b32 at every start (runs 1
  and 2). The harness sets `static_transport_identity = Yes`.
- **RNS:** I2P `peers` with IFAC and no explicit `ifac_size` fail on every
  frame (run 2). Run 4 passed with `ifac_size = 128`.
- **i2pd 3.0.0:** a SAM `HELLO` split across TCP segments drops the
  connection. That broke run 3 (retinue on i2pd sent no command). The
  example's logging proxy now forwards whole lines.

## Runs

| Run | Start (UTC) | Setup | Result |
|---|---|---|---|
| [1](run-1.json) ([log](run-1.log)) | 19:22:01 | all variants, emissary | dial no-IFAC and IFAC 16 passed; RNS-dials failed (RNS first-lookup defect); keyfile and reconnect failed (RNS ephemeral identity) |
| [2](run-2.json) ([log](run-2.log)) | 20:23:44 | all variants, static identity, lookup wait | all exchanges passed except RNS-dials IFAC 16 (RNS IFAC defect); keyfile passed; reconnect never online (emissary) |
| [3](run-3.json) ([log](run-3.log)) | 21:06:01 | retinue on i2pd | void: the proxy split `HELLO`, which i2pd drops |
| [4](run-4.json) ([log](run-4.log)) | 21:21:20 | RNS-dials IFAC 16, explicit `ifac_size` | passed |
| [5](run-5.json) ([log](run-5.log)) | 21:23:22 | dial + reconnect, 40-minute window | exchange and keyfile passed; 89 redials, never online (emissary) |
| [6](run-6.json) ([log](run-6.log)) | 22:08:30 | dial + reconnect, retinue on i2pd, fixed proxy | exchange and keyfile passed; online after 107.7 s, route kept; first post-restart link timed out |
| [7](run-7.json) ([log](run-7.log)) | 22:25:46 | as 6, instrumented (RNS level 7, `STATS`, retry) | reconnect passed; both post-restart links timed out; RNS saw no connection |
| [8](run-8.json) ([log](run-8.log)) | 22:34:02 | as 7, `--fresh-session` | post-restart link passed; online 123.3 s after kill |
| [9](run-9.json) ([log](run-9.log)) | 22:39:42 | retinue on emissary, `--fresh-session` | post-restart link passed; online 135.6 s after kill |
| [10](run-10.json) ([log](run-10.log)) | 22:48:56 | all variants, new default (fresh session), emissary | all passed except one step: RNS's Resource to retinue in RNS-dials no-IFAC (`FAILED`, sender cancel) |
| [11](run-11.json) ([log](run-11.log)) | 23:16:28 | RNS-dials no-IFAC, RNS log level 7 | passed |
| [12](run-12.json) ([log](run-12.log)) | 23:20:36 | as 11 | passed |
| [13](run-13.json) ([log](run-13.log)) | 23:22:58 | as 11 | passed |
| 14-33 (`run-14` to `run-33`, json and log) | 00:13:48 (Oct 24) to 01:04:42 | as 11, looped until a failure (capped at 20) | all passed |

Exchange times, from the start of each exchange to RNS's Resource completing:
- TCP control: 5.3 s every time.
- Retinue dials RNS: 509 s (run 1), 99 s (run 2), 55 s (run 5), 172 s (run 6).
- Retinue dials RNS, IFAC 16: 146 s and 78 s.
- RNS dials retinue: 24 s (run 2) and 50 s (run 4), after i2pd took 30 s and
  63 s to resolve retinue's b32.

## Setup notes

- i2pd 3.0.0 ran with the I2P3 client-only config. Windows Defender had
  quarantined `i2pd.exe` (`Trojan:Win32/Kepavll!rfn`, a heuristic) before
  these runs; the owner restored it, and it hashed to the published
  `6f501e1f…` afterwards.
- emissary storage: `Code/testing/retinue/emissary/` (warm).
- RNS came from the oracle venv, pinned at 1.5.7 (wheel sha256
  `0e4aaf8c…`).
- Each variant used a fresh RNS config directory, removed afterwards.
