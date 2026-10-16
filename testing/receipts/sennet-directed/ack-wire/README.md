# Two-way ACK wire experiment, 2026-10-16

The scoped probe matches an authenticated stock PKI text, emits a channel
ACK correlated to that stock packet, and physically completes a separate
PKI text reply. The untouched stock CLI reports "Received an ACK." Sennet then
matches the peer's channel response code zero and request ID 2 for its reply.
IDs 1 and 2 are distinct and come from the persisted interval 1–256; durable
next state is 257. ACK and text airtimes are 477.184 ms and 845.824 ms.

This independently qualifies Sennet's observed ACK encoding and the strict
routing-response parser against identified Meshtastic 2.7.26.54e0d8d. ACKs are
channel-encrypted, unauthenticated correlation facts, not read receipts or 2.8
ACK proofs. The stock CLI completion and matched Sennet control frame remain
separate facts in their original reports. Full retained-runtime/native
directed/reconnect/history qualification remains open.

Both board restorations and readbacks pass; the custodian stops. `source/`
matches the raw source inventory byte-for-byte and corresponds to the code
checkpoint published as `0f2f4d8b020`. The raw bench opened before that commit
and retains its original HEAD plus working-source hashes. Private material is
excluded. Local attributes preserve all emitted receipt bytes.
