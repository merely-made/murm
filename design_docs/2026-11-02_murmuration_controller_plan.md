# Murmuration controller and shared-radio measurements

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-09-10_murmuration_controller_plan.md`; the original is archived whole.

## Open phases and done conditions

- **MC4 resident acceptance:** measure high-water, fragmentation and full configured-capacity workload for each exact image; test reservation power removal, driver/restoration faults, adversarial host/RX pressure, deadline return and missed home opportunities. Preserve identities and monotonic reservations; separate a failed restoration from successful return. Compare against the later bounded memory profile in [protocol parity](2026-11-02_protocol_parity_plan.md), without transferring results across builds or workloads.
- **Resident controls and maintenance:** expose configured home, pin and bounded excursion settings through the appropriate firmware/UI owners. Add quiet-maintenance replenishment of the Retinue announce lease without restarting, with authoritative reservation before use and failure refusing transmission.
- **MC4 cooperative schedules:** after local switching, configure recurring schedules and optional authenticated peer coverage/rendezvous. Measure actual captures and misses separately from promised coverage. Automatic packet-triggered switching and implicit cross-mesh forwarding are not selected.
- **MC5 shared-radio cost:** first identify/calibrate the PPK2 and prove VBUS isolation. For the designated third V4 record its own firmware, profiles, supply, sampling and event alignment. Measure single-protocol standby/RX/TX/workload baselines; then transition and excursion time/energy relative to those baselines. Send home traffic at a known cadence and report opportunities, captures, misses, unknown and unobserved time per excursion. Do not rewire the qualified pair; a trace without calibration is not acceptance.

**Withdrawal note, 2026-11-14 ([protocol_parity D1](2026-11-02_protocol_parity_plan.md#protocol_parity-d1--foreign-meshes-through-stock-radios)).** Mark: "Stock radios only": a stock MeshCore or Meshtastic radio holds the keys, we hold none and compose no crypto; "the resident-protocols firmware image and WN7's foreign-adapter condition are withdrawn." The MC4 and MC5 items above were measured on that image with Retinue, Sennet and Tucket resident; the image, `radio-hand`'s `instances` runtime and the MC3/MC4 probes are removed. Their statuses are left as they were: whether these phases continue for Reticulum-only personalities is not decided by this note.

## Constraints and deferred work

A single SX1262 listens to one compatible active profile. Protocol instances and their identities survive bounded visits or explicitly report loss; board ownership confirms transitions and deadlines. Power-cut, physical fault, memory, protocol parity and shared-radio measurements are distinct gates. The power meter was reported present but disconnected; wiring and calibration need fresh confirmation. Exact trace alignment via GPIO remains optional, with timestamp alignment identified if used.

## Findings, 2026-11-02

`crates/retinue/src/node/poll.rs` now calls `expire_sessions`, whose `node/session.rs` contract removes associated and orphaned resource state and reports expiry; the original idle-link cleanup finding is therefore superseded at source level, without a fresh test run. `crates/radio-hand/src/instances.rs` contains the resident runtime; `firmware/heltec-v4-phy/src/resident/build.rs` reserves its announce lease during construction. MC0–MC3 and the named MC4a–c slices have the original's scoped software, host-retained and physical results. MC4c's sampled stack was not high-water, and its missed-request observation does not establish multi-board coverage. The later parity campaign qualifies a named synthetic pressure profile and bounded Tucket endpoint, not every MC4 failure or power-cut condition.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
