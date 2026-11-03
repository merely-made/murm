# Protocol and firmware feature parity

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-10-16_protocol_parity_plan.md`; the original is archived whole.

## Open phases and done conditions

- **Parity matrix:** reconcile current Retinue interfaces/shared-instance and management coverage against the review; inventory pinned reference releases and each endpoint, relay/server, management and optional product feature, with owner, applicability and software/physical/firmware status.
- **Tucket:** extend from bounded endpoint/contact-restore profiles to selected multiple-hop/scoped routes, companion/management and current-release differences; qualify repeater, room-server persistence and sensor roles separately against stock peers, including restart, retry, failure and bounded state on the named image.
- **Retinue:** complete selected host interface, service and platform cases and bounded embedded transport/resource/restart/hostile-traffic cases. Compare against the named RNS reference and keep host, embedded node and modem roles distinct.
- **Sennet:** extend selected routing, channel, discovery/configuration and standalone PKI/counter behavior; independently qualify chosen security, position, telemetry and module roles against public specifications and stock captures, including malformed input, restart and board memory. Meshtastic 2.8 ACK proof is not established by earlier correlated ACKs.
- **Product acceptance:** for each advertised image/role measure full workload and resource reserves and qualify applicable USB, BLE/Wi-Fi, display, GNSS/sensors, persistent configuration, upgrades/recovery, sleep, unattended behavior, power-cut and driver/deadline faults. Shared-radio missed windows are a separate controller gate. Requalify when the workload or image changes.

## Constraints

Keep device role, protocol feature and firmware-product acceptance separate; one endpoint exchange does not prove a router. Named 16 KiB untouched-stack and 25% free-heap reserves apply to the qualified memory profile, not all future images. Public contact storage contains pseudonymous associations, excludes secrets, labels, routes and pending operations, and requires its own scoped opt-in. Its commit is last, malformed or mismatched committed state refuses before erasure, and an uncertain readback is not a rollback promise. V4 live writes use the board quiet guard and make their listening blackout explicit. The current T114 endpoint uses volatile private setup; power cuts and installed daemon/UI remain unqualified.

## Findings, 2026-11-02

`crates/radio-hand/src/instances.rs` owns the bounded runtime; `crates/tucket/src/instance.rs` restores public contacts and handles ACKs, and `crates/sennet/src/instance.rs` requires a caller-reserved packet-ID interval. The later named pressure campaign measured painted high-water and fragmented heaps on both boards; selected V4 storage and T114 Tucket-only service have separate bounded reboot/traffic evidence. These are not simultaneous three-network traffic, broad role parity or unattended acceptance. Host contact recovery and the later board recovery supersede earlier open statements at their exact scopes only.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
