# murm-sync

Shared network sessions over injected p2panda endpoints. `JoinedSpace` owns
one LogSync session, its live stream and the receive drain in shutdown order.
`SyncedSpace` reports actual received operations and reconciliation rounds.
`MunimentAddressBook` stores transport addresses over a caller's backend.

The host supplies endpoints and gossip through Murm's `P2pandaTransport`.
The domain supplies an operation store and an admission callback. Stickleback
in Eidetic retains validated history, causal processing, retention and group
encryption. `GroupKeySync` connects its receiver capability to this network
session without passing the lane's signing material to the receive task.

Moved from Eidetic's Stickleback networking feature at
`1014cf48e1882ff926b55f3509ac3dbf5a5aec8e`, preserving protocol ids and storage
formats. The canonical scope and qualification live in Murm's transport
consolidation plan, T7.

## Catalogue advertisement carrier

`advertisements::Advertisements` joins caller-selected gossip over the existing
endpoint. The caller supplies message and pending-queue limits; overflow is
counted and the oldest pending hint is dropped. Drop stops the receive task.
Payloads remain opaque: Gemot verifies catalogue identity, signatures, expiry,
sequence watermarks and current authority. Subject classes never determine
discovery topics. Public metadata advertising must be explicit; confidential
rendezvous requires an unpredictable topic and its own audience protection.

Qualified by the real two-endpoint gossip/budget test; this is a local transport
gate, separate from app UI or remote-machine acceptance.

Repeated hints use a fresh 16-byte delivery nonce inside a versioned bounded
carrier frame, so gossip deduplication does not suppress reannouncement after
late history or a new peer. The application signature, sequence and deadline
remain unchanged and authoritative. The real gossip test covers a second
delivery of identical application bytes.
