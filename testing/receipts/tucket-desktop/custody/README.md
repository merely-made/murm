# Djinn-backed Tucket consumer, 2026-10-16

The owner selected Djinn custody before further RF trials. Personae derivation
is published at Dramatis `5c89001a`; Mere's custody API and receiving checks are
published through `6d1908f4`. The actual broker's positive/refusal/watch tests
and restored-fallback negative controls are in Mere's
`testing/receipts/tucket-custody`.

The desktop activates one device UUID/realm scope, watches custody for the
connection lifetime, and builds a Tucket-only runtime. Guards cover serial
activation, carriage ingress, dispatch and physical transmission. Public
configuration rejects private-material fields. An absent Djinn test stops
before serial or journal access, leaving its temporary directory empty.

The desktop and public contact-card exporter build. The custody checkpoint's 40 desktop library
tests and 63 core Signalman tests pass; one existing desktop test stays ignored.
Formatting and strict desktop all-target Clippy pass. The source inventory has one canonical source each for Mere, Genet,
Dramatis and Netrender. The legacy Comms source declaration was updated after
verifying its package source is byte-identical to the qualified Mere revision.

The physical/native gate remains open. Public-only configuration is retained
here; the live Djinn wallet, runtime and journals remain in ignored local
validation results. The guarded runner restores the saved DUT profile and
pinned Tulle peer image even when the test fails. Software checks do not imply
successful Tulle-to-stock RF delivery.

The [first Djinn native trial](native-initial/README.md) delivers both texts and
the desktop's outbound ACK, but fails waiting for the stock return ACK. Both
boards restore successfully. The native scenario now waits for the matching
peer receipt before teardown; its extra regression brings the current desktop
suite to 41 passed (one existing ignored), with build and strict all-target
Clippy passing. Its [physical retry](native-peer-gate/README.md) still misses
the return ACK while connected. A configurable 200 ms reply turnaround is
now software-qualified (300 core executions, 41 desktop tests, strict Clippy,
bare `no_std` checks); its physical trial remains separate.
