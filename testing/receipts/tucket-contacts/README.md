# Tucket persistent public contacts, 2026-10-17

The Signalman resident journal now saves learned full public peer keys in its
existing local-identity/realm scope. Attaching a freshly constructed runtime
restores them before new work. New contacts start without routes; deduplication,
clocks, pending sends and private custody material are not replayed. The
protocol still learns fresh paths through its normal wire behavior.

This is host-owned persistence for Tucket over Tulle. It does not add a contact
flash journal to standalone firmware. The bounded firmware memory gate remains
versioned separately at source `a7107a7266f`; restoration uses the node's
already reserved contact capacity and adds no retained node fields.

Version-1 journals without contacts remain readable. Saving contacts promotes
the resident book to version 2, which older version-1 writers refuse before
writing. Stock companion books remain version 1 because their firmware owns
contacts. Invalid, weak, duplicate or colliding public keys, incompatible scope,
and configured capacity overflow are refused before contact mutation. Key
validation checks encoding and wire-address uniqueness; the storage owner binds
previously learned or explicitly selected keys to the original scope. It is
not a fresh signature or trust assertion.

The existing atomic journal commit publishes a new contact snapshot only after
success. A save failure leaves the previous snapshot intact and faults the
resident session so further carriage stops. Repeated adverts with the same
keys do not rewrite the contact snapshot. Existing history and uncertain-send
restart rules are preserved.

Required receipts for this slice are full-key cold reload with acknowledged
text and exact history, refusal/atomic-save failure tests, target checks and
the native reconnect fixture with its explicit peer re-advert omitted. The
shared retained-input budget with the preceding memory continuation is 100 MB.
The qualification ledger records current results and their boundaries.

The [native trial](native/run.json) passes four tagged texts and four matching
protocol ACKs across two connections, with exact history before new sends and
after final disconnect. The initial stock advert remains; the explicit
reconnect advert is omitted. This is not a claim that no other advert could
exist anywhere on air. The separate cold-runtime unit test injects no fresh
peer advert at all. The final version-2 book contains one full peer key and
four distinct message records. All seven [captures](visual-review.json) were
visually inspected. Existing clipped fields/viewport and stale connecting
notice remain cosmetic findings.

Full software suites pass 83 Tucket, 75 Signalman and 168 radio-hand tests.
Rust formatting, strict core and desktop Clippy, desktop/contact-card builds,
both bare-target checks and the unsafe inventory pass. The retained LOC
report still fails three unrelated existing files: companion actor, desktop
main and V4 radio owner. The contact extraction brings the central radio
adapter and resident session below their ceiling.

The pinned MeshCore 1.17.1 application runs on SB1 under its unchanged factory
partition table, which has no SPIFFS entry. Live protocol/profile/contact
readback and both exchanges pass. This reference qualifies wire behavior,
not stock filesystem or reboot persistence. Fresh rollback inputs were read
before installing it and checked against ROM MD5. Restoration reproduces the
entire original 1 MiB prefix and protected 64 KiB tail, with exact named
firmware/status/settings readback. SB2's original same-boot profile is
acknowledged and its named firmware/settings remain exact. T114 is untouched
in this continuation. No manual reconnect was needed, and all owned desktop,
peer and custodian processes stopped.

The first preflight refused an obsolete supplier hash before hardware writes.
The current shared Tucket/Sennet custodian matches its published qualification;
it uses an isolated real Keeper wallet through the scoped Djinn route. Neither
installed personal wallets nor private seeds appear in this receipt. Private
recovery images and the stopped generated wallet remain ignored under
`validation/results/tucket-contacts`, owned by this receipt. Frozen attended
scripts in `source/` document that original private-input layout; they are not
portable unattended runners.

[Qualification](qualification.json), [exact tested sources](source-sha256.json)
and [public file hashes](files.json) bind the outcomes. The native run used the
listed source hashes on top of its recorded base revision; it was not a
clean-source build receipt. Existing shared Cargo caches were reused. No
isolated target, Cargo home or worktree was created.
