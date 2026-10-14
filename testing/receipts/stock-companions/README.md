# Signalman stock companion software qualification

**Status, 2026-10-14:** software qualified at
`b982319b6f203dffc81225a7232784c980c94ef8` on x86_64 macOS. The
[qualification](qualification.json) freezes source inputs, commands, compiler
versions and selected supplier identities. It records 159 passing test
executions: 58 Signalman library tests on each of Rust 1.99.0 and 1.97.1,
33 desktop library tests on 1.99.0, and 10 mounted companion/voice/accessibility
tests. One existing desktop custody bench test remains intentionally ignored.
Strict all-target Clippy passes for both packages on 1.99.0; desktop all-target
compilation also passes on the 1.97.1 MSRV. Historical optional-patch/Cargo
candidate warnings are present; selected Git manifests use immutable Cargo
checkouts and the graph has one Comms, Personae, LayoutDom and radio family.

The 15 companion actor regressions use independent peer framing over Tokio
duplex byte links. They check stock handshake/command fields, fresh config
nonces, enabled channels and device identity binding; device acceptance versus
matched remote ACKs; partial frames across timeout, receive deduplication and
startup packets; interruption/reopen without replay; journal refusal before
writes; unknown/ambiguous MeshCore prefixes; early MeshCore confirmations;
queue rejection, unrelated routing requests, malformed bounds and a 16-command
pending limit. Mounted production DOM tests activate the real connection and
queue buttons and observe the same journals. The invalid-settings actor test
refuses before serial I/O; tests open no physical radios or audio devices.

The new client follows Meshtastic's [public client API](https://meshtastic.org/docs/development/device/client-api/)
and [field contract](https://github.com/meshtastic/protobufs/blob/28f4f5f5c31c0323bfb86d890a1acaff15d1c4c6/meshtastic/mesh.proto),
and MeshCore's [1.17.1 companion protocol](https://github.com/meshcore-dev/MeshCore/blob/d92964352441e53b93e8667b802e04f6e072b39e/docs/companion_protocol.md)
and associated firmware semantics. Source inputs were consulted explicitly;
no upstream schema/source/generated client files were copied into shipping
code. Signalman's new client is separate from Sennet's prior independent RF
implementation and provenance record, which remain unchanged.

Connect/Disconnect and attended environment startup are documented in the
[canonical plan's R4 section](../../../design_docs/2026-10-13_murm_family_move_plan.md#r4--stock-companion-connections-2026-10-14-software-qualified).
Meshtastic carries text by node number (including broadcasts) on a selected
configured channel; MeshCore carries plain contact text by full public key.
The device retains private keys, configuration and radio execution. A changed
radio/channel-key environment requires a new caller-owned realm name.

This receipt does not qualify physical serial/RF interoperability, native
headed appearance, BLE, other-platform execution or power-loss behavior.
Destructive stock receive-queue reads cannot guarantee recovery after a host
journal failure. Device acceptance is not recipient delivery; protocol ACKs
are not authenticated/read receipts and Signalman does not verify Meshtastic
PKI ACK proofs. Firmware/distribution/native/wire suppliers and prior frozen
receipts are unchanged. GitHub Actions stays disabled.
