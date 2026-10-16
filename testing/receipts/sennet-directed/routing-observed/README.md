# Stock routing response to a Sennet PKI reply

The `pki-reply-preflight` experiment first matches the exact authenticated
stock text, then sends one PKI reply with a caller-durable packet ID of 1.
Tulle reports 845.824 ms physical airtime. The identified stock 2.7.26 peer
responds with a public LongFast channel packet addressed to this Sennet node.
The independently decrypted application bytes are:

`08 05 12 02 18 00 35 01 00 00 00 48 00`

They retain the known application field 1/field 2 envelope: port value 5,
body `18 00`, fixed32 field 6 equal to the just-transmitted packet ID 1, and
the already observed trailing scalar field 9 value zero. The body's field 3
has value zero on this successful reply experiment. Header source is the
identified peer, destination is this station, discriminator is 8. This is
channel-encrypted correlation, not authenticated acknowledgment proof.

The probe labels the response an unclassified routing candidate rather than
promoting a delivery status. Its full frame, PKI reply and physical completion
are in `stock-pki-capture.json`. The next slice assigns these observed bytes a
strict response parser and proves that a Sennet-generated response is accepted
by the stock sender. Native directed/reconnect/history qualification is open.

The durable high-water mark advances from 1 to 257 before the first RF send;
unused IDs are abandoned. Both board restorations pass and the custodian stops.
`source/` preserves the four experimental codec/probe/helper sources byte-for-byte
against the raw bench's SHA-256 inventory. No private material is included.

An earlier reply trial refused startup with profile result 198 before any RF
send. It restored the saved same-boot profile with result zero; the cause is
unclassified. The passing trial adds an immediate same-boot profile readback
after contact setup. It does not relax the Rust pump's refusal or retry RF work.
