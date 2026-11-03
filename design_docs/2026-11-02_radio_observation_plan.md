# Radio observation and Signalman availability

**Status, 2026-11-02: plan.** Successor to `archive_docs/2026-11-02/2026-09-06_radio_observation_plan.md`; the original is archived whole.

## Open phases and done conditions

- **O3, measurement:** compare identical RF traffic with collection absent, normal, slow and stopped, and measure T114 record/encode CPU, interrupt/FIFO and USB drain cost, overwrite and radio-unavailable time under a declared workload. Record ring capacity and loss; observation reads must cause no quiet-window entry or control-journal write.
- **O4, V4 owner and cost:** qualify clean-start and pressured-host retirement and ordinary write backpressure, the signed quiet-control path, UART low-power sleep edges and failed quiet exit on the actual image. Show listening, transmit, control quiet and sleep distinctly, preserve the signed status counter and journal during reads, and measure costs and ordinary-frame loss at the declared load.
- **O5, Signalman acceptance:** verify scrolled accessibility targets, clipboard paste and physical cancellation; exercise two boards with complementary listening, transmit, refusal, reboot and overflow, export/reload identical intervals and totals, and disabled durable capture with live display. A finite serial collection does not synchronize independent board clocks. Join GPX at the consumer boundary with a stated error bound; never present predictions as measurements.
- **O6, two-listener acceptance:** record exact assignments/profiles, unavailable and transmit intervals, actual captures/misses and recovery after a listener disappears. Report missing observation time separately from coverage percentages. Coordinate scheduling with the listener-executive owner.

## Constraints and deferred work

Only the board owner reports physical transitions. Boot/sequence/uptime are device coordinates; host civil-time mapping and USB association are host evidence, not authentication. Gaps, disconnects and unmatched intervals remain uncertainty. RAM recording must not wait for host or flash. Hardware entropy and remote-carrier authentication remain integration gates; entropy failure visibly disables observation rather than creating a common zero identity. Persistent device history, non-USB carriers and packet-body capture are deferred, each requiring its own storage, authentication or disclosure decision. The sole assembled T114 is not a disassembly fixture; electrical tests needing power-path access wait for an accessible suitable board. The prior user-assisted volatile-history power-cut leg was completed and need not be repeated for that receipt.

## Findings, 2026-11-02

`crates/radio-hand/src/observation.rs` and `observation/recorder.rs` define the owner events and bounded recorder; `apps/signalman/src/observation.rs` owns a host bundle, while `apps/signalman-desktop/src/main.rs` handles collection and import/export actions. O1/O2 are software-complete. Earlier T114 physical RX/TX, cursor-loss, pressure, power-cut-history reset, refusal and zero-flash checks do not measure detailed timing. V4's declared-load run required an application reboot. O5 has keyboard export/reload and finite V4/T114 collection, not fresh RF coverage or physical Stop acceptance.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.
