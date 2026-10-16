# Sennet over Tulle desktop continuation, 2026-10-16

R7 in the [family plan](../../../design_docs/2026-10-13_murm_family_move_plan.md#r7--sennet-channel-text-over-tulle-2026-10-16-in-progress)
qualifies public LongFast channel text and reconnect separately from directed
messages, protocol ACKs, private-channel provisioning and RF authentication.
This checkpoint does not yet qualify desktop or physical delivery.

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
not invent a protocol ACK. The later native gate must independently match
that returned body at the Sennet end. Public software logs are in `software/`.
