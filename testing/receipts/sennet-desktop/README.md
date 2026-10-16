# Sennet over Tulle desktop qualification, 2026-10-16

R7 in the [family plan](../../../design_docs/2026-10-13_murm_family_move_plan.md#r7--sennet-channel-text-over-tulle-2026-10-16-functional-pass)
qualifies public LongFast channel text and reconnect separately from directed
messages, protocol ACKs, private-channel provisioning and RF authentication.
The [native reconnect gate](native-reconnect/README.md) passes four exact tagged
texts across two connections, with exact history, disjoint outgoing packet IDs,
zero protocol ACK claims, seven reviewed captures and verified restoration.

The core passes 146 radio-hand and 66 Signalman library tests, 19 runtime
integration tests, strict selected library/test Clippy and a bare runtime
check with `instances` enabled. A Sennet-only runtime constructs neither
Retinue nor Tucket identities. The packet-ID allocator persists exclusive
intervals before use, holds the file lock throughout the connection, abandons
unused IDs on reopen, and refuses missing/corrupt/wrong-scope/exhausted state.
A separate create-once commissioning fence prevents lost allocator data from
becoming an implicit same-identity reset. This is a local crash/reopen contract;
it does not qualify deliberate rollback of all files or hardware power loss.

The stock peer helper's opt-in `channel-peer` mode requires the exact tagged
receive, returns a channel broadcast and records device acceptance. It does
not invent a protocol ACK. The native gate independently matches that returned
body at the Sennet end. Public software logs are in `software/`.

The desktop passes 44 library tests (one existing ignored), strict all-target
Clippy, native/station-exporter builds, formatting and script parsing. The
actual Sennet worker refuses absent custody before serial, journal or nonce
initialization; public configuration rejects private material and private
channel selection. Both protocols retain their existing ownership exclusions.

The receiving cohort is canonical Mere `e12788bc`, Dramatis `19be4831`, Genet
`e37762d7` and Netrender `f4f9c0d8`. All 66 Signalman tests pass again against
the new Comms source; the channel peer builds and passes strict Clippy. The
initial Mere update moved Dramatis back to its older cached pin; reapplying
the qualified Dramatis pin and auditing the complete source inventory repaired
that resolution failure before this gate. No overlay or URL bridge is used.

The [producer ledger](custody-supplier.json) freezes six passing real-Keeper
cases; its canonical evidence lives in Mere's `testing/receipts/sennet-custody/`.
Both selected custody policy tests and strict port/example Clippy also pass,
published through Mere `b31363d2`. The temporary supplier integration worktree
and branch are removed. The existing shared Cargo cache remains co-owned.

The [first physical setup](setup-initial/README.md) failed in stock CLI
configuration before native launch; COM6 restoration passed and nonce state
stayed unused. The retry explicitly watchdog-resets the stock image after
flashing and captures filtered public diagnostics. The
[setup readback](setup-readback/README.md) now passes after retaining the
client connection for five seconds to drain writes. A subsequent
[USB transition refusal](native-usb-boundary/README.md) required a fresh
read-only handshake after configuration. The successful native gate uses those
setup checks without relaxing the production actor's reboot refusal.

Existing targets were reused. The shared `C:\t\cargo-homes\mere-upstreams`
cache remains co-owned by the family qualification lanes; the temporary Mere
worktree and branch are removed. Ignored synthetic fixture wallets remain with
helpers stopped after automatic approval review blocked recursive deletion.
They are excluded from these public receipts.
