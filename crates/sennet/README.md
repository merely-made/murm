# sennet

A schema-free reader and writer for Google's public protobuf wire format, used by
Signalman's stock-companion client (`apps/signalman/src/radio_comms/companion/meshtastic.rs`)
to talk to a **stock Meshtastic-compatible radio** over its client stream. The radio
holds its keys and runs the mesh. The crate also names the radio's largest text
payload, `MAX_APPLICATION_PAYLOAD` (232 bytes).

`tests/structural_capture.rs` parses real captured device output with the reader;
the `capture/` scripts recorded those fixtures from a stock radio (see
`capture/STRUCTURE.md`).

Sennet is an independent implementation, developed from wire observation and
public documentation. It is not affiliated with or endorsed by any existing
mesh project.

A sennet is a ceremonial fanfare for a procession.

## What this crate no longer is

Until 2026-11-14 Sennet was a native participant: transport header and channel
AES-CTR, PKI direct messages, managed flood, routing ACKs, NodeInfo, a persisted
packet-ID allocator and a retained instance for the resident firmware image.
Mark ruled that out:

> considering the stack, tucket and sennet are not so important nowadays ...
> reticulum covers what the other two don't and what they do. the only value
> would be interoperation with existing non reticulum nodes, and for that, a
> thinner layer is possible.

and chose "Stock radios only": a stock radio holds the keys, we hold none and
compose no crypto. The removed code, its captured-frame tests and its receipts
remain in git history; [PROVENANCE.md](PROVENANCE.md) keeps the clean-room record
for all of it. See `design_docs/2026-11-02_protocol_parity_plan.md` (protocol_parity D1).

## License

Licensed under the Mozilla Public License, Version 2.0 ([LICENSE](LICENSE)).

This is a deliberate choice. Sennet is an independent implementation, built
clean-room (see [PROVENANCE.md](PROVENANCE.md)) — it contains no third-party
protocol code and needs no one's permission to exist. But a *permissive*
independent implementation would be an easy route around the copyleft the
protocol's own authors chose, letting anyone take the work without giving
improvements back. MPL-2.0 keeps that door shut: build whatever you like on top,
under any license, but improvements to *this implementation* stay published.

MPL is GPL-compatible. The firmware images this project ships combine it with
Reticulum-licensed `retinue`, so the images themselves are not GPLv3.
