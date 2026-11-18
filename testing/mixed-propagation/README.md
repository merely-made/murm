# Mixed propagation fetch

From the Murm root on a Windows bench, use the [pinned stock RNS and LXMF interpreter](../../crates/retinue/oracle/README.md) and a separately installed Ratspeak lxmd-rs executable. Each case creates fresh profiles and submits a unique stock LXMF message over loopback TCP. The opposite backend fetches from that same live propagation node using the selected carrier.

| Propagation node → recipient | TCP | Raw Tulle | Paired Tulle |
|---|---|---|---|
| Outrider → Ratspeak | Matching persisted inbox | Matching inbox at link MTU 247 | Matching inbox at link MTU 500 |
| Ratspeak → Outrider | Verified fetch and acknowledgement | Ratspeak v1.5.0 can exceed the 255-byte frame limit during discovery | Verified fetch and acknowledgement |

## Run

Build the fixture and the three programs it drives. Set the two executable paths for your installation.

```powershell
$env:CARGO_TARGET_DIR = 'C:\t\cargo-targets\murm'
$env:CARGO_BUILD_BUILD_DIR = 'C:\t\cargo-targets\murm\build'
cargo build --locked -p signalman --bin signalman --example radio_bridge -p outrider --example stock_propagation_server --example stock_propagation_fetch_receive
cargo test --locked -p signalman --example radio_bridge

$python = './crates/retinue/oracle/.venv/Scripts/python.exe'
$lxmd = (Get-Command lxmd-rs.exe).Source
$bench = @('testing/mixed-propagation/run.py', '--bin-dir', "$env:CARGO_TARGET_DIR/debug", '--lxmd', $lxmd)
& $python @bench --output testing/receipts/mixed-tcp
& $python @bench --carrier tcp raw paired --relay-radio COM10 --recipient-radio COM7 --output testing/receipts/mixed-matrix
```

TCP is the default and runs both directions without hardware. Select one direction with `--node outrider` or `--node ratspeak`. Each invocation requires a fresh output directory. Radio cases use the bench's direct-PHY profile with DTR enabled on the relay side and disabled on the recipient side. Bandwidth and transmit power default to 250000 Hz and 2 dBm and can be set with `--bandwidth` and `--power`.

## Reading a run

Each JSON receipt names the node, recipient, carrier and outcome. A delivered case requires the deposited message ID and body at the recipient. Ratspeak's persisted container also identifies the intended destination. Outrider checks signature verification and acknowledgement. Outrider relay radio cases also check its accepted TCP and radio MTUs while the original relay process is alive.

Raw mode gives each protected packet one physical frame. Paired mode adds an 11-byte header on both radios and can split a protected packet across frames. The fixture records the largest protected packet offered to each bridge and fragment sizes when a packet exceeds capacity. Each carrier runs independently, with fresh processes and identities except for the fixed Outrider example keys.

The runner reports `known-raw-refusal` only for the Ratspeak relay's raw bridge rejecting a 258-byte packet against its 255-byte limit. The receipt distinguishes this outcome from delivery. A future successful raw fetch records `delivered`. Other failures return a nonzero suite exit code, and the remaining cases still run. Inspect both the exit code and the per-case outcomes. Fresh output directories preserve earlier attempts.

Receipt budget is 100 MB total. Keep the compact per-case JSON files. Temporary identities, passphrases, inboxes and full process logs are discarded after each worker exits.

The shared Rust fixture can also be run directly as `radio_bridge SERIAL_PORT TCP_PORT BW_HZ TX_DBM DTR raw|paired`. TCP port zero requests an available loopback port. Both bridge ends must use the same framing mode.
