# tucket

MeshCore companion serial frame types, for a host talking to a **stock MeshCore
radio** that holds its own keys and runs the mesh.

Today that is one type, `companion::DeviceInfo`: the public companion
`DEVICE_INFO` response, pinned to MeshCore 1.17.1's
`docs/companion_protocol.md` at `d92964352441e53b93e8667b802e04f6e072b39e`. It
keeps the firmware release separate from the serial API version and never
retains the BLE PIN. Signalman's stock-companion client
(`apps/signalman/src/radio_comms/companion.rs`) is its user.

## What this crate no longer is

Until 2026-11-14 Tucket was a native MeshCore participant: identity, adverts,
private and group text, the ECB cipher with its 2-byte MAC, routing, retries and
a retained instance for the resident firmware image. Mark ruled that out:

> considering the stack, tucket and sennet are not so important nowadays ...
> reticulum covers what the other two don't and what they do. the only value
> would be interoperation with existing non reticulum nodes, and for that, a
> thinner layer is possible.

and chose "Stock radios only": a stock MeshCore radio holds the keys, we hold
none and compose no crypto. The removed code and its receipts remain in git
history. See `design_docs/2026-11-02_protocol_parity_plan.md` (protocol_parity D1).

## License

MPL-2.0 (see `LICENSE`). `NOTICE` carries the MIT notice of the MeshCore project.
