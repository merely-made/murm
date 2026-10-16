# Stock configuration readback, 2026-10-16

The stock client initially returned success while independent CLI and SDK
readback retained region 0, frequency override 0 and power 30. Separate setting
groups and transaction commits did not resolve this. The selected failed
`setup-final-commit` receipt preserves that result.

The earlier successful bench used the public client's `--wait-to-disconnect 5`.
Restoring that connection lifetime, rather than sleeping after client exit,
passes `setup-client-drain`: region US, LongFast, 906.875 MHz, requested 1 dBm,
primary channel and the intentional public default key all read back correctly.
The SDK and separate CLI region query agree. This establishes the setup fix;
it does not establish Sennet RF delivery or attribute the original unclassified
CLI exit to the same cause.

Both setup-only trials leave custody inactive and packet IDs at `next=1`.
COM6 returns to its pinned Tulle image and exact saved status, region and channel.
COM7 undergoes only the same-boot baseline snapshot/reapplication. No erase or
factory reset was performed. Actual source hashes in each raw bench report
identify the experimental scripts beyond their common committed HEAD.

The wrapper also ends the first cold-stock read-only handshake when it observes
a reboot, waits five seconds and requires a fresh successful nonce/identity
handshake. This bounded setup retry changes no production actor refusal or
uncertain-send behavior.
