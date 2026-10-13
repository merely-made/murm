# Retained repair host core

An incremental, sans-I/O host library for exact opaque objects. A caller can
publish a new object after startup while older objects remain available for
RaptorQ repair. This is a design prototype, not a standards conformance gate or
a radio-ready transport. It does not decode or admit application operations.

The producer retains at most four nonempty originals, each at most 16 KiB,
with a 64 KiB **original payload** ceiling. RaptorQ encoders, matrices, prepared
frames and returned candidates require additional heap memory. The dynamic
collector reserves at most four objects/64 KiB of declared object lengths;
completed decoder output is retained until its manifest expires. The library
has no built-in fixture signing key: the owner supplies authority and namespace.

## Caller flow

1. Construct `Profile`, `Producer` and `Collector` with an explicitly selected
   namespace and signing authority.
2. Call `advance(now)` before commands at that monotonic logical tick. It
   releases expired objects and returns their metadata, plus any invalidated
   producer token. Other calls at a new time return `AdvanceRequired`.
3. `publish(now, generation, original, expires_at)` validates size, lifetime,
   duplicate identity and capacity before constructing an encoder. It never
   silently evicts. Identical duplicate publication returns the original
   metadata; changed bytes or expiry conflict. New generations increase
   strictly, including after expired generations leave the inventory.
4. `prepare(now, channel, requested_generation)` creates one outstanding frame.
   Only `settle(now, token, Accept)` advances the cursor and queued counters.
   `Defer` retains identical frame bytes/token/ESI. Preparation of another
   channel/object while a frame is outstanding returns `PendingFrame`.
5. Feed received signed frames to `Collector::ingest`. It authenticates a
   bounded borrowed envelope before parsing OTI or allocating a decoder.
   Manifest-created state needs no predeclared scenario or original bytes.
   Completion verifies exact length and BLAKE3 and returns the actual decoder
   vector. `recovered(generation)` borrows that retained vector.

The clock uses the existing authored v1 logical epoch, ending at tick 8192;
expiry must be within that epoch. `EpochExhausted` is explicit. An owner starts
a distinct epoch with a distinct namespace; this library supplies no restart
persistence or credential lifecycle. Logical ticks have no promised RF duration.

One opportunity is accepted per channel/tick. With no requested generation,
channel 0 repeats manifests at the configured cadence, fresh symbols follow
publication order, and the selected repair floor uses a retained-generation
round robin. A requested generation is the owner's explicit exact-inventory
selection: its manifest repeats on channel 0/cadence, then fresh and repair
symbols are offered. This selection can override automatic fairness.

The one outstanding preparation is a deliberate first-slice limit: a deferred
head blocks further preparation until accepted or expired. Callers should ask
only for channels whose bounded sink has capacity. They may fill several
physical queues serially, then let their existing owners transmit independently.
Queue acceptance is not physical transmission, remote recovery or domain
admission. Original signed application bytes remain opaque and unchanged.

## Profile and bounds

The host coding profile is T=128, Z=1, N=1, Al=4, one full symbol per packet,
ESI at most 4095, and one to four logical channels. The collector accepts at
most 512 distinct authenticated symbols per object and 4096 decoder calls per
epoch, rejects duplicate symbols before decoder work, and releases each decoder
on completion/expiry. Frame size is at most 512 bytes. Application and radio
owners must also bound authenticated request rate and sink/output queues.

`wire` shares the existing authored fixed-array CBOR/Ed25519 layout from
`testing/transport-conformance/src/simulation/wire.rs` at `f1f3a89`. Its public
wire-level OTI parser retains that broader bounded 64 KiB/16-block profile;
producer/collector independently enforce their narrower limits before coding.
Historical simulation inputs and receipts are untouched. This is not COSE,
FLUTE, NORM, native-drop framing or Reticulum wire compatibility.

A signed T=128 symbol for generation 1 is 275 bytes, exceeding Selvage's
255-byte physical radio limit. Smaller symbols or fragmentation, radio lease
integration, duty cycle, clock acquisition, MCU memory and power require later
profile choices and measurements. The existing radio-hand/Tulle owners retain
their activation, physical queue and airtime authority. No 1–2 Mbps claim is made.

## Use the host example

```sh
cargo run --manifest-path crates/retained-repair/Cargo.toml --locked --offline \
  --example streaming
```

The bounded stdin example uses a public throwaway outer key and logical clock.
Commands are `publish GENERATION EXPIRY HEX`, `tick COUNT`, `join TICK`,
`inventory` and `quit`. Input lines are bounded, and `tick` work is bounded.
For example, publish object 1, advance some ticks, publish object 2, then advance
again. It prints actual fast/four-channel and late/one-channel decoder results.
Its channels are host delivery opportunities; it starts no network or radios.

```text
publish 1 100 00010203ff
join 20
tick 8
publish 2 100 1020304050
tick 60
inventory
quit
```

Catch-up requires adequate received symbols before retention expires. Repair
floor, capacity refusal and expiry expose the tradeoff; they cannot guarantee
catch-up under sustained overload or missing manifest/clock acquisition.
