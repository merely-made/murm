# Signalman management surface: remaining work

**Status (2026-11-02): plan.** This succeeds the [original management plan](archive_docs/2026-11-02/2026-08-15_signalman_management_surface_implementation_plan.md). S0/S1, the bounded S2 model and live-station leg, S3 headless shell, and later software rungs remain as recorded there; none implies the open headed, physical or two-site gates below.

## Phases and done-conditions

1. **S2 headed Network judgement.** In a real window, the owner judges pan, zoom, dragging, focus and selection, with the live station and real projection where required by the original. Done when the interaction and accessibility evidence names its exact window, source and station boundary. The joined headless mixed-realization receipt is not this judgement.
2. **S4 text on the station.** The [family move successor](2026-11-02_murm_family_move_plan.md) records that the later R1 slice already routes persisted text intents through the attended station and Comms; qualify headed serial-radio LXMF text and preserve the later scoped behavior. Done when restart replay, deduplication, explicit saved contacts and unknown senders, and receipt-specific queued/handed/accepted/fetched/failed states remain truthful across the real path. Propagation acceptance needs application-level storage acknowledgement.
3. **S5 voice.** Connect voice capture/send and propagation fetch to the live worker as needed for this specific path; the later R1 text-send slice does not itself prove voice carriage. Done when a headed two-site run records through a real microphone at A, carries and fetches through the propagation node at B, and audibly plays through the selected output, with codec, sample rate, duration, sizes, mode and cancellation result. File-backed and injected audio controls are bounded software evidence.
4. **S6 placement and S7 telemetry.** Store persona-scoped owner placements with provenance, timestamp, optional accuracy and retention; project them into the shared identity/selection model. Done when an owner pin survives restart, changing it changes only placement, unplaced nodes remain explicit, Network and Map share selection, the geographic score round-trips, and offline headed pins and ranges are shown. Capture the pinned stock telemetry subset, preserve unknown fields, reject malformed input and respect owner source preference; a named GPS board/firmware, field bytes and actual coordinate/age receipt are separately required. Saved surveys and placement ranking belong to the [survey plan](2026-09-06_signalman_survey_and_placement_plan.md); radio availability belongs to the [observation successor](2026-11-02_radio_observation_plan.md).
5. **S8 Browse.** Prove injected gemtext composition, focus, scrolling, links and accessibility in the real window before fetching. Then use the owning smolweb address/security gates for one Retinue fetch with visible source posture and equivalent document facts; no bytes bypass the host's authority. Micron remains its own engine and round-trip lane.
6. **S9 Graphshell guest.** Serve the proven Network, then Map, as bounded facts and constrained intents. Done when loopback and authenticated remote rendering preserve identities, intent is accepted or refused by Signalman, reconnect resumes from a generation, and both consumers use extracted shared device material without exposing the radio or private authority.

## Decisions and constraints

Signalman owns user-facing device reconciliation, contact/retention choices and visible facts. The station and radio owners retain hardware and lease authority; Linkboy retains flashing plans. Owner settings govern stale age, history, layout and visibility. Unknown senders do not silently become contacts; handed-to-radio is not delivered. User placement outranks telemetry unless the owner chooses otherwise. Preserve the one-source type identity and the mixed-realization focus, pointer and accessibility model.

## Findings (2026-11-02)

`apps/signalman-desktop/src/network.rs:34,99` contains the paint builder and layout worker; `apps/signalman-desktop/src/station.rs:16,271` uses station custody. These source observations do not close the real-window interaction or two-site acceptance.

## Deferred and follow-on

Speaking-lease display waits for an exposed fact; propagation-peer edges wait for peering. Online tiles, live calls, fleet campaigns, remote flashing, public packaging of the private composition and Micron fetch before its engine/carriage gates remain deferred. Reconcile later existing implementations before commencing any item.

## Progress

- 2026-11-02: adopted the reviewed open-work successor; original archived whole. No new implementation or acceptance run.