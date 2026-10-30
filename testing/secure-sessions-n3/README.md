# Native secure-session qualification

This isolated workspace runs one bounded admitted echo service over Murm's
existing iroh carrier with Noise, raw TCP with Noise, Reticulum reliable links
over a local TCP interface with Noise, and native iroh TLS without inner Noise.
Each case launches a separate native server process and
client process. The raw TCP adapter reports no authenticated carrier identity;
the Reticulum adapter retains its actual shared link and absent initiator
identity. Noise facts remain separate from those observations.
Reticulum already encrypts its links; this fixture's inner Noise adds the
application identity/profile binding. A native Reticulum profile without inner
Noise is not qualified by this workspace.

The fixed seed keys and grants are public fixtures. They must never be used
for personal data or a deployed service. The fixture authenticates a single
reader, accepts one 256-byte-bounded request frame and requires a reader
acknowledgement within a two-second whole-operation deadline. Setup uses N2's
three-second deadline, one pending setup, 16 KiB each direction, 4096-byte
frames, 16 frames and 1024 I/O operations. These are fixture limits rather
than product defaults.

Build with the repository's toolchain, then pass the actual binary path Cargo
prints for this workspace to the harness:

```sh
cargo build --locked --manifest-path testing/secure-sessions-n3/Cargo.toml
python3 testing/secure-sessions-n3/run.py \
  --binary testing/secure-sessions-n3/target/debug/secure-sessions-n3 \
  --output /tmp/murm-secure-sessions-n3
```

Workspace or user Cargo configuration may change the binary path. The harness
refuses an existing output directory, retains each process log/contact/result,
snapshots its fixture/transport sources and records their hashes plus the
binary hash, and writes `summary.json` even after a failure. The summary starts
with `overall_passed: false` and only flips after all expected cases finish.
An interrupted or failed case
does not become a passing receipt. Narrow controls are available through
`--carriers` and `--scenarios`.

The reconnect case performs an explicit request, replays its old signed hello
on a fresh secure session/stream, then makes a new explicit request. It checks
three different channel bindings, two distinct admitted epochs and exactly the two
intended handler invocations. The other cases exercise wrong server/client
keys, signed-hello mutation, a ciphertext mutation below Noise, peer
cancellation, stalled setup and a consumer that never acknowledges its reply.
Every outcome must release the pending setup lease, and negative admission
controls must reach no application handler.
The native mode obtains backend evidence from each concrete iroh stream and
uses the TLS exporter/stream binding directly. A wrong expected publisher can
be refused before QUIC's lazy application stream is disclosed. Native signed
hello tamper is tested; the below-Noise ciphertext mutator is explicitly
inapplicable to native iroh because its stream API is above TLS. N2's backend
test separately checks two streams on one TLS connection have distinct bindings.
The default matrix therefore contains 31 applicable process cases.

The client keeps its endpoint and completed stream/connection handles alive until the server has completed stream
shutdown and written its result. The bounded harness lifecycle wait is separate
from the service's application acknowledgement and avoids counting endpoint
teardown races as successful application completion.

This is local native process/network evidence. It does not establish physical
radio, firmware, browser, remote-host, relay failover, durable mutation or
group-key qualification. The slow-consumer case establishes a bounded service
lifetime through its reply/ack deadline; it does not measure a network's
bandwidth or an operating system's socket-buffer capacity.
