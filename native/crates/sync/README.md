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
