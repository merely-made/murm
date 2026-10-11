# Signalman desktop stack and custody refresh

**Status (2026-10-11):** complete, verified on Windows before landing. This
closes the stale nested desktop build gate recorded by Chatelaine P4.

## Phases and done-conditions

- [x] Resolve one source for each shared type, using the qualified Mere
  renderer cohort; both the nested desktop and root Signalman compile locked.
- [x] Request station keys and grants from Djinn; absent or Locked custody
  refuses startup before creating local state or opening a radio.
- [x] Run the desktop and core suites, plus real absent/Locked receipts and
  a deliberately restored local-seed fallback that fails those receipts.
- [x] Restore the source byte for byte, rerun both receipts, and retain hashes
  and logs in the [manifest](../validation/signalman-refresh-receipt.json)
  and [evidence bundle](../validation/signalman-refresh-logs.zip).

## Findings (2026-10-11)

The desktop was on Mere `7019` and an older Genet cohort. Local and Git
Retinue packages split `selvage::PhyProfile` identity. Its station actor also
opened its own wallet. The refresh uses Mere
`43cd7f24afea6b2d3b54939f2093c098da50713b`, Genet
`cec760f4ae6ce336823bc2be250d9ab4e35a8a47`, Netrender
`46606610cd387e2e6956d8a3ebbcf442eab210d9`, Vello Classic 0.11.0 at
`5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a`, and wgpu 30.0.1.
Personae and Gaz use Dramatis `0234e7eea6f2ddf2330a71b9425da01b06ae0391`.
The desktop's locked metadata contains 1,049 packages, one source for each
shared type-bearing family, and no Castellan package. Root Signalman's
optional `mere-comms` uses the same Mere revision.

In [the station actor](../apps/signalman-desktop/src/station.rs), Djinn
releases only station-scoped keys and signs grants. The desktop persists a
public device id after release and grant issuance. It uses Djinn's reported
wallet root for public grant records and rechecks custody before each poll.
A corrupt public scope is refused. A former application wallet at a different
root is preserved and refused until Djinn adopts it, avoiding a silent new
station identity. This checks the public manifest's existence only.

Windows Rust 1.98.1 checks both workspaces locked and offline. The desktop
harness reports 71 passes and one ignored test: 70 software tests ran, and
the physical live-station bench returned early because its activation
environment was absent. The ignored custody test runs separately against an
absent endpoint and a real isolated Locked Djinn, before and after the
negative control. Signalman's core passes 62 tests with all features.

The negative control restores an actual application-local master seed path.
Both custody tests fail at the local-state assertion, then pass after exact
source restoration. Locked keeper files remain byte-identical. The runner
stops its owned hosts and removes its marker-identified synthetic runtime.
No new headed window or physical radio receipt is claimed. Cooperative
revocation and the management plan's remaining headed gate retain their
existing boundaries.

## Progress

2026-10-11: source redirects, renderer pins and the desktop station authority
were updated in the primary checkout. Existing Tabard work was preserved.
Both locked checks, both suites and all custody controls passed. The primary
Mere and Retinue build caches were reused; no worktree or Cargo home was made.
