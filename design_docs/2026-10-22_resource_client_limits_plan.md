# Resource client receive limits

**Status (2026-10-22):** implemented; clean publication-tree gates pass,
consumer qualification in progress. Native receive allocation
and Micron page decoding only; wire format and serving authority are retained.

## Scope and ownership

`ResourceReceiveLimits` distinguishes the sum of encoded Resource segment
tokens from the decoded Resource envelope. The exact page-body cap remains
`FetchPageConfig::max_page_bytes`. Enforce each budget before its allocation,
decompression, segment append or owned body copy. Existing page helpers use
finite defaults; generic raw request APIs retain their existing semantics.
Explicit limits allow consumers to tighten each quantity separately.
Transfer silence timeout is separate from a caller's whole-operation deadline.

The supplier lane owns Retinue receive/helper source and tests. Turnstone owns
page and explicit Submit adoption, environment settings and graceful shutdown
inside its outer deadline. The coordination plan lives at
`smolweb/design_docs/implementation_strategy/2026-10-22_client_limits_plan.md`.
Required evidence is compact source/lock identities and focused native tests
under that orchestration's **100 MB total retained-byte cap**. Reuse the
existing Retinue target/build directory; create no isolated Cargo home.

## Phases and done-conditions

1. Add explicit receive budgets and early admission. Done when oversized
   plaintext advertisements, understated sizes with excessive encoded tokens,
   and part-table geometry refuse before payload allocation.
2. Bound recovered and segmented data. Done when compressed and uncompressed
   payloads obey decoded limits, each segment uses the remaining budget, and
   overflow is checked before extending accumulated data.
3. Bound native page decoding. Done when packet and Resource exact-fit bodies
   succeed, excess bodies refuse before an owned copy, and existing MessagePack
   compatibility remains covered. Publish the supplier before consumer pins.

## Findings (2026-10-22)

- `nomadnet::fetch_page_with_config` called `request_raw` and copied the
  unpacked response body before checking its page cap.
- `request_raw_with_limit` already refused excessive advertised Resource data,
  but one cap had different packet/envelope semantics and did not bound the
  encoded token allocated from an understated advertisement.
- Compressed recovery was bounded; uncompressed recovery bypassed that limit.
  Segmented receive extended its accumulated body before checking overshoot.
- Resource silence deadlines reset on inbound traffic. Turnstone's outer
  deadline, which includes graceful endpoint shutdown, remains necessary.
  Payload buffer bounds do not constitute a total allocator/bz2 workspace cap.

## Progress

- 2026-10-22: completed read-only allocation/timeout audit. Merged the clean
  primary's I2P work with current upstream Signalman receiving changes using
  the normal merge strategy. Began separate encoded/decoded/body repair;
  physical radio and independent-reader qualification remain separate gates.
- 2026-10-22: supplier gates pass: 19 focused limit cases with and without
  compression, 46 Resource transfer cases, seven byte codec cases, eight native
  page cases, 25 endpoint Resource cases and the decompression bomb control.
  Counts overlap. Alloc-only check and strict production Clippy pass. Packet
  response decryption reuses the admitted sealed buffer and retains the link
  destination guard. Refusal followed by a fresh successful read is covered.
  Windows-only unavailable socket getter assertions now remain Unix-gated;
  supported assertions are preserved. Existing unrelated no-compression test
  import warning is recorded separately. Publication uses clean upstream to
  keep the primary checkout's foreign I2P work out of this supplier slice.
- 2026-10-22: clean upstream source `5f32366fc0e` passes all 19 focused limit
  cases with and without compression and strict production Clippy. Manifests
  and lock stay unchanged. The [native receipt](../support/receipts/client-limits/native.json)
  separates these exact-source gates from the broader preliminary primary
  matrix and preserves the existing no-compression import warning. No I2P
  source is introduced into the receiving cohort.
