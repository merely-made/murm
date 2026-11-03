# Linkboy public flashing: remaining work

**Status (2026-11-02): plan.** The [original flashing plan](archive_docs/2026-11-02/2026-08-08_linkboy_public_flashing_plan.md) preserves F0–F7 decisions and scoped receipts. Later evidence must be checked before claiming a physical gap closed.

## Phases and done-conditions

1. **F3 factory first flash.** Show an as-shipped V4 and an as-shipped T114 reaching a valid immutable plan without first running Retinue. Require explicit recorded carrier confirmation where the loader cannot identify the board; contradicting observed facts refuse. Preserve device identity through unplug, reset and re-enumeration.
2. **F4 V4 recovery.** Prove V4 recovery using its own reset/boot controls without relying on the application, including non-returning or interrupted writes. Require the expected application after helper success, or `RecoveryRequired` on silence. Preserve recovery instructions and secret-free receipts. The original records T114 double-tap and a separate V4 state-preservation fact; neither is the missing V4 hardware-entry proof.
3. **Remaining public route facts.** Keep the Meshtastic T114 catalog entry partial until its own app/serial/mobile interface confirms the installed version. Investigate repeated T114 Windows CDC semaphore timeouts under their own owner. A cross-platform claim requires exact-device flash and recovery on each named platform.
4. **Authenticated catalog before network fetch.** Complete the offline signing-key ceremony, install the public trust root and publish and verify the first real signed index under the existing catalog-auth authority. Done when the authenticated index, trusted key, package bytes and rejection cases are receipted; a static digest-pinned site projection is a separate bounded result.
5. **Optional S2–S7.** Local BLE flashing requires authenticated selection, a real update protocol, verification, progress, reconnect and cable recovery. Over-link image delivery requires authenticated management, signed authorization and replay protection, resumable bounded chunks, whole-image verification, airtime/battery budgets, safe staging, trial health and rollback, power-cut and cable recovery, and per-target layout proof across LoRa/BLE/WiFi. Remote listener policy requires FS2/FS3 authority, acknowledgement before apply, fallback and audit. Browser install reuses package/plan authority and retains native recovery. Additional boards require exact manifests and physical discovery/flash/recovery. Fleet work requires explicit identities, per-device plans, resumability and independent receipts; serial is the initial default.

## Decisions and constraints

Linkboy owns package admission and immutable plan execution. A processor, port or helper success alone does not establish the carrier or returned application. Network catalog fetch requires authenticated index verification. Firmware choice must not imply unproven persistence, generic OTA or public platform support. The T114 UF2 route was promoted and is not an open sidequest.

## Findings (2026-11-02)

`apps/linkboy/src/discovery.rs:22` models T114 loader facts; `apps/linkboy/src/receipt.rs:43` models flash receipts. These source facts do not establish factory-device or V4 button-entry physical acceptance.

## Deferred and follow-on

S2–S7 remain optional, not prerequisites for the already qualified cabled release. Link listener-policy authority to the current executive and control plans when that work is selected.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.