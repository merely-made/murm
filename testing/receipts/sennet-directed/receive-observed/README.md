# Identified stock PKI receive, 2026-10-16

The scoped Djinn-backed Sennet receive-only probe authenticates and exactly
matches `sennet-pki-02ae107821df4d76bcf1cfe53af615d0` from the pinned Meshtastic
2.7.26.54e0d8d peer `f66afb28` to Sennet `472e0246`. No Sennet RF transmission
or ACK was generated; nonce state remains unused. The CLI completion is kept
separate from the independently observed Sennet receive.

The frame has channel discriminator 0 and an eight-byte CCM tag followed by
a four-byte extra nonce. Authentication succeeds with the 13-byte nonce
`LE32(packet_id) | extra_nonce(4) | LE32(source) | zero(1)`. The decrypted
application has the previously observed text envelope plus field 9 value zero.
The actual frame and decrypted envelope are in `stock-pki-capture.json`.
No private key or derived pairwise key is included.

The public stock contact URL contains both outer field 1's numeric node and
the nested User's textual ID. Earlier replacement of only text ID/key left the
outer address pointing at the stock peer; both SDK and QR readback refused the
new node. The [failed discriminator](../contact-address-mismatch/) preserves
that result. The corrected public contact updates both observed addresses,
public key and names, omits the copied physical MAC, and passes independent
SDK and QR identity/key readback before the stock send. This is controlled
black-box observation, not a generated-schema or implementation-source import.

Both board restorations/readbacks pass and the custodian stops. The raw report
records the current HEAD plus actual experimental source/binary hashes. This
qualifies one stock-to-Sennet PKI receive, not a directed desktop round trip,
routing ACK semantics, private-channel provisioning or 2.8 compatibility.
