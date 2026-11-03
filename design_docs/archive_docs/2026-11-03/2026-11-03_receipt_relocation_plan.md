# Receipt relocation and executable inputs

**Status (2026-11-03): landed locally.** Finish Murm's receipt relocation and fixture/harness path repairs, then checkpoint before Woodshed. This is repository maintenance under the approved closure rulings. The public firmware receipt URLs were already pinned to a published revision. Protocol design forks and the acceptance boundaries in the thirteen adopted successors retain their owners.

## Scope and standing decisions

Immutable run outputs leave the source repository for `Code/testing/murm/`, retaining their original repository-relative paths. Copies come from Git blobs and are verified byte for byte. Existing external evidence is compared and preserved rather than overwritten. Inputs actually read by tests or executable harnesses belong under the owning `tests/fixtures/`; filenames containing “receipt” do not establish that a file is disposable output. Executed and registered campaign tools remain versioned source. Historical source snapshots remain part of their original evidence bundle.

Archive documents, original rulings, recorded JSON fields and commit-pinned public references retain their historical content. Live documentation points to the relocated evidence or its published immutable source. Specifications, wire references, schemas, licenses and existing protocol vector fixtures remain repository inputs. An old passing capture does not qualify the relocated executable against current hardware or peers.

The total newly retained receipt budget for this pass is **100 MB**, including the original output copies, compact relocation manifests and verification summaries. Full compiler captures and additional hardware runs are unnecessary. Builds reuse an existing approved target under `C:/t/cargo-targets/`; no isolated Cargo home or worktree is required while main is free of a file collision.

## Phases and done-conditions

1. **Inventory and secure originals.** Classify the tracked output groups, standalone run reports, data readers, executable campaigns and source snapshots. Preserve the original blobs externally with their Git object identity, size and SHA-256. Done when every removed original has a verified destination and every retained executable or input has an explicit reason. The saved metadata is evidence in `Code/testing/murm`, rather than a run log embedded here.

2. **Place executable inputs and tools.** Move the signed Linkboy artifacts, desktop loader snapshot and actual campaign data inputs into owner fixtures with byte-preserving Git attributes where necessary. Move the selected reusable tools under `testing/campaigns/`, keeping adjacent imports and child processes coherent. Repair manifest commands, discovery ownership and fingerprint selectors to name the executable sources and fixtures they actually use. Done when there are no new missing selectors, orphan assets, duplicate registrations or reads from the removed frozen output paths. Existing unrelated registry diagnostics stay identified, with their dates preserved.

3. **Repair writers and references.** Route generated results to ignored local output or a caller-selected external destination. Preserve explicit version checks, custody provenance, board identity and restoration guards. A new result must refuse to overwrite an existing cited output. Repair live document and source references; grandfathered archive bodies and recorded provenance remain unchanged. Carry still-open criteria from a relocated acceptance report into its existing live owner where necessary. Done when the source tree holds reusable code and inputs, all removed output references have a historical locator, and public evidence links retain a usable immutable destination.

4. **Verify and checkpoint.** Exercise the actual relocated fixture readers and safe pure harness checks; parse Python and PowerShell without opening radios. Verify the validation registry against its recorded baseline and test output preservation with an existing-file control. Compile or run the focused Rust fixture tests using the reusable target. Inspect the final diff, staged blobs and retained-byte total, commit only this work locally, and report any remaining qualification limitation. Stop before Woodshed; pushing and physical campaigns require their own instruction.

## Findings

- 2026-11-03: the signed-artifact test dynamically reads four files beneath the old Prns retirement output tree. A desktop semantic-input test separately reads a loader snapshot in `design_docs`.
- 2026-11-03: the registered Sennet/Tucket checks execute three scripts and parse one nested historical JSON. The registry uses `fnmatchcase`, whose wildcard also matches directory separators; its old JSON selector includes unrelated run outputs.
- 2026-11-03: source snapshots and active tools coexist beneath the old receipt tree. Some historical wrappers are fingerprinted without being the suite's executed command.
- 2026-11-03: registry verification already reports eleven expired exemptions and three orphan assets at the starting revision. Expiry dates are not renewed by this maintenance pass.

## Progress

- 2026-11-03: inspected current main and work ownership, classified source/fixture boundaries, and secured the original tracked output blobs outside the repository. Executable inputs and campaign tools now have versioned owners; reader, writer, registry and live citation repairs are complete.

- 2026-11-03: byte-preserving fixtures and relocated campaign sources landed with focused software verification and existing-output refusal. The registry retains only its identified pre-existing expiry/orphan debt. External evidence and verification records are indexed in the receipt pointer; this checkpoint ends before Woodshed.
