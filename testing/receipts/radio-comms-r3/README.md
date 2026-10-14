# Resident radio-to-Comms software qualification

The R3 adapters use the existing Sennet/Tucket resident runtimes and the real
Comms router. Tests carry native frames between two software runtimes, preserve
Meshtastic source/packet identity, verify MeshCore adverts and private text,
withhold an ACK to produce a real retry, match the delayed ACK, and reopen the
public journal without duplicate text. Refusals cover scope, subject, UTF-8
length, stale work completion, failed intent persistence and exclusive storage
ownership. The desktop test queues both protocols from its Messages state into
the same owner journal and reads them through the combined model.

`qualification.json` records the input hashes, source suppliers, compiler gates
and 155 passing test executions (not 155 distinct cases). Physical/RF, stock
companion serial, headed/pixel and power-loss acceptance are separate. The
existing isolated-daemon desktop custody case remains ignored. No private key
or channel secret is stored in this journal.
