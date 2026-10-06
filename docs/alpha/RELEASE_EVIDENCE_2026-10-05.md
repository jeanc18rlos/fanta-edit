# Release evidence — 5 October 2026

This appendix preserves the dated checkpoints, including original failures and
later corrected attempts. Results apply only to their named builds and scope;
statements that a check was pending describe that checkpoint. Use the
[current capability coverage report](CAPABILITY_COVERAGE.md) for the latest
acceptance status and remaining release gates.

## Local checkpoint — 5 October

The follow-up candidate pins Fanta UI to
`2e5793fcbff9dd55b8d0db5a364e0b70f8747378` and fixes resolved bound-paint display,
paint-binding detachment, ineffective bound-alpha controls, timeline text entry,
Variables header containment, and the prototype detail close/delete confusion.
Its combined automated run passed **1,050 editor tests (1 ignored), 3 benchmark
CLI tests, 234 workspace tests, 9 UI tests, app/CLI build checks and the full
twelve-crate lint gate**. Captured source hashes stayed unchanged throughout the
run. Evidence is in
`target/release-verification/2026-10-05-variables-timeline-prototype/`.
Mounted tests include real timeline typing after shipped keymap reload and
prototype Add/Save/Close/reopen, explicit Remove/Undo, invalid-draft preservation,
valid-draft commit and read-only dismissal. The targeted native follow-up below
verifies the corrected controls; the older observations retain the original
failures and their build provenance.

Native acceptance used `0a0d3cd1c4c9cf825c13e8cc50f5bbc9c7bdaf8c`, binary
SHA-256 `2e839e194323fcadadd6b5ec99ca7f72b78ae7778ded1eae5ba7c7a783bfef2d`,
on the disposable four-node Variables/Motion/Prototype fixture. The bound fill
row and picker showed resolved Mode 2 blue (`2F80ED`) and the variable label;
bound opacity was read-only with a visible explanation and Detach control.
Detach removed only the first fill's binding and baked RGB 47/128/237, plus the
save timestamp. Undo restored the entire typed baseline except that timestamp.
The long Variables project title stayed within its header, and the sidebar
toggle collapsed and expanded the panel. In **Current time**, Command+A, `0.5`, Return
sought to 0.50 s, displayed Start at X = −96, and retained the selection.

The prototype detail **X** closed the detail while retaining its interaction
card and wire. Save left the typed snapshot entirely unchanged, including the
timestamp. A complete process stop and relaunch used the same binary hash,
reopened the scene and preserved the typed snapshot strictly. Evidence is in
`target/release-verification/native-bound-controls-20261005/`:
`native-verdict.json`, `provenance.json`, and the before, picker-detached,
picker-undo, prototype-close and restart typed snapshots. Explicit **Remove**
was not repeated natively on this build; its mounted Remove/Undo regression is
separate evidence. This pass does not cover every node, interaction or installed
release configuration.

A separate clipboard correction now remaps internal node targets in every
primary and secondary Navigate/OpenOverlay/ScrollTo action when pasting or
duplicating content. Both new regressions failed before the fix and passed
afterward; the full clipboard operation suite passed **15/15**. The cases check
subtree/page copies, unchanged external targets and source content, action order,
and one-transaction Undo/Redo. Evidence is in
`target/release-verification/clipboard-prototype-references-20261005/`.
The subsequent component-aware correction also remaps copied component and
complete variant-set references in instances, swap overrides and primary or
secondary UpdateVariant actions. Partial sets and references outside the copy
remain unchanged. Its two new regressions failed before the fix and passed
afterward; the combined clipboard suite passed **17/17**, including action order
and single-transaction Undo/Redo. The final source passed **1,054 editor tests
(1 ignored)** and `./script/clippy --locked -p fig_viewer`, including the dependency
check. Logs, exact source hashes and `combined-root-verification.json` are in
`target/release-verification/component-prototype-copy-references-20261005/`.
These are typed-reference/operation tests, not native prototype playback proof;
arbitrary string-valued component properties are outside the remapping scope.
Both clipboard corrections postdate the native binary above. The later bounded
native keyboard acceptance on `9ca127c1be` is recorded below.

Canvas, keyboard and toolbar Duplicate now use the same component-aware copy
path as Layers menu Duplicate. A mounted Command+D regression reproduced the
missing component definitions before the fix and passed afterward across five
deterministic seeds. Its Command+C/Command+V control retains ordinary Paste's
shared definitions. The test checks complete variant sets, local and external
instances, primary and secondary variant actions, and exact typed document
content through Undo/Redo, excluding only the edit timestamp and separately
asserted selection/history. The combined editor suite passed **1,055 tests
(1 ignored)** and `./script/clippy --locked -p fig_viewer` passed. Evidence is in
`target/release-verification/canvas-duplicate-components-20261005/`.
This correction also postdates the native binary above.

Native keyboard clipboard acceptance passed on `9ca127c1be1296d25a69a7102b8680337f3df909`
(binary SHA-256 `b5ce6e5f9990f8a3ada90b8486cd9932f5b6f984c6197ac81f188d4cdf578183`).
The asset-free fixture starts with 21 nodes, three component definitions and one
complete two-member set. Command+D copied the 15-node subtree, producing 36
nodes, five definitions and two sets; ordinary Command+C/Command+V produced 36
nodes while retaining the original three definitions and one set. Typed saved
readback verifies internal node/component targets, ordered primary/secondary
actions, unchanged external controls and all other persisted fields against the
explicit expected copy. Save/Undo/Save restored the baseline and Redo/Save
restored each copy, with only `modified_at` excluded and reported separately.
The saved **Paste** state then survived clean process quit/reopen and another
Save with complete persisted equality, including the timestamp. All nine
comparison reports pass. `native-clipboard-20261005/native-acceptance.json`
retains snapshot/report hashes, independently rechecked binary and startup-log
identity, and scope below `target/release-verification/`. Duplicate restart,
prototype playback, Cut, page/menu Duplicate and asset-bearing component/subtree
copies remain separate checks; the bounded Spectrum image results are recorded
below and do not establish broader release readiness.

New mounted asset-bearing checks expose **three product failures and one passing
control**. Command+X deletes a directly locked Bitmap, Command+D duplicates it
despite the disabled canvas-menu policy, and Command+X on a component master
leaves its definition pointing at deleted content while an instance survives.
The actual canvas-menu Duplicate→Cut control passes for a PNG Bitmap plus
image-filled Vector subtree, retaining all original nodes, payloads, shared
asset IDs/bytes and Undo/Redo. These guards are being corrected; no green or
native acceptance is claimed yet. The meaningful red result is
`target/release-verification/asset-clipboard-guards-20261005/red-2.log`;
`first-red.log` is an earlier test-harness compile failure, not a product result.
Additional mounted regressions in `set-reference-red.log` reproduce an empty
surviving set instance after Cut or Delete removes the last variant. The
strengthened `variable-appearance-red.log` then catches opacity changing from
0.4 to 1.0 during detachment. Corrections and Cut→Paste component-dependency
coverage are in progress. Asset-bearing Save/reopen and remaining native routes
are still required.

The large Spectrum native journey on `0a0d3cd1c4` is **failed/open**. At 100%
zoom, the image-filled master at page index 9 / traversal index 2110 moved from
(17548, 1105) to (17580, 1121), retaining its visible image and 40 × 40 size.
Save remained dirty for several minutes. A process sample confirmed an active
save worker opening component sources and cloning their reference context;
component revision changes had selected every artifact. Repeated Save requests
were queued during diagnosis, so this is not a controlled single-save timing.
The disposable QA process was stopped after capturing evidence. The original
`.fig` was never edited. The post-stop snapshot is byte-for-byte identical to
the current-reader baseline: all 73,925 nodes, 41 assets and 12 Boolean bakes
remain intact (`aborted-save-disk-comparison.json`). Evidence, typed snapshots
and target identities are in
`target/release-verification/native-spectrum-20261005/`. A corrected build must
repeat edit/Save/Undo/Save/restart; the later image result below records that
bounded retry without overwriting this earlier failure.

The component-save invalidation fix now treats revision-only definition changes
as edits to those masters, unioned with scene-delta owners. Nested master edits
include every bumped definition. Name/schema/set/registry changes, structural
edits and unavailable deltas retain the conservative all-artifact fallback.
The regression-only run reproduced four failures; all **6 focused tests pass**
with the fix, including a 128-master fixture, actual Save/readback that opens
only the edited source, preserved unrelated FNX comments, and rejected external
edits to unopened FNX and component headers. The broader run passed **1,061
editor tests (1 ignored), 3 benchmark CLI tests, 98 FNX tests and 212 format
tests with source-order preservation**. `./script/clippy --locked -p fig_viewer`
also passed after a test-only local-variable cleanup. Logs, source hashes and
`validation.json` are in
`target/release-verification/master-save-invalidation-20261005/`. These checks
validate artifact selection and conflict handling; the later bounded native image
journey is recorded below, while broader large-page acceptance remains open.

Opened source artifacts now reuse an immutable reference table when the actual
component IDs/names and variable IDs/paths are unchanged. Component loads borrow
the full library and retain only their own definition. **5 focused regressions
pass**, covering shared ownership, nested named component/variable references,
public registry changes without generation bumps, duplicate-name ambiguity and
recovery, restored source vocabulary, clean reload, dirty identity preservation,
conflicts and asset-index guards. The baseline reproduced three sharing failures;
two existing-behavior controls already passed. The broader checks passed **217
format tests, 98 FNX tests with source-order preservation, 1,061 editor tests
(1 ignored), 3 benchmark CLI tests**, and the format/editor repository lint gate.
Production hashes stayed unchanged through validation; final focused tests and
lint passed after removing a redundant test-only clone. Evidence and retained
failure logs are in `target/release-verification/shared-reference-tables-20261005/`.
Each cache lookup still scans the vocabulary, scoped variable registries retain
their existing ownership, and separate parse/conflict paths can still build
tables. This proves bounded sharing and preservation, not measured native memory
or save latency; see the later bounded native image journey below.

The native retry on `9ca127c1be` also remains **failed/open**. The same image
master visibly moved by (32, 16), retaining its image and 40 × 40 size. A single
Save at **09:43:25.895 UTC** was rejected because external changes were still
being reconciled. A startup process sample showed eight concurrent watcher
reloads, seven waiting on the project read lock; this is not a controlled startup
benchmark. Undo restored the displayed (17548, 1105) position. Quit/Don't Save
left the app busy, so only the verified disposable QA process was terminated.
The post-stop snapshot is byte-for-byte identical to the baseline, and the exact
comparison reports **zero differences**, including the timestamp: **73,925 nodes,
23 roots, 9,012 component definitions, 41 unchanged asset manifests and 12 Boolean
bakes**. Native observations and binary identity are in
`native-copy-save-candidate-20261005/native-spectrum-retry.json`; the snapshot,
reader provenance, log and `after-rejected-reload-save-comparison.json` are in
`native-spectrum-20261005/`, both below `target/release-verification/`. This
confirms preservation after that rejected attempt. The later corrected image
journey below passes; this earlier failure remains retained.

The reload correction now serializes initial, watcher, merge and discard reads
through an asynchronous gate owned by each document's worker. Obsolete queued
reloads can be cancelled before parsing, and initial-load watcher reconciliation
waits for adoption of success or failure. Conservative full/asset refresh and
existing conflict/epoch checks remain. Four original regressions failed before
the fix; the final **5 focused tests passed across 5 scheduler seeds**, followed
by **1,066 editor tests (1 ignored)**, the viewer repository lint gate and
formatting checks. Production source stayed unchanged through final validation.
Evidence is in `target/release-verification/project-reload-coalescing-20261005/validation.json`.
The subsequent native retry completed Save as recorded below; these checks
establish neither startup nor Save latency.

On native candidate `eaec74bfbbf89932b822f5d5363f6fe01f4dd257` (binary SHA-256
`bf8e194f42b9e63db6c8eac40c695d4563de4dd7839f2eea734a6b073a87e878`), the single
Spectrum Save completed, but **content preservation still failed**. Moving image
2110 by (32, 16) unexpectedly reparented it from its mobile frame to the Avatar
ancestor, changing its sibling index and local coordinates. The strict saved
comparison also found **12,018 unrelated geometry changes across 6,552 nodes**:
3,378 on the visited Darkest Theme page and 3,174 on the visited Wireframes page.
The other 21 pages were unchanged. These changes affect 3,298 text nodes, 1,692
groups, 958 instances and 604 vectors; they include frame resizing and shifts
over 200 pixels, not just numeric rounding. Of these nodes, 6,532 have an
auto-layout frame in their ancestry or are auto-layout frames themselves; the
remaining 20 are auto-resizing text. Source inspection identifies page opening
and post-edit layout solving as writers to the saved document. An independent
solver replay has not yet established the cause of every individual value.

Undo/Save completed and the app quit cleanly. Readback restored the moved node's
parent, sibling order and full payload exactly, but retained **all 12,018 unrelated
geometry changes**. The target component revision rose from its omitted/default
zero to 3 after the move and 6 after Undo; the timestamp also changed. The strict
Undo comparison therefore still fails with **12,020 differences**. All 73,925
nodes, 23 roots, 9,012 definitions, 41 asset manifests and 12 Boolean bakes remain;
unchanged inventory does not establish unchanged content. No geometry, numeric
or revision exceptions were applied. Typed snapshots, complete differences,
source classification and `eaec74-native-save-undo-verdict.json` are retained in
`target/release-verification/native-spectrum-20261005/`. The several-minute Save
observations include other app work and are not controlled performance results.

The nested-drag correction reproduced three product failures with three controls
passing, then passed **6 focused pointer regressions**, **346 tool unit tests,
6 tool integration tests, 1 ink oracle test and 12 mounted viewer tests**, plus
the tools/viewer repository lint gate. The cases cover staying inside or partly
overlapping the current frame, overlapping less-specific containers, leaving
the frame completely, and legitimate sibling/deeper targets, including exact
scene Undo/Redo. Evidence is in
`target/release-verification/nested-drag-parent-20261005/validation.json`.
The corrected native image journey and instance flow-child move/Undo pass as
recorded below. Instance restart is inconclusive. Later source Undo, draft
recovery and small scoped-layout native passes are recorded with their separate
build identities below; they do not close the remaining dense-page gates.

The layout engine also stopped recording mutations when calculated geometry is
unchanged. A 2,051-node regression previously recorded 18,459 false mutations
over nine unchanged passes, exhausting the precise scene-change history; it now
records zero and retains a subsequent isolated transform change. Seven new
regressions and the strengthened variable-text fixture cover settled horizontal,
vertical and grid layouts, text sizing, unsized children and the existing vector
scale tolerance. The complete document suite passed 391 tests (1 ignored), the
renderer suite passed 341 (2 ignored), and repository lint/dependency checks
passed. Evidence is in `layout-noop-mutations-20261005/validation.json` under the
release-verification directory. This prevents false dirtying; it does not yet
prove native layout preservation on load or repair the recorded Spectrum failure.

The source reader now records runtime-only layout hints when authored FNX omits
required text/frame geometry or a flowing child's position, before defaults
erase that distinction. Cold project reads, scoped page/component reads and
incremental source replacement retain those hints. Explicit complete geometry,
including zero values and width/height sugar, does not request reflow. Three
baseline regressions failed; the first **4 focused tests**, **221 format unit
tests, 52 integration tests and 1 doctest** passed with the correction, with
1 integration test and 1 doctest ignored. The format repository lint gate also
passed. The subsequent focused run passed **5/5**, including intentionally
unsized page roots with complete children and an omitted flowing-child position.
Evidence is in
`target/release-verification/source-layout-omissions-20261005/validation.json`.
The fifth-test result is in
`loaded-layout-preservation-20261005/format-pending.log` under the same evidence
root.
A read-only check using the updated reader found **zero layout hints** in the
fresh canonical Spectrum project: 73,925 nodes, 23 pages, 9,012 component
definitions and 41 assets. This is a reader check, not native layout or latency
acceptance.

The viewer's **13 focused layout tests passed**, with five scheduler seeds for
the GPUI cases.
They cover preserved geometry on open/page visits, isolated edits and Undo/Redo,
paint-only edits, gesture cancellation, authored omissions, discard, external
merge and actual FNX Save/readback. Source-save cases retain the canvas lock
while computed geometry is written/refreshed, preserve an invalid second source
draft, and keep failed geometry writes visible and retryable. They also check
that discarding an invalid second draft retains the first saved draft's computed
geometry as dirty/saveable, and page visits refresh stale bounds without
changing geometry. The first full editor run found one component-registry
notification ownership regression (1,076 passed, 1 failed, 1 ignored); the fix
retains the existing assertion and the four streamed-update tests now pass.
The final combined run passed **1,079 editor tests (1 ignored), 3 benchmark CLI
tests, 320 source-order tests (98 FNX + 222 format)** and
`./script/clippy --locked -p fanta-doc -p fanta-format -p fig_viewer`. All twelve
implementation file hashes stayed unchanged through final validation. Logs,
the retained intermediate failures, `validation.json`,
`implementation-evidence.json` and `final-tested-source-hashes.json` are in
`target/release-verification/loaded-layout-preservation-20261005/`.

Derived layout now joins the authored history transaction and its journal
record, while cancelled edits invalidate stale amendment tokens and edits after
Undo preserve Redo until commit. Three guard regressions failed before the
correction; **19 focused history tests**, the complete **399-test document suite
(1 ignored)** and its repository lint gate passed. Evidence is in
`target/release-verification/derived-layout-history-20261005/validation.json`.
These document and GPUI checks alone do not establish native geometry
preservation or performance. The bounded native image follow-up is recorded below.

For the corrected native retry, `/tmp/fanta-spectrum-layout-acceptance-20261005`
started as a fresh reconstruction matching the immutable current-reader baseline
strictly, including timestamps and assets. Preserve both the pristine copy at
`/tmp/fanta-spectrum-pristine-20261005` and the earlier failure project at
`/tmp/fanta-spectrum-release-save-check-20261005`; the latter retains the
unintended geometry changes and must not become the next baseline.

The corrected native image journey **passes** on `a82f90f87b73b7d1b18aea22d0ab122b56e89b47`,
dev binary SHA-256
`a4ba02cf35f5146fb67cb69316d2d802638447d05b7258e52a78df0a39af915d`.
Page visits followed by Save changed no compared field, including the timestamp.
Moving the nested image-filled master by (32, 16) and saving changed only its
local translation from (48, 48) to (80, 64), the containing definition's omitted
revision (default zero) to 1, and the timestamp. Its world position changed from
(17548, 1105) to (17580, 1121), with its 40 × 40 size unchanged. Parent, order
and all unrelated content remained unchanged. One Undo/Save restored all node content
and asset manifests to the immutable baseline, with only exact revision 2 and
timestamp changes. Clean quit and same-binary restart matched that saved Undo
state strictly, including timestamp and snapshot SHA-256. All stages retained
73,925 nodes, 23 roots, 9,012 definitions, 41 assets and 12 Boolean bakes; the
comparison checked complete content, not just inventory.

The initial external action oracle falsely rejected the move and Undo because
it represented an absent revision as explicit JSON null. The original failed
verdicts remain retained. Version 2 preserves absent/present/null distinctions,
adds no geometry or numeric tolerance, passes **19 checker validation cases**
and an independent **19-case reproduction**, and still rejects the earlier bad
native Save. Build/protected-diff identity, exact comparisons, original failures,
corrected verdicts and independent review are linked by
`target/release-verification/native-layout-candidate-20261005/native-image-acceptance.json`.
This closes only this image journey; the moved state was saved/read back, while
restart specifically checked the saved Undo state.

The same candidate's instance flow-child move/Save and one Undo/Save also
**pass**, using the post-image-restart snapshot as their immutable baseline.
For `ChevronDown` (`01M45ENCQVRX92SYSQRNDEBZ36`), release changed only local Y
from 1.5 to 2, the containing definition's omitted/default-zero revision to 2,
and the timestamp. World position changed from (7712, 2264.5) to (7712, 2265),
with 20 × 20 size and the `Chevron` parent retained. This is the predicted
auto-layout result, not the horizontal pointer displacement. One Undo restored
every node payload, parent/order and asset manifest; only exact definition
revision 4 and timestamp differed. Instance restart is **inconclusive** and must
be repeated against the saved Undo state strictly, including timestamp. The
attempt launched the same candidate as PID 73223 at 11:59:32 UTC. An early Save
hit the reconciliation guard; after an
alert-acknowledgement error, refreshed UI showed an unexpected unsaved Shape
and changed scroll. Native UI was paused for clarification about concurrent
input. This attempt is inconclusive, with no post-restart preservation claim.
Native double-click reopened its Instance inspector with its original position,
size and clean state observed; its screenshots are retained in the native session.

The first instance readback wrapper omitted the required checker `check`
subcommand and exited before verification. The failed provenance/log remain;
rerunning the unchanged checker with the correct invocation passed on the same
move snapshot. Its **22 adversarial cases, 15 real-comparator fixture cases and
2 corrected-wrapper invocation checks** are harness validation, not extra native
coverage. `native-layout-acceptance.json` in the same candidate evidence directory
links the image and instance results, exact hashes and retained harness failures.

Two related source-review defects were reproduced and corrected in
[PR 52](https://github.com/jeanc18rlos/fanta-edit/pull/52), commit `e6362d8a1f`.
The baseline failed preservation of unchanged fixed nested geometry after an
outer spacing edit, and failed dirty/save protection for geometry computed by
a clean external FNX watcher reload. The corrected scoped solver retains fixed
islands whose inputs/allocation did not change, while a real nested allocation
change still reflows. External computed output remains dirty/saveable; the
source editor retains ownership of its separate locked, deferred save path.
**Three focused GPUI tests and 13 existing layout-preservation tests each passed
across five scheduler seeds; 73 engine layout tests and repository lint for
`fanta-doc`/`fig_viewer` passed.** The tests verify single Undo/Redo and actual
watcher edit→Save→cold reopen. Baseline failures, controls, commands and tested
hashes are in `target/release-verification/scoped-layout-followup-20261005/`.
The later `3e3f275fcd` native candidate now passes these bounded corrections.
On the eight-node fixed fixture, inspector Gap 8→16 and Save change only outer
spacing and sibling X 128→136; the imported nested child stays at (12, 7).
Single canvas Undo/Save restores baseline content; Redo/Save restores the edit.
On the allocation fixture, outer width 148→188 changes nested width 120→160,
its End-aligned child (12, 7)→(140, 0), and sibling X 128→168. Single Undo/Redo
and Save pass exact complete typed comparisons.

The external fixture's trigger changes only source spacing 8→16. The native
watcher updates the UI and the existing one-second canvas autosave persists
sibling X 136 before the manual Save checkpoint. The original
`external-source-only` oracle **fails and remains retained** because it expected
the earlier on-disk X 128. The trigger report and reconstructed source hash,
followed by FNX/sidecar mtimes about 1.85 seconds later, support this autosave
sequence. A separate completed-state comparison and subsequent explicit Save
both pass with only the intended spacing/position changes. The transient dirty
indicator was not observed; this is not a manual-Save-only assertion.

A separate **Fanta Cold QA** process (PID 97065, launched 13:37:32 UTC) uses the
identical `3e3f275fcd` binary SHA-256 shown above and a fresh profile. Its first
loads of external, growth and fixed fixtures retain the preceding saved
snapshots exactly, including timestamps. Only outer selection changed; no
content edit or Save was performed. The original Candidate process retained its
source-recovery failure window and was not restarted. Independent evidence in
`target/release-verification/native-scoped-layout-20261005/` includes
`native-journey.json`, `aggregate.json`, cold bundle/runtime provenance,
14 passing checkpoints and the unobserved transient checkpoint's strict failure.
These small fixtures do not establish whole-Spectrum visual fidelity or timing.

One native instance Undo also coincided with an unexplained zoom change from
200% to 15%. Possible concurrent input prevents attributing that observation to
Undo. A mounted instance drag→Save→keyboard Undo/Redo→Save/reopen regression
retains the exact viewport, active page, parent and complete scene across five
scheduler seeds. No camera production change was made; this is not a native
reproduction or a blanket navigation pass. Evidence is in
`target/release-verification/native-viewport-undo-20261005/`; the regression is
committed separately in PR 52 as `6e97d01eed`.

The seven-node source-layout native fixture used the same `a82f90f87b` binary
in a separate Source QA bundle/profile. Editing only FNX spacing 8→16 and Save
persisted exactly that field and the second child's local X 28→36; the other row,
all sizes/order and document fields remained exact. **Source Undo failed:**
a single Command+Z followed by Save, and a retry after explicitly focusing the
FNX editor, retained spacing 16 and X 36. Manually restoring the source and Save
passed the strict baseline comparison; it is not an Undo pass. Command+F also
failed to open embedded Find, so subsequent typing changed the source buffer;
that temporary edit was undone before the recorded spacing journey.

With a valid FNX edit and invalid JSON draft, Save retained the invalid JSON and
canvas lock. The valid FNX spacing was saved while computed child geometry
remained pending; there was no visible action to discard only the invalid draft.
Repairing JSON with its exact original metadata, reviewing the overwrite warning
and saving persisted the computed X 36 and passed the complete spacing-16
comparison. **This is repair, not Discard acceptance.** A final manual source
restoration and Save matched the full baseline strictly. Clean quit, same-binary
cold restart and an unedited Save also matched that baseline including timestamp.
`target/release-verification/native-source-layout-20261005/` retains
`native-journey.json`, bundle identity, the failed `native-source-undo-verdict.json`
and passing spacing, repair, final-restore and restart verdicts.

Those source corrections are now implemented and pass automated checks. The
embedded editor uses the existing Find bar and retargets it across FNX/JSON and
page changes. Source Save preserves authored Undo through canonical/computed
geometry updates and repeated Save. Ordinary saved-canvas source refresh remains
its own undoable text transaction, and a no-op refresh does not erase earlier
source history. **Discard draft** affects only the selected buffer, retaining
another dirty buffer and its lock; discarding invalid JSON keeps already
validated computed FNX geometry dirty/saveable.

The final editor run passed **1,089 library tests (1 ignored) and 3 benchmark
tests**, plus repository lint/dependency and formatting checks. The 29 focused
Code workspace cases passed across five scheduler seeds: one initial test-oracle
mismatch was corrected to compare against independently derived persisted
geometry, and its five-seed rerun passed without another production change.
Original failures, raw logs and final fingerprints are retained in
`target/release-verification/source-editor-find-undo-20261005/validation.json`.
The `3e3f275fcd` development candidate builds with identical before/after source
fingerprints; binary/bundle identity is in
`target/release-verification/native-source-recovery-candidate-20261005/provenance.json`.
The subsequent native `3e3f275fcd` retry passes Find in FNX (`spacing`, five
matches) and JSON (`order`, one match), with all 19 project files unchanged.
One source spacing edit followed by Save, repeat Save, one Undo/Save and
Redo/Save passes complete typed comparisons. The repeated Save is byte-identical.
The valid-FNX/invalid-JSON journey on `3e3f275fcd` exposed two recovery symptoms: after
Discard restores JSON and retains pending geometry, switching to Canvas and
pressing Save leaves the old child position on disk; focusing the canvas and
retrying Save shows a stale-file overwrite warning despite no external edit.
The failed comparison is retained in
`target/release-verification/native-candidate-source-layout-20261005/discard-saved/`.
This failure remains retained separately from the corrected-build pass below.

The own-write race now has a meaningful mounted reproduction: delayed FNX and
identity-sidecar notifications were treated as external changes while invalid
JSON retained the source lock. The correction records exact accepted writer
bytes and keeps the shared write lease through source-buffer refresh and canvas
adoption. Three mounted controls and one lease control pass across five
scheduler seeds; two format tests check exact FNX/sidecar/JSON hashes, including
unchanged noncanonical sidecars and later external changes. Actual external
FNX, sidecar and unrelated metadata changes still prompt, and Cancel preserves
disk bytes, typed content and assets. No focus production change was made: the
real Code→Canvas keyboard Save route passes as a control. Final validation passed
**1,095 editor library tests (1 ignored), 3 benchmark tests, 280 format tests
across its targets (3 ignored), 325 FNX/format source-order tests (1 ignored),
repository lint and formatting**. Evidence and exact source hashes are in
`target/release-verification/source-recovery-watchers-20261005/validation.json`.
The editor run also includes the two mounted mixed-opacity pointer tests; they
pass across five scheduler seeds, including Escape/outside release, but do not
replace native pointer-cancellation acceptance.

Native recovery now passes on `ac038333e3`, with the exact binary hash recorded
at the top of this report. A fresh seven-node, two-row, asset-free project first
matches its immutable baseline. Valid FNX spacing 8→16 plus an invalid JSON
draft saves only the FNX: disk spacing is 16 and the second child's X remains
28, while the draft and canvas lock remain. **Discard draft** on JSON, switch
to Canvas and Command+S then persist X 36 without a stale-file overwrite prompt.
The control row, parent/order and all other typed content remain unchanged.
Repeated Save preserves the entire saved snapshot and all 19 project files.

The clean quit exited PID 3491 before the same Recovery QA bundle relaunched
as PID 4820 at 13:57:24 UTC. It automatically reopened the correct project;
no Save followed launch. Readback is strictly identical to repeated Save,
including `modified_at`, and all 19 project files are byte-identical. The native
canvas showed both rows, the intended larger gap and clean state. Independent
full-field comparisons, the five passing checkpoints, runtime/build/source
fingerprints and original reports are retained in
`target/release-verification/native-recovery-source-layout-20261005/`
(`independent-review.json`, `native-journey.json`, `aggregate.json`).
Only snapshot `project_path` and reader-presence defaults are excluded; no
numeric, geometry or revision exceptions are allowed. This bounded pass does
not establish recovery Undo, forced native watcher-race handling, concurrent
external edits, multiple windows or performance. The failed `3e3f275fcd`
checkpoint has not been relabeled.

On a fresh four-node prototype copy, the same `a82f90f87b` Source QA build
**passed three persisted checks**: open matched the baseline exactly; explicit
Remove→Save removed only Start's click-navigation reaction plus the timestamp;
and one Undo→Save restored its exact reaction ID, action and every other content
field, with only the timestamp changed. The strict checker passed 29 synthetic
sensitivity cases, which are harness validation rather than extra native coverage.
The subsequent restart comparison **failed on three position values**. Those
changes were saved at 12:54:56 UTC during the interruption, after the successful
Undo checkpoint and before the 13:00 restart. The user subsequently confirmed
that the Start/End position changes were their edits. This is a **confirmed
user-edit baseline mismatch**, not evidence of an app regression. The original
strict FAIL remains valid against the older baseline and is not converted into
a pass. A later matching-baseline native pass is recorded separately below.
The confirmation, fixture and failed comparison remain preserved in
`target/release-verification/native-source-prototype-20261005/`, alongside
`native-open-verdict.json`, `native-remove-verdict.json`,
`native-remove-undo-verdict.json`, `native-restart-verdict.json`,
`user-position-edit-confirmation.json` and the updated journey/aggregate.

On `ac038333e3`, a fresh four-node copy includes those confirmed user position
edits. Native Command+S leaves all typed fields, including the timestamp, and
all 19 project files unchanged. Selecting Start shows its **On click → End**
card and wire. PID 4820 then exits before the same Recovery QA bundle relaunches
as PID 6244 at 14:04:49 UTC and automatically reopens the copied project, with
no Save after launch. The complete typed snapshot and 19-file manifest again
match the new baseline exactly. The original project was unchanged during the
copy and was not edited. The two passing checkpoints and independent equality
review are in
`target/release-verification/native-prototype-current-baseline-20261005/`.
This closes matching-baseline Save/restart preservation only; explicit Remove,
Undo and playback were not repeated on this build, and the original failed
comparison against the older baseline remains intact.

The Save acceptance correction in commit `b433e7d28f` reuses an aggregate
artifact hash only when its paired
per-file hashes and filename order match the freshly read bytes. Fresh disk
reads and source/header/sidecar/singleton conflict checks remain. The matched
512-page diagnostic with 8,744,448 bytes of retained source comments reduced
median acceptance time from **546.673 ms to 292.797 ms (46.4%)** over five
iterations each. This is one phase in the development profile, with native UI
and readback paused; it is not total native Save or release-build latency.
Baseline and fixed safety regressions passed; final validation passed **225
format unit tests, 52 integration tests, one doctest, 98 FNX plus 225 format
tests with source-order preservation, six master-save and ten save-generation
viewer tests, and repository lint**. Three format diagnostic/doctest cases are
ignored in the ordinary full suite. Source hashes, raw matched timings, conflict
retry/rejection checks and commands are in
`target/release-verification/save-acceptance-hashing-20261005/validation.json`.
The earlier native image Save completed about two minutes after the single
request, an observation bound rather than a standardized latency result. The `3e3f275fcd` candidate includes this optimization; complete native
Save/gesture performance acceptance remains required.

The later gesture instrumentation has **10 focused passes (9 pure and 1 mounted),
the mounted pointer-lifecycle case across five scheduler seeds, and 42 canvas
policy passes** in
`target/release-verification/native-gesture-metrics-20261005/focused-validation.json`.
These are source checks after the `ac038333e3` bundle was built; full combined
validation and native measurements are pending. The intended measurements are
completed Metal-worker scene walk/encode, synchronous flush wait and encompassing
render wall time. They are not displayed FPS, process CPU time, GPU timestamps,
OS input latency or post-release settling time. Atomic native drags may produce
too few completed frames for sustained-gesture percentiles. No native performance
pass or below-16-ms result is claimed.

The inspector/menu native follow-up used `85fc694b67c78f616401def73506e515594f52d7`,
binary SHA-256 `d42e94c0febfa6c701c257485a470779b5c991c7e3567b856f6e5b6fb7dced99`.
Build/source fingerprints and observations are in
`target/release-verification/native-inspector-followup-20261005/`. The matching
automated rerun passed 1,041 editor tests (1 ignored), 3 benchmark CLI tests,
234 workspace tests, 9 UI tests and the repository lint gate in
`2026-10-05-menu-export-retry/`. After the two fixes described below, the combined
working-tree rerun passed **1,044 editor tests (1 ignored), 3 benchmark CLI tests
and the full twelve-crate lint gate** in `2026-10-05-opacity-paste-final/`.
The subsequent native build verified both fixes as recorded below.

At the 320-logical-pixel inspector width, native checks now pass for context-menu
Escape (menu closes and selection stays), clicking from a menu into the X field,
and short, medium and long Export labels. The long label truncates within the
button padding and exports the correctly named 440×29 PNG. Typing X from 30 to 45
then pressing Escape restored 30. An invalid numeric draft showed its red border;
switching Core → Specialist pages retained the original X. In the text-fill
picker, replacing hex `232837` with `E34A6F` without Return, then pressing Escape,
restored `232837`. Save/readback after the numeric and color cancellations matched
every typed field and all asset bytes strictly. See `native-control-observations.json`,
`after-numeric-cancel-compare.json` and `after-color-cancel-compare.json`.
These are typed-draft cancellations; mouse-down → Escape → mouse-up scrub/color
drag cancellation remains unverified because the native automation drag is atomic.

Native bitmap copy/paste onto another page created exactly one new node with a
new ID and a 16×16 translation, preserving its payload, asset reference, all 34
original nodes and all asset bytes. Undo/Save restored the original document and
Redo/Save restored the pasted document, each except `metadata.modified_at`.
See `bitmap-cross-page-verdict.json` and its Undo/Redo comparisons in the same
directory. Asset-bearing component/subtree clipboard journeys remain separate.

The same build exposed two defects. A Node Graph + 3D opacity edit
correctly saved only their opacities as 0.5, and Undo restored both, but the mixed
inspector still displayed 100%. The aggregate-opacity correction passed its two
focused regressions and the combined suite. Pasting that bitmap into a separate
New Design correctly preserved the destination unchanged but gave no visible
refusal; the error appeared only in the log. The new notification and regression
also passed the combined suite. The original evidence includes
`after-mixed-opacity-verdict.json`, `after-mixed-opacity-undo-compare.json`,
`mixed-opacity-before.log`, `mixed-opacity-after.log`, and
`cross-document-before-fix-compare.json`.

Both corrections passed native acceptance on
`ee80ec48c642b42b704d674da2a87dc8c7f42a67`, binary SHA-256
`01d17bcfa2791ce3eab72cbf56779da39cbe2f2132d7ae0432962e6439215b8c`.
The rebuilt app reopened the saved 35-node bitmap-paste state with strict typed
equality and identical asset bytes. Deleting only that test paste through the UI
restored the 34-node baseline except `metadata.modified_at`. Editing the Node
Graph + 3D selection to 50% then saving changed exactly those two opacities;
the inspector still showed 50% after collapsing/reopening Appearance. One canvas
Undo restored the 100% display and the full typed baseline and asset bytes,
except the save timestamp. Pasting into the separate design now showed the
explicit **Paste failed: cross-document canvas paste is not supported yet**
notice, and Save/readback left the destination strictly unchanged. Cross-document
canvas paste remains unsupported; displaying its refusal does not add that
capability. Provenance, `bitmap-restart-compare.json`, `opacity-native-verdict.json`,
`opacity-saved-verdict.json`, `opacity-undo-compare.json`,
`cross-document-native-verdict.json` and `cross-document-after-fix-compare.json`
are in `target/release-verification/native-opacity-paste-20261005/`.

The same `ee80ec48c6` binary completed a representative Variables journey on
`/tmp/fanta-release-variables-prototype-20261005`. Native controls created a color
variable with Mode 1 pink (`E34A6F`) and Mode 2 blue (`2F80ED`) and bound a
rectangle's first fill. Switching the project to Mode 2 changed only
`active_modes` and the save timestamp; the canvas changed from pink to blue.
Unbinding removed exactly that binding and stored the resolved blue as the fill
literal. Undo/Save restored every typed persisted field, node, order and asset
except `metadata.modified_at`. Quit/relaunch restored the blue canvas with strict
typed snapshot equality, using the same exact binary SHA-256 recorded above.
Reader-default selection/history/viewport are excluded from persisted-content
claims. Evidence: `mode-unbind-verdict.json`, `unbind-undo-compare.json`,
`restart-verdict.json` and the before/after typed snapshots in
`target/release-verification/native-variables-prototype-20261005/`.

This earlier journey exposed two visible defects: the long project title
overlapped the collection header, and the bound Design fill row displayed its
gray authored fallback without indicating the binding while the canvas resolved
blue. Both corrections passed the `0a0d3cd1c4` native follow-up recorded above.
`observations.json` preserves the initial failures; its then-pending mode,
unbind/Undo and restart checks are superseded by the verdicts above. Aliases,
renaming/deletion and other variable types/bindings remain outside this sample.

The same project and `ee80ec48c6` binary also passed representative Motion and
Prototype journeys. Motion playback advanced to the two-second endpoint, ruler
seeking worked, and dragging one keyframe from 500 to 750 ms changed only its
time and `metadata.modified_at`. Undo/Save restored the complete typed document
except that timestamp. Prototype presentation followed the authored click from
Start (1/2) to End (2/2); Restart returned to Start, and Escape restored editor
selection and viewport. Saving with the interaction detail open preserved the
reaction. After restoring a subsequently deleted reaction with Undo and saving,
process restart preserved all typed project content except `modified_at`.
`prototype-save-open-detail-typed.json` and `prototype-motion-restart-typed.json`
confirm that comparison. The consolidated verdict is
`native-variables-prototype-20261005/motion-prototype-native-verdict.json`.

That earlier build exposed two further native defects: the timeline's
**Current time** field lost editing shortcuts after host keymap reload, and the
interaction detail's **X deleted the reaction instead of closing the detail**.
The latter reproduced twice, dirtied the document, and was reversible with Undo;
ordinary Save retained the reaction. The input and close corrections passed the
combined mounted tests and the `0a0d3cd1c4` native follow-up above. Native explicit
Remove/Undo on that corrected build remains unverified. An empty `flows.json`
after setting a starting point is expected:
`flow_start.json` stores that choice, while `flows.json` holds named flows.
These samples do not cover every motion property/easing, prototype trigger,
overlay or transition.

The Boolean/import candidate is `a4adca9570`, followed by the behavior-preserving
lint cleanup `0bec851a8a`. Its engine, importer and
source-order checks are in `2026-10-05-preservation-engine/`; the editor and
remaining assembled-app checks are in `2026-10-05-preservation-app/`, all under
`target/release-verification/`. That checkpoint recorded 1,764 engine
tests (6 ignored), 282 importer tests (11 ignored), 310 source-order tests,
1,040 editor tests (1 ignored), and 3 benchmark CLI tests, with zero failures.
Eight focused standalone-document tests additionally cover the final JSON
precision dependency. Source-file hashes are retained with the app run.
The same app run passed all 234 workspace tests, 9 UI tests and app/CLI
compilation. Existing component baselines also passed without regeneration:
animation panel 99.973%, timeline 99.989%, inspector 99.956% (minimum 99.95%).
The initial lint failure is retained; the result-conversion cleanup passed the
full repository lint gate in `2026-10-05-preservation-lint/`. That source
passed all eight default stages plus component visuals. The
final native rebuild is recorded separately; historical native results below
do not substitute for it.

The earlier isolated native QA bundle was built at
`6fd579b408860b0d09ec44d3260c4ae1a1b66d1d`, binary SHA-256
`226082debcbb88f462eab356e402b04fe1d9ae0a691d5f62ed5abb8dd9572d9a`.
Build and working-tree fingerprints are retained in
`native-preservation-20261005/provenance.json`. Live MCP passed **18/18 assertions
over stdio and 18/18 over the Unix socket** on this bundle, including exact
created-node persistence in FNX, the active file's Git status and valid PNG pixel
data. Logs, commands and screenshots are `mcp-stdio.log`, `mcp-socket.log`,
`mcp-metadata.json`, `mcp-stdio.png` and `mcp-socket.png` in that directory.
These passes establish protocol and persistence behavior, not native control
coverage. Native Boolean journeys below are separate from these MCP results.

Native Boolean acceptance on this same bundle passed on the isolated 53-node
project built from all 12 Spectrum Boolean subtrees. Recolor changed only the
selected fill; Flatten produced the exact preserved vector and removed only its
two operands; double-click selected the rectangle operand, whose one-step move
changed only its local X and invalidated only its parent’s baked appearance. Each
Undo and Save restored the full typed baseline except the save timestamp, with
all 12 bakes active. Quit/relaunch restored the page and all 19 non-Git files were
byte-identical; typed data exactly matched the pre-quit snapshot. Evidence:
`native-preservation-20261005/boolean-native-journey-provenance.json` and its
referenced snapshots/comparisons. This is complete acceptance for those sampled
Boolean journeys, not every Boolean operation or the entire Spectrum project.

The same native pass found that Escape did not dismiss the canvas context menu
and instead cleared the underlying selection. A mounted right-click/keyboard
regression reproduces that failure; the focus correction passed the subsequent
`85fc694b67` native check recorded above.

Native inspection on `6fd579b408` also caught an Export-label defect: the label
could collapse to an ellipsis despite the passing component visual baselines.
The upstream Fanta UI PR #8 correction was integrated and passed the subsequent
`85fc694b67` native assembled-inspector label/export check recorded above.
The earlier long-name overflow fix and component screenshot pass alone did not
close this defect.

The previous full local automated checkpoint is recorded in
`2026-10-05-acceptance-engine/` and `2026-10-05-acceptance-final/`
under `target/release-verification/`; the historical table below gives its exact counts.
Native checks then used the isolated QA bundle built at
`450efc87934aee3d1834b24bc13be37ad3fcc8d4`, SHA-256
`caa67385097a608c6a7db3b627ec3f86ba0802cfeaf879b68268a5e213e17edb`.
`native-acceptance-20261005/provenance.json` records the build and the separately
owned pointer-appearance working-tree change included in that binary.

The initial polished working tree passed all seven default verification stages.
The initial evidence is retained under
`target/release-verification/2026-10-05-release-polish/`, including command output
and environment records. Final engine evidence is in
`2026-10-05-engine-final/`; the earlier complete editor, benchmark-test and
app/CLI build checks are in `2026-10-05-pattern-guard-final/`, all under
`target/release-verification/`. The final Clippy rerun passed in
`2026-10-05-pattern-guard-lint/`. The combined run's earlier failed lint log is
retained: a redundant clone in a new test was removed before the successful
lint-only rerun.
The latest editor run includes Boolean context-menu targeting, Flatten appearance
and pattern-source guards, Undo, the mounted-pointer regression and the
export-label regression found during native QA. Four focused
export checks also wrote page/selection PNGs; their log is retained as
`native-qa-20261005/export-followup.log`. An earlier lint attempt caught a
benchmark example compilation error; the corrected example passed the final
lint and test reruns.

The historical editor and app/CLI checks below cover the production changes
from `55d420ae87`. The QA binary was then rebuilt at
`02f33b7f789bc313de689d4d28400d1cbb991894` and opened successfully through the
native app. On 5 October at 04:14 UTC, all 24 recorded project files matched
their pre-restart hashes, with none changed, missing or added; see
`target/release-verification/native-qa-20261005/final-restart-comparison.json`.
The binary SHA-256 is
`bc18712dcdc77a6bf8239f35eaf7d9a917f4c9a70c0347650e339c6902f72b2f`.
The detailed earlier native journeys below retain their `b05316fa0b` provenance;
they were not all repeated on this final binary. These are local checks, not
complete capability coverage or signed-distribution acceptance.

| Stage | Recorded result |
| --- | --- |
| Engine libraries, integration suites and doctests | Acceptance rerun: 1,754 passed, 0 failed, 6 ignored across 35 test targets; 53 seconds including compilation. Evidence: `2026-10-05-acceptance-engine/`. |
| Synthetic Figma importer | Import-report rerun: 269 passed, 0 failed, 11 ignored. The loader warning regression also passed 1/1. The explicit private Spectrum preservation gate failed: 12 flattened Boolean parents and 28 omitted operands. Evidence: `spectrum-import-20261005/spectrum-preservation-final-validation.json`. |
| FNX/format with preserved JSON order | Acceptance rerun: 310 passed, 0 failed; 24 seconds including compilation. Evidence: `2026-10-05-acceptance-engine/`. |
| Editor GPUI/unit suite | Acceptance rerun: 1,030 library/GPUI tests plus 3 benchmark CLI tests passed, 0 failed; 1 projection benchmark ignored in the ordinary suite and run separately. Library test execution 58.07 seconds. Evidence: `2026-10-05-acceptance-final/`. Includes eight per-node double-click cases, four virtual-text hit guards, both mounted inspector layout checks, context-menu entry activation/ordering/Undo, mounted blank-click input and Flatten appearance/pattern/undo checks. |
| UI primitives | 9 passed, 0 failed. |
| App and CLI compilation | Acceptance rerun: `cargo check --locked -p zed -p cli` passed in 11 seconds. Evidence: `2026-10-05-acceptance-final/`. |
| Repository Clippy gate | Acceptance `./script/clippy --locked` rerun passed for the runner's eleven Fanta crates, including all targets/features and denied warnings, in 37 seconds. Evidence: `2026-10-05-acceptance-final/`. |
| Native component screenshots | Passed against existing baselines: animation panel 99.973%, timeline 99.989%, inspector 99.956% pixel match (required 99.95%). Images and logs are in `target/release-verification/2026-10-05-native-components/`. No baseline was regenerated; these are component galleries, not full-app inspector screenshots. |
| Native debug build | Passed with isolated-profile log-path support. Final QA build provenance is recorded separately in `target/release-verification/native-qa-20261005/`. The disposable QA bundle is not a signed-distribution acceptance pass. |

The direct-executable QA bundle was exercised through native controls with an
isolated profile and a disposable component fixture. The vector context menu's
**Edit vector** entered path editing. Double-clicking instance text and replacing
`EDIT` with `QA` displayed `QA ME` only in that instance; the main component and
sibling stayed unchanged. Clicking away autosaved the override into `page.fnx`,
and native Command+Z restored `EDIT ME`. After quitting through the native app
menu and reopening the rebuilt `b05316fa0b` candidate, all 22 recorded project
files matched their pre-restart SHA-256 values: none changed, disappeared or were
added. The comparison is retained in
`target/release-verification/native-qa-20261005/restart-comparison.json`.
These are specific native journeys, not a completed per-node matrix. This pass
exposed a stale inspector label: after deselecting an instance, the Page header
could retain **Export Instance**. The fix passed its regression and a native
recheck in `b05316fa0b`: selecting the instance showed **Export Instance**, and
Escape restored **Page / Export Page** while preserving the existing PNG/1× row.
Fresh native repetitions of blank-canvas clicks also cleared selection,
including at the location of an earlier transient failure. The mounted GPUI
regression `empty_canvas_pointer_click_clears_instance_before_or_after_repaint`
then passed four actual pointer-input cases: Select and the return from vector
editing through Escape, each with and without repaint between press and release.
It checks deselection, pointer-state reset and unchanged document/history. No
production fix was made for the unreproduced transient; broader native pointer
capture, repeated select/deselect and drag-release acceptance remain open.

Native instance context-menu checks on `b05316fa0b` also showed distinct **Go to
main component**, **Detach instance** and **Reset all overrides** entries.
Activating Detach produced a Frame retaining `EDIT ME`, with the main component
and sibling intact; Undo restored the Instance. The first file comparison
against the earlier restart baseline differed in `doc/metadata.json` and
`pages/group/page.fnx`. Without the earlier file bytes, the reason for that
difference is unresolved; `detach-undo-comparison.json` retains the failed
comparison. A repeat using the complete 22-file `detach-before/` snapshot
restored exact project content except `metadata.modified_at`; no files were
added or missing. `detach-undo-repeat-comparison.json` records that difference,
and the snapshot confirms only the timestamp changed in metadata. These
artifacts are in `target/release-verification/native-qa-20261005/`. The repeat
supports content restoration for this journey, not identical hashes for all
files or an explanation of the first comparison.

Native inspector PNG export also passed for the selected instance (700×120)
and the full page (900×944). Both files were opened and visually inspected: the
instance retains its yellow background and `EDIT ME` text, while the page retains
all three banners, the additional rectangles and no review pins or selection
handles. Files and hashes are retained in `native-qa-20261005/exports/` and
`native-export-results.json`. The pointer-test follow-up also passed repository
Clippy for `fig_viewer`; its log is `native-qa-20261005/pointer-followup-lint.log`.

Live MCP against that same isolated app passed **18/18 assertions**, including
stdio initialization, tool inventory, editor/source reads, a rectangle mutation,
autosave and rendered screenshot response. The log is
`target/release-verification/native-qa-20261005/mcp-smoke.log`. The fixture's Git
repository was initialized by the app on editing, with files still untracked;
the smoke status assertion does not prove a tracked Git diff or sync. The PNG
response was checked in memory but not saved by this script. It does not replace
native menu/pointer checks; the separate native quit/reopen evidence is recorded
above.

The subsequent native Save As pass used the `02f33b7f789b` binary. Cancelling
the picker preserved all 24 original project/export files. Completing Save As
activated the copy; moving a rectangle then autosaved its new position only in
the destination. Native Quit/reopen preserved all 22 destination project files
and all 24 original files, and the copied canvas retained its component master,
both instances, shapes and comments. Loading both projects through the format
reader confirmed the same eight nodes, one page and one component; every node
and component definition matched except the intended rectangle translation.
Existing PNG exports were not copied into
the new project. Evidence is retained in
`target/release-verification/native-save-as-20261005/`.

That pass exposed a separate restoration defect: reopening the workspace
activated the original project, even though the copied design was active at
Quit. Both tabs and their content were available. Canvas tabs lacked persisted
restoration state, and asynchronous project auto-opening chose the active tab
by completion order. The fix at `b5fe804c0c` persists canvas navigation and prevents late background
opens from taking focus. Five regression tests passed across five deterministic
scheduler seeds, including a reproduced/fixed race between scoped-tab identities.
They release the original views before restoring saved content, verify clean page
navigation, and retain the close prompt for dirty canvas and actual dirty FNX
buffers. Logs, including the race failure, are in `canvas-restore-20261005/`.
Native Save As on `450efc8793` preserved every typed node, document field and
asset byte in the copy. Quit/relaunch restored that destination and its selected
third page. A subsequent copy-content comparison detected a 3D-node move during
concurrent window interaction; the original stayed unchanged. The failed
comparison is retained and is not an unchanged-content restart pass.
A fresh-copy repeat then passed: crop Apply/Save/Undo/Save restored all 34 nodes
and five assets except the modification timestamp. Quit/relaunch restored the
copy and selected media page, with strict typed equality and all 30 file hashes
unchanged. Both earlier projects also retained their 30 file hashes.
Evidence: `native-acceptance-20261005/clean-repeat-*-compare.json` and
`clean-repeat-all-project-restart-hashes.json`.
Two scoped page tabs after Save As and immediate Quit now pass all seven
canvas-restoration tests across five scheduler seeds. The queue fix flushes
queued/in-flight item state and orders regular batches so an older slow write
cannot overwrite a newer one. Eight workspace serialization and five Hot Exit
tests also passed across five seeds. These changes preserve existing dirty-state
and content-save behavior. The failure logs and successful reruns remain in
`canvas-restore-20261005/`, including `immediate-flush-before.log` and
`workspace-flush-order-before.log`.

Native New Design on `450efc8793` passed picker cancellation with all 30 files
of the previous project unchanged, project creation and text insertion/save.
It exposed a dark-canvas/light-inspector background mismatch and repeated local
image placement rejection while the native picker was open. The canvas background pixel regression passed for default, authored and
explicitly transparent pages without changing document data. The rebuilt native
app now places PNGs through the system picker and shows the matching light page
background. One Undo/Save restores all typed document data except the modification
timestamp; the imported PNG remains unreferenced on disk, so strict file equality
is explicitly not claimed. The same new project's invalid-source journey passed:
saving broken FNX showed a parse error and left all 19 files unchanged; the canvas
retained its text and blocked a drag. Native source Undo/Save cleared the lock,
and a subsequent drag saved its position. Canvas Undo/Save restored all typed
content except `metadata.modified_at`. Evidence is in
`native-acceptance-20261005/new-design-*.json`. Native inspector JPG, SVG and
PDF exports of the text also passed visual inspection at 87×19; a repeated PDF
export created `Text-2.pdf` and preserved all original export hashes. Evidence:
`native-acceptance-20261005/native-text-export-results.json`.

The later native build used `793aa5cb91` plus the resolver change committed as
`ed4efa03ee` and the unchanged, separate agent-presence working-tree edit. Its
binary SHA-256 is
`4ca4c465ae4d6b4b1ad198b34367b4d5984ef5aa0dbbac58dfbd2dba98ad0593`.
Source provenance and observed results are in
`native-acceptance-latest-20261005/`. Native two-page Save As preserved exact typed
document data and asset bytes; two successive Quit/relaunch journeys restored
both scoped tabs and the latest active tab/page. No fixed delay was inserted
between navigation and Quit, although native event-delivery latency was not
measured. All 30 source files, 30 destination files and 24 files in the separate
media project retained their hashes through both restarts. A long image name
exposed an Export-button overflow. The shared UI fix passed all hosted checks
and merged as Fanta UI PR #7 (`179643a0f4`); the editor pins its tested
`d4dd5163` revision. Native validation of the rebuilt editor is recorded separately.

The same `4ca4c465` binary passed all **18 live MCP assertions** using the
isolated profile, including a uniquely named rectangle, its exact node ID in
the autosaved FNX and a retained 240×160 PNG inspected as the expected red
rectangle. Evidence: `native-acceptance-latest-20261005/mcp-smoke-strict.log`,
`mcp-smoke-strict-metadata.json` and `mcp-screenshot-strict.png`. The disposable child project had a committed
baseline, although its parent project also contained untracked files; the
original whole-repository Git-status assertion alone was insufficient proof.
The first run's failures are retained: the harness compared prefixed MCP IDs
with unprefixed FNX IDs and rejected a valid compact PNG by byte length. The
harness now compares normalized IDs, checks the exact FNX Git path and validates
PNG chunks, decompressed scanline lengths and filter bytes. Its retained negative
checks reject unrelated Git changes and corrupt/incomplete screenshot pixels.
This is live protocol/persistence evidence, not a complete native UI pass.

A new forced-write-failure regression passed 1/1 and is committed at
`8b572a41c1`. It pauses a real save, makes newer edits and places an image, then
forces a write failure. The live edits, asset bytes, history and previous disk
content survive; a subsequent save reopens the latest content with a decodable
image, and Undo/Redo still works. This is GPUI/file-system harness evidence;
the log is `target/release-verification/save-failure-20261005/focused.log`.

Four additional mounted pointer/keyboard regressions passed and are committed
at `fadf63b2db`: bitmap crop apply with repeat-Enter protection and Undo/Redo;
crop cancellation during and after a drag; Group → Boolean → Vector drilling;
and locked/source-read-only Bitmap and Vector guards. Crop checks preserve the
original asset bytes, bitmap payload and world placement. These use actual
GPUI input dispatch in the harness, not native OS input. Their log and metadata
are in `target/release-verification/mounted-node-input-20261005/`.

The broader native node fixture on `02f33b7f789b` verified crop Apply/Cancel
and Undo, Group → child text word editing, Boolean → operand path-tool entry,
and video/audio/specialist selection and menu routing. It exposed a real save
failure: an existing 3D leaf caused `import not allowed: model3d` after an
unrelated crop edit. All 30 baseline files remained unchanged during the failed
save. The fix at `c676649fe5` preserves stored 3D payloads in both materialization
paths; two focused regressions passed, including an unrelated page edit, checked
save and reopen with model nodes in another page and a component. Evidence is in
`native-node-matrix-20261005/`; a native explicit Save on `450efc8793` subsequently persisted both a curved-text
edit and a cropped bitmap while retaining the stored 3D node and all five assets.
This closes the explicit-save reproduction; it does not independently establish
autosave timing.
The follow-up `66f478fd91` gives Audio/Node Graph/3D/AI/Embed their own titles
and supported dimensions/effects/export sections, plus specific context-menu
labels. Two focused wrapper tests passed, exercising all five kinds through real
property actions, exact Undo restoration and 120×80 PNG exports. All seven menu
tests passed, including activation of the specialist entries. Matching-layer
selection now keeps these node types distinct. Native rechecking on `450efc8793` confirmed all five inspector titles,
size/effects/export sections and their corresponding context-menu labels. Each
properties entry was activated. These are wrapper controls, not specialist
authoring engines.

That fixture also reproduced missed Text on Path clicks inside a letter counter.
The fix at `b808faeefc` adds the shaped-cluster footprint for authoring while
retaining exact-ink inspection. Two mounted-input regressions passed at 100% and
65% zoom, covering ink, counters, whitespace, Crop → Escape, editing and Undo,
plus occlusion, locks, ordinary/rounded clipping and distant empty baseline
space. All 27 existing canvas geometry tests passed. Evidence and source/binary
fingerprints are in `text-path-authoring-20261005/`. Native verification on `450efc8793` selected the curved `EDIT` word from its
letter counter, replaced it with `QA`, and saved `PATH QA ME`. Native Undo and
Save restored the original typed document and exact assets except the modified
timestamp. Evidence is in `native-acceptance-20261005/`. The first crop
apply/Undo save comparison in the same fixture also recorded three vector
viewport backfills performed by the existing load path; its strict comparison
remains explicitly unequal. No comparison exception was added for them; the
fresh-copy repeat above passes after that existing load normalization.

The subsequent native Motion-mode pass on `02f33b7f789b` played and sought
the local H.264 video: both the preview playhead and burned-in frame timestamp
advanced, and seeking displayed the expected later frame. Audio Play switched to
Pause with an advancing marker, and seeking updated its elapsed-time display;
audible output was not captured. `native-node-matrix-20261005/native-media-controls.json`
records these checks. Trim, complete animation timelines, specialist editors and
the final rebuilt binary are outside that pass.

On a synthetic CPU render of 256 shadowed rectangles, moving one rectangle for
120 frames at 1024×768 produced the following single before/after runs:

| Measure | Before | After |
| --- | ---: | ---: |
| Frame p50 | 267.509 ms | 266.492 ms |
| Frame p95 | 277.571 ms | 277.666 ms |
| Maximum frame | 289.838 ms | 306.803 ms |
| Layer-cache refill misses during motion | 15,360 | 0 |

The cache change eliminated refill work, but these measurements **do not
demonstrate a frame-time improvement**. Both cache and pattern focused suites
passed 10 tests each. Logs are retained in
`target/release-verification/2026-10-05-renderer-before-after/`. This was a synthetic CPU measurement.

The new read-only `drag_bench` also measured the actual Spectrum import on an
Apple M3 Max with 36 GiB RAM, in a **dev build**, at 1280×800 pixels. It mapped
73,896 nodes and decoded 41 images, with no malformed nodes or rejected images.
Page index 9 (Darkest Theme) contains 8,636 scene nodes. Each result uses three
warmup frames and 120 measured frames:

| Moving target | Zoom | CPU render p50 | p95 | Maximum |
| --- | --- | ---: | ---: | ---: |
| ChevronDown instance, node index 4358 | Fit all | 131.008 ms | 140.671 ms | 153.920 ms |
| Same instance | 100% | 8.377 ms | 9.033 ms | 9.941 ms |
| Image-filled frame, node index 2110 | Fit all | 134.719 ms | 139.783 ms | 148.317 ms |
| Same image-filled frame | 100% | 1.141 ms | 1.294 ms | 1.446 ms |

This is a current CPU baseline, **not a before/after improvement or native drag
latency result**. Fit-all remains expensive. No compilation ran during measured
frames; lightweight native QA ran concurrently. Renderer incomplete-artwork and
effect-failure flags stayed clear. The source bytes remained unchanged, SHA-256
`8130233d07c31d41bfb980ecd37aac91563d1c3377d6e32eee772910157ba3ab`.
JSON, fixture/build metadata and logs are retained in
`target/release-verification/spectrum-drag-20261005/`. The benchmark's three
tests and explicit-index replay passed. Release-profile, native GPUI/Metal
presentation, drag/drop/undo and the below-16-ms gesture target remain unverified.

A separate ignored `import_timing_diagnostic` passed 1/1 on that unchanged
Spectrum fixture at revision `49d14072a731`, in the test profile using dev
settings on the same M3 Max. Container/Kiwi decoding (`read_fig`) took **2.798 s**;
scene mapping (`fig_to_doc`) took **3.080 s**. It reported 73,896 mapped nodes,
0 skipped and **28 instance children dropped**, producing 73,897 scene nodes,
13,785 instances, 9,012 components and 41 assets. The diagnostic does not assert
complete imported-content fidelity. Its single timing baseline excludes the
filesystem read, image decoding, layout, persistence and native app opening;
it establishes no improvement. The fixture hash stayed unchanged at the SHA-256
recorded above. Command, build/revision metadata, binary hash and raw timings are
in `target/release-verification/spectrum-import-20261005/`.

A source-level audit subsequently classified those 28 removals: they are
**24 vector and 4 rounded-rectangle operands under 12 Boolean operations**,
not instance expansion copies. The parents retain baked paths, but their
editable operations and children are flattened away. The parents occur on
Wireframes and Application Frames & Grids, without instance/component ancestry.
A reduced projection reproduced 40 source nodes becoming 12 vectors, with 28
operands removed. This reproduced a structure-preservation defect without
establishing missing rendered artwork. Evidence and unchanged source
hash are retained in `spectrum-import-20261005/spectrum-preservation-classification.json`.
The corrected importer retains all 12 Boolean operations and 28 operands.
Its full synthetic library suite passed 282 tests (11 ignored), and the explicit
Spectrum structure gate reports zero flattened Booleans, lost operands, other
child losses or skipped nodes. All 12 isolated local subtrees render to
byte-identical PNGs at the same recorded viewport and 4× scale before and after
the change. This comparison excludes outer ancestor effects and is not a Figma
screenshot or full-page oracle. Evidence is in `boolean-preservation-20261005/`,
including `interop-final.log`, `pixel-parity.json` and `capture-after-run.json`.
Direct import → FNX → reopened typed scene → render also preserves all 12
stored geometry signatures and produces 12 byte-identical PNGs; root independently
compared their hashes with the original vector baseline. Evidence:
`direct-fnx-summary.json` and `root-independent-direct-fnx-parity.json` in the
same directory. Four operand translations change from about `1.69e-13` to zero
under the existing FNX precision policy, so typed payloads are not all bit-exact.
The signature now uses that same precision and stable key ordering. A failed
intermediate JSON harness run is retained separately: its decoder lacked the
`float_roundtrip` feature already used by the application's FNX/project codecs,
altering other coordinates by one floating-point unit. That failed run is not
counted as a successful persistence check.
The standalone document API had the same precision gap without feature
unification from those codecs. Its own `float_roundtrip` dependency and a
compact/pretty JSON regression now preserve exact operand values and active
baked geometry; all eight focused document Boolean tests pass. The reproduced
failure and successful rerun are `doc-json-roundtrip-before.log` and
`doc-json-roundtrip-after.log`.
Two assembled-inspector regressions pass for normal and stroke-outline Boolean
paints: the Fill/Stroke sections project their actual values, color edits render,
geometry edits invalidate the bake, and Undo restores node data and artwork.
Evidence: `inspector-paints-second.log`.
Unknown operations and unsupported or unavailable operands still use the
explicitly reported vector fallback. Native edit/restart acceptance on the
rebuilt candidate remains separate.
The initial report also had 504 unresolved nested override/derived paths out of
9,342. The first resolver fix (`ed4efa03ee`) recovered 376 paths by following
component-property swaps and inherited nested swaps in the correct master
context. The second (`6dd4f5192f`) applies component assignments nested inside
symbol overrides, recovering the remaining 128. The untouched Spectrum import
and an independent typed-swap checker now both resolve **9,342/9,342 paths**.
A separate check confirms all 64 affected placements emit the component selected
by the source assignment. Regressions cover selected component, text, derived
geometry, source order, precedence and rejection of foreign-component paths.
Evidence is in `spectrum-override-classification-20261005/`, including
`nested-assignment-selection-validation.json` and
`unmodified-report-nested-assignment.log`. The original fixture hash is unchanged.
These checks establish routing and component choices; they do not claim complete
rendered fidelity. Boolean preservation and rendered parity are being verified
separately, including expanded component instances.

The full Spectrum import → project write → typed readback diagnostic now
retains **73,925 nodes, 23 pages, 9,012 component definitions and all 41 assets**.
Ordered roots/children are unchanged, all asset bytes match, and the 12 Booleans
retain their 28 operands and active baked appearances. **Strict equality fails**:
15,278 values across 5,052 nodes change from nonzero magnitudes below `1e-9` to
zero under the existing FNX printer policy. The largest magnitude is
`9.417827628865894e-10`; affected fields are paths, transforms, derived geometry
and gradient starts. The raw report retains every difference, with **zero
unexpected or inventory differences** and no blanket precision waiver. The
comparison covers the entire persisted typed document and assets, excluding
only selection, history, viewport and active-page presence state. The source
fixture hash remains unchanged. Evidence:
`spectrum-save-reopen-20261005/attempt2/spectrum-save-reopen-summary.json` and
`spectrum-save-reopen-report.json`; the command deliberately exits 1 because
strict comparison fails. The first attempt ran out of disk space during the
transactional write, before readback, and its failure remains recorded in
`spectrum-save-reopen-attempts.json`. This is an offline format/import diagnostic,
not a native app save/restart, full-page visual oracle or drag-latency result.
Compilation ran concurrently, so its recorded durations are not performance gates.

The subsequent `55d420ae87` change protects direct and indirect pattern sources
from destructive Flatten operations. Its focused Flatten-filter run passed eight
tests, including typed paint-slot/override coverage and pixel preservation when
refusing a referenced source. Layer adapter tests passed seven cases; the bounded
benchmark remained ignored in that ordinary run and passed when explicitly run.
The complete editor, app/CLI build and final lint reruns passed. The subsequent
`02f33b7f789b` native rebuild opened successfully and preserved all 24 recorded
project files across restart, as recorded above; the earlier detailed native
journeys remain scoped to `b05316fa0b`.

Sharing the component-root lookup between layer rows reduced CPU projection work
on the same synthetic scene: **2,000 visible vectors and 2,000 component
definitions**, a dev-configured test build, three warmups and eleven measured
projections. The before state was the intermediate `49d14072a731` polish tree
with the new pattern-source guard, **not released main**.

| Layer-tree projection | Before shared lookup | After shared lookup |
| --- | ---: | ---: |
| Median | 283.149 ms | 5.835 ms |
| p95 | 296.739 ms | 6.084 ms |

Ordered action lists had the same SHA-256, with generated IDs excluded:
`595c2a69ad5d2905b413b630aea12a44f6f300ab4db46f8afdb119425cc2a1a2`.
This demonstrates a scoped layer-projection improvement, not native or Spectrum
drag latency. Raw before/after and focused test logs are retained in
`target/release-verification/layer-projection-20261005/`.

## Combined automated checkpoint — 938807f801

On 5 October, 15:15–15:20 UTC, the complete default
`script/verify-fanta-release` run passed all eight stages at
`938807f801ac249b5ec7418ef0473eb794a11413`. The retained working-source diff
SHA-256 `b7d3a89e6cdcfaf0dbba84ef973cc9c81283ea3b3309d8d6650d6a19ebe253ca`
was unchanged before/after; there were no untracked source files. This records
stable tested source, not a clean working tree. The driver elapsed time was
314.71 seconds and is a verification duration, not an application benchmark.

| Stage | Passing tests / result | Ignored or excluded |
| --- | --- | --- |
| Engine | 1,817 unit, integration and doc tests; includes 400 document, 353 renderer and 346 tool unit tests | 7 ignored: profiling/timing gates, network font fetch and one documentation example |
| Import | 282 library tests | 11 private real-`.fig` fixture/diagnostic checks |
| Source order | 325 library tests: 98 FNX + 227 format with `serde_json/preserve_order` | 1 acceptance-hashing benchmark |
| Editor | 1,131 library tests + 3 `drag_bench` CLI tests | 1 component-heavy layer-projection benchmark |
| Workspace | 234 tests | None ignored |
| UI | 9 tests | This is the local UI unit stage, not the optional native component screenshot stage |
| Build | App and CLI `cargo check --locked` | Does not build or verify a signed installer |
| Lint | Twelve-crate `script/clippy` gate and dependency check | No native acceptance implied |

All executed tests report zero failures. Source-order intentionally reruns
FNX/format library cases under a feature configuration, so summing stage totals
would not count unique tests. The optional `visual` and `import-preservation`
stages were not selected. Private fixture checks, the ignored timing/network
checks, authenticated live services, native UI journeys and signed-DMG
installation are not established by this run.

The full editor run includes the new asset-bearing clipboard lock guards,
master/final-variant deletion preservation, missing component/set restoration,
repeated independent Paste, post-delete dependency checks and visible atomic
refusals. It also includes source recovery, pointer-scrub and gesture-metrics
regressions. Renderer checks include the twelve opt-in split-render cases;
this does not prove an interactive cached-drag path or native latency. The
subsequent ancestor-splitting extension was external/uncompiled at this
checkpoint and is not part of this result. Original failing reproductions and
the narrower clipboard five-seed validation remain preserved separately.

Evidence: `target/release-verification/combined-938807-20261005/`
(`results.tsv`, `driver-provenance.json`, per-stage logs and independently
extracted `independent-review/counts.json`). Native clipboard journeys were
still in progress when this automated checkpoint was recorded; later results
must retain their own exact binary and saved-document evidence.

## Native clipboard guards and refusal checkpoint — 938807f801

The isolated **Fanta Preservation QA** development bundle was built from stable
`938807f801ac249b5ec7418ef0473eb794a11413` source with the retained protected
working-tree patch. Binary SHA-256 is
`a6684a7e927d27fdc52e18143cbb050712f721b5162fb8d229e46554f5970b8a`.
Bundle ID is `dev.fanta.preservation-qa`; its isolated profile is
`/tmp/fanta-preservation-qa-profile-20261005`. Native actions used the actual
Layers panel, keyboard shortcuts and canvas context menu.

- **PASS:** select the directly locked photo, send Duplicate, Cut, Delete and
  Save; repeat on the image child of a locked frame. Both checkpoints match
  every typed field, timestamp and all 39 project files byte-for-byte.
- **PASS:** select one member of a two-variant set and Cut. The visible notice
  explains that every variant must be selected because Paste cannot restore
  a partial set. Save leaves all typed fields and 33 project files unchanged.
- **PASS:** use the instance-specific canvas context menu's Detach command on
  an instance with variable-bound opacity 0.4 and main-component opacity 0.5.
  The visible notice explains that detachment would change appearance. The
  operation and Save leave the same complete document and 33 files unchanged.
- **PASS:** Delete that main component through the keyboard. The same
  appearance limitation is visibly reported, and the original document and
  files remain unchanged. Quit/relaunch of the exact bundle also preserves
  those files; PID 21152 exited and PID 29984 restored the correct project.

These checks do not directly assert the system clipboard contents. The
mounted tests separately cover clipboard preservation on refusal. Restart
compares persisted content and restored project identity, not an in-memory
snapshot. The instance context menu visibly includes Go to main component,
Detach instance and Reset all overrides.

**Failures retained:** in the asset fixture, Duplicate creates the expected
three nodes and preserves the image bytes, but Save adds `local_size` to two
existing vector shapes and the cloned vector. Undo restores the original
node count but retains the two added values. Redo equals the saved duplicate
state with the same IDs; that narrower result does not clear the first
failure. Deleting the three-node master preserves both images of its surviving
instance, but Save adds `local_size` to the detached vector. Undo restores the
master with that extra field. These are strict preservation failures.

Source inspection traces the additions to the unconditional
`backfill_vector_viewports` call in generic `FigDocument` construction, before
any clipboard command. Native authored shapes legitimately omit this field;
the conversion must be restricted to its supported import boundary. At this
checkpoint, a fix and new native acceptance were pending. The original fixtures
and failed verdicts remain unchanged. A further inspector issue was observed:
the bound-opacity instance displayed its raw saved 100% value although the binding
resolved to 40%. Both corrections and their later `100fe26921` native acceptance
are recorded below; the original failures are not reclassified.

Evidence: `target/release-verification/native-clipboard-candidate-20261005/`,
`native-asset-clipboard-20261005/` (including `root-native-actions-v1.json` and
all failed verdicts), and `native-clipboard-refusals-20261005/`
(`root-native-actions.json`, four strict native checkpoints). These bounded
results do not establish successful asset clipboard persistence, exhaustive
node acceptance, performance or signed-DMG readiness.

## Automated viewport and bound-value corrections — 1d03cb877b / a5ab8d436b

The original `938807f801` native failures above are unchanged. The viewport
correction in `1d03cb877bc80328687bedc399210261b5cd41ce` restricts missing
vector viewport inference to the Figma decode boundary. Generic native-document
construction preserves omitted `local_size` fields. Two real regressions failed
before the correction; all three final cases pass, including encoded Figma
import controls and actual Duplicate/Save/Undo/reopen with asset preservation.
The mounted case passed five scheduler seeds, and the document suite passed
91 tests. Test-harness construction failures are retained separately from the
two original product failures.

The inspector correction in `a5ab8d436b9174ba97f2a360b3d66ca52e7356eb`
resolves the three existing whole-node opacity, visibility and uniform-radius
bindings in their effective modes. A selected wrapper bound to 0.4 displays
40%; its main root's separate 0.5 opacity is not multiplied into that property.
Explicit corner arrays retain precedence, unsupported Group radius bindings use
the real fallback, and unresolved bindings remain detachable with a fallback
label. No literal, binding or document history is changed by projection. All
six new regressions failed before the correction and pass afterward, including
three mounted inspector tests across five scheduler seeds.

The combined final source passed **1,140 editor tests, one ignored projection
benchmark, three benchmark CLI tests, viewer lint/machete and formatting**.
The run was performed on `938807f801` plus the then-uncommitted viewport and
binding changes; those exact files were subsequently committed above. Their
hashes stayed unchanged during the run: `document.rs` SHA-256
`753a5bae165301012a2872aa97e5619dd57512d999005092ab0093ab69c461c7`,
`gpui_adapters/design.rs` SHA-256
`f1fbd9339ffe5638c184da4fa11415d39517e7f07b13d9ded2c8ed27dae1d226`.
The retained protected source file also stayed unchanged. This checkpoint did
not repeat all eight runner stages, visual/private-fixture checks, native
journeys or signed-artifact checks. Native reruns were pending at this automated
checkpoint; later `100fe26921` results are recorded below. The original failures
are not reclassified as passing.

Evidence: `target/release-verification/vector-viewport-load-20261005/validation.json`
and `bound-opacity-inspector-20261005/` (`validation.json`,
`final-validation.json`, original red logs and final editor/lint logs).

## Inspector success-label contrast correction

The component card's “Available” text inherited a dark button-fill color from
the UI defaults. The local theme bridge now seeds the complete success palette
from the active application theme. It preserves an authored success color that
already reaches 4.5:1 against the panel-field background and minimally adjusts
one that does not. Filled controls receive a matching foreground and readable
hover/pressed colors; other semantic palettes are unchanged.

Three focused Rust tests pass for the actual One Dark and One Light colors and
translucent inputs. The affected crate passes the repository lint and dependency
checks, and the changed file passes formatting. Computed status contrast is
5.10:1 in One Dark and at least 4.5:1 in One Light; these are palette calculations,
not native screenshot measurements. The later `100fe26921` native check observed
readable Available labels in One Dark and One Light; custom-theme and filled
success-button states remain outside the native check. Evidence: `target/release-verification/inspector-success-contrast-20261005/`
(`success-contrast.patch`, `review.json`, `validation.json`, test and lint logs).

## Native clipboard and bound-value acceptance — 100fe26921

The dev candidate `100fe269210ad4f7adea22be10e0199efd7237db`, binary SHA-256
`4d688bd5c27661a07baa4e2d456bc71bd13143264c7d1bf42c6f7f167c2d7fc3`,
passes all 41 declared checkpoints in the small asset-clipboard harness. The
original `938807f801` viewport-mutation failures remain retained; these are
separate corrected-build results.

| Journey | Completed checkpoints | Persisted result |
| --- | --- | --- |
| Image subtree Duplicate | Open, Duplicate/Save, Undo/Save, Redo/Save, restart (5) | Exactly three copied nodes, shared images, original content and omitted vector viewport preserved. Restart: 39 files exact. |
| Image subtree Copy/Paste | Open, Paste/Save, Undo/Save, Redo/Save, restart (5) | Expected three-node copy and offset; Undo restores baseline, Redo the same copy IDs. Restart: 39 files exact. |
| Image subtree Cut/Paste | Open, Cut/Save, Paste/Save, Undo-Paste/Save, Redo-Paste/Save, restart (6) | Cut removes only the subtree; Paste restores its payload and image references. Undo-Paste returns to the post-Cut state. Restart: 39 files exact. |
| Delete component main | Open, Delete/Save, Undo/Save, Redo/Save, restart (5) | Surviving placed identity materializes two fresh image children; removed definition disappears. Restart: 36 files exact. |
| Delete final variant | Open, Delete/Save, Undo/Save, Redo/Save, restart (5) | Surviving set instance retains resolved photo opacity 0.4 and images; removed definition and empty set registration disappear while the empty container remains. Restart: 35 files exact. |
| Cut/Paste component bundle | Open, Cut/Save, first and second Paste/Save, Undo-second-Paste/Save, Redo-second-Paste/Save, restart (7) | Each Paste from the original clipboard restores five fresh nodes and an independent fresh definition; previous pasted content remains exact. Restart: 42 files exact. |
| Direct and ancestor lock guards | Open, Cut/Duplicate/Delete for each locked case, restart (8) | All six keyboard actions preserve typed content, timestamp and all 39 file bytes. |

Every saved-state oracle compares all persisted fields and asset SHA/lengths.
Edits allow only separately validated, nondecreasing `modified_at`; copied fresh
identities are matched bijectively against the declared baseline-derived result.
Undo/Redo must recover the previously captured IDs. Initial opens, guards and
restarts also require exact timestamp and project-file bytes. Recorded process
exits and same-binary relaunches are 37930→39798 and 39798→47300. Root's native
records describe the expected image artwork and layer structures after reopening;
no post-restart Save was required for these checks. This harness does not prove
unrecorded clipboard-buffer contents or every unsupported dependency combination.

The 28-node binding fixture also passes native Open, Save, cold reopen and a
theme roundtrip with all typed fields, timestamp and 25 project files exact. Before restart, root
observed A01 wrapper opacity 40% separately from its 50% main, A03 hidden, B01
radius 18, B02 explicit-corner summary 7, and B03 summary 1 with TL1/TR2/BL4/BR3
controls. Ancestor-pinned controls show C01 opacity 70%, C02 visible and C03
radius 26. Available is readable in both One Dark and One Light. A separate
native theme roundtrip selected One Light, confirmed its chooser checkmark and
40% bound-opacity value, then restored One Dark with the same value. No document
edit or Save was performed; all 25 file bytes and typed fields stayed exact.
Cold reopen confirms artwork and persisted bytes; the detailed inspector
selections were observed before restart, with A01 repeated during the theme check. B02's four separate fields and the precedence label were not opened.
Optional mode-change/Undo, unresolved-binding controls, custom themes and filled
success-button native states remain unverified. Readability is a visual
observation, not a screenshot contrast-ratio measurement.

Evidence: `native-clipboard-capture-harness-20261005/ledger-final.json`,
`native-asset-clipboard-20261005/viewport-ledger-*`,
`native-whole-node-bindings-20261005/` (Open, Save, cold, theme-return and both
root action records),
and `native-viewport-candidate-20261005/`. Independent recomputation of all 41
clipboard and three binding verdicts, plus plan/operator/runtime/file-hash checks,
passes with no findings in `native-viewport-acceptance-review-20261005/all41/review.json`.
The separately recomputed fourth binding checkpoint is recorded in
`all41/theme-addendum.json`; the original 41+3 review remains unchanged.
This is bounded native acceptance, not exhaustive release readiness, a native
performance result or signed-distribution validation.

## Ordered ancestor-paint and resolved instance-text source checkpoints

Renderer commit `b1774b9ec4` extends the opt-in partition API to fixed clipped
ancestors. Background commands belong to Below; clips apply in every phase;
borders and inner shadows belong to Above after restoring the ancestor's own
child clip. Raster-phase composition is rejected for these specs before touching
the target. Ordered painting or Picture replay clears only Below and preserves
Middle/Above pixels and incoming canvas state. Normal rendering remains separate;
instances, masks, bindings, backdrop-dependent paints and composited ancestors
remain outside the eligible subset.

The full renderer library passed **362 tests, 2 ignored**, including all 21 split
cases, and renderer/viewer lint passed. The 240-case ancestor matrix retains its
2-LSB full-RGBA tolerance, reverse order, fractional/Retina scales, reflection,
image/gradient backgrounds, stale-spec, empty-phase, cache and unchanged-document
controls. The original separate-raster attempt failed 48 cases at low zoom
(maximum channel differences 3–6); those artifacts remain retained. No retained
native drag implementation, GPU/native timing or performance gain is claimed.
Evidence: `renderer-split-ancestors-20261005/integration/validation.json`,
`full-renderer.log`, `clippy.log` and `independent-review.json`.

Instance-text commit `a3a51b0` uses the renderer's variable context at the placed
instance. Component-property aliases, active/inherited modes, resolved virtual
visibility/opacity, typography and clips therefore agree with the displayed
text, while imported derived geometry and sparse override Undo remain intact.
The regression-only run had **three real failures and one passing fallback
control**; after correction, all **18 instance-text tests** pass. The full editor
library passed **1,147 tests, 1 ignored**, and viewer lint passed. This latest
run did not rerun the benchmark CLI or all eight release stages. Missing/cyclic
alias fallback and unchanged authored content are covered; recursive true nested
instances, virtual TextPath and a new policy for direct text-content bindings
are not added. Direct text bindings retain their existing precedence over sparse
overrides.

Evidence: `resolved-instance-text-20261005/integration/final-validation.json`
and its retained red, green, full-editor and lint logs. These two source changes
are newer than the native `100fe26921` binary above. Instance-text native
acceptance is recorded next; future native renderer integration remains pending.


## Native resolved instance text — a3a51b084b

The dev candidate `a3a51b084b094dc9cf0f3c78fdd5cf0bb7e59120`, binary SHA-256
`6f292ee25db180df73c9d2ac6d8937d3ddd6e32e4c40dc641329110e92420046`, passes
three bounded instance-text journeys. Its source fingerprints stayed unchanged
through the build, including the retained user-owned source diff. The disposable
fixture contains 24 nodes, four definitions, seven variables, two modes, no
assets and 34 project files. Its master roots pin Mode A while the placed and
derived rows pin Mode B, deliberately testing the placed context.

| Journey | Native observation and saved result |
| --- | --- |
| Default property alias | Double-click seeds Default A; replacing it with QA default changes only A01's exact text-content override. Single Undo/Save restores the baseline; Redo/Save restores the captured edited content. A02 stays Default A. |
| Placed property-value alias | Double-click seeds Placed B despite the master pin. QA placed, single Undo/Save and Redo/Save each match the expected complete document. Other text and the hidden control stay unchanged. |
| Derived text | Double-click seeds Derived B at the displayed derived position/baseline. QA derived, single Undo/Save and Redo/Save preserve its derived box, transform and typography, with C02 unchanged. |
| Hidden virtual text | Double-clicking B02's blank text location opens no inline editor or caret. Its properties remain available; Save preserves all typed content, timestamp and 34 file bytes exactly. |
| Quit/relaunch | PID 57120 exited before the same bundle/profile/binary relaunched as PID 61631. The final derived-Redo state reopened with QA derived on C01, Derived B on C02 and the other labels intact. No post-launch Save was performed; all typed fields, timestamp and 34 files match the saved state exactly. |

Each content edit permits only the declared single sparse text-content override
on its target instance and a present, nondecreasing `modified_at`. Masters,
siblings, component definitions/revisions, variable values, modes/pins, geometry,
styles, child order and empty asset manifests remain exact. No format or numeric
normalization exemption was used. Initial Open also preserves all 34 files.
Independent review recomputed all 14 stored checkpoints: initial Open, nine
Save/Undo/Redo states, two intermediate baseline restorations, hidden no-op and
cold restart. The frozen oracle's 37 sensitivity cases and an independent
32-case review remain distinct from native acceptance.

Evidence: `target/release-verification/native-resolved-instance-text-20261005/`
(`native-*`, `native-acceptance-review.json`, `independent-review.json`) and
`native-resolved-text-candidate-20261005/` (`provenance.json`, `runtime.json`,
`runtime-restart.json`). Root's capture records contain the native observations;
the snapshot helper supplies no input. Only the final derived-Redo state was
restarted. Native mode-changing actions, rich-text/IME input, true nested
instances, virtual TextPath and direct text-content binding edit/refusal remain
outside this fixture. This does not establish native rendering speed, all-node
coverage or signed-installation readiness.


## Native Bitmap Crop and TextPath transitions — a3a51b084b

The existing dev candidate `a3a51b084b094dc9cf0f3c78fdd5cf0bb7e59120`, binary
SHA-256 `6f292ee25db180df73c9d2ac6d8937d3ddd6e32e4c40dc641329110e92420046`,
was exercised on a fresh three-page node-matrix project with 34 nodes, five real
local assets and 30 project files. Frozen oracles compare every persisted field,
root/child order and asset hash; they allow no geometry, numeric, style or
reference exemptions beyond each declared edit and a present nondecreasing
`modified_at`. They supply readback evidence, while the operator records actual
input and visible results.

| Crop checkpoint | Observed result and exact scope |
| --- | --- |
| Cancel | Canvas **Crop image**, drawn preview, Escape/Return/Escape and Save preserve the original typed document, including timestamp. The operator's contemporaneous full-files assertion failed on automatically initialized root `.git` storage and reported the original 30 project files unchanged. No canceled filesystem manifest was archived, so the formal cancellation claim is strict typed equality only. |
| Failed entry retained | Batched double-click on an unselected bitmap followed immediately by drag left Select active and moved it from 70/110 to 310/230. Saved evidence fails with exactly these two unexpected transform changes plus timestamp. No crop was created. This is not relabeled as a passing crop or a fixed production defect. |
| Recovery | One Undo/Save restores the complete original document/assets except valid timestamp advance. The baseline was never replaced by the failed moved state. |
| Apply | Isolated double-click visibly enters Crop before a separate rectangle drag; Return/Save clips the grid. Exactly one fresh default Group named Crop is added at the original parent/index, with world rectangle approximately 110.67/141.24 to 351.29/261.22. Its exact values must satisfy the frozen gesture bounds. The original bitmap retains its ID/payload/world placement, reparents into Crop with the prescribed local translation, and all other content/assets remain exact. |
| Undo and Redo | Single Undo/Save restores the original 34-node state; single Redo/Save restores the actual captured Crop ID/rectangle and complete 35-node state, apart from timestamp. |
| Quit/relaunch | PID 61631 exited before PID 67292 launched the same binary/profile. The cropped grid reopened without another edit/Save. Typed data including timestamp and all 30 project files match the saved Redo; the complete 48-file inventory, including root `.git`, also happens to match. |

The crop oracle was frozen before Apply. Its 45 sensitivity cases and three
file-policy checks are synthetic validation, not extra native journeys.
Independent review recomputed all six captured stages, including the expected
failed entry, checked stage timestamp chronology and verified oracle, reader,
provenance and snapshot hashes. The earlier cancellation is qualified in
`initial-cancellation-operator-note.json`; no missing filesystem evidence was
reconstructed.

The TextPath journey uses the exact saved Crop cold snapshot as its immutable
baseline: 35 nodes, five assets and 30 project files. Actual canvas **Edit text**
selects `PATH EDIT ME`; Command+A and `QA PATH`, followed by Escape to commit and
Save, leave blue 32 px text on the same curve. Only the target content and one
explicit style run covering bytes 0–7 with the exact original base style change.
The curve, transform, parent/index, all other nodes including Crop, document
registries, child order and every asset stay exact. Single Undo/Save restores
`PATH EDIT ME`; single Redo/Save restores the captured edited state.

After PID 67292 exited, the same binary/profile relaunched as PID 68654 and
restored QA PATH and the cropped grid, without another edit/Save. All typed
fields, timestamp and 30 project files match the saved Redo exactly. The layout
reopened at 38% zoom after the left dock restored; viewport continuity was not
this oracle's acceptance target. Independent review recomputed all four
TextPath captures and their provenance/file checks. Its frozen oracle passed
35 sensitivity cases and four file-policy checks before input.

Evidence: `target/release-verification/native-crop-transitions-20261005/` and
`native-textpath-transitions-20261005/` (`native-*`, `runtime-restart.json`,
`exit.json`, `native-acceptance-review.json`, `oracle-ready.json`). Root `.git`
exclusion is an explicit project-byte policy; full inventories and their
comparisons remain retained separately. The initial batched entry failure
remains open as a reliability observation. Rotated/nested/mask crop, vector-mode
transitions, TextPath conversion, range styling, Unicode/IME, virtual TextPath,
broader per-node input and release-artifact installation remain unverified.


## Native Group, Boolean, media and specialist entry — a3a51b084b

The same `a3a51b084b` binary/profile, PID 68654, was checked against the frozen
TextPath cold snapshot: 35 nodes, five assets and 30 project files. These are
entry/navigation checks, with no intended authored edits.

| Checkpoint | Operator-observed native input and result |
| --- | --- |
| Group and selected vector | The Group background menu offered Ungroup and no Edit vector. One separate double-click drilled into its background child with Move active; another pair entered Edit Path. Selecting the child directly in Layers then right-clicking preserved that target, and the actual Edit vector entry opened Edit Path. Escape returned Move and cleared selection; a subsequent right-click therefore correctly targeted the Group. |
| Boolean and selected operand | The wrapper menu offered Boolean properties and opened its inspector. Double-click drilled into the left operand while Move remained active. Right-click retained that selected operand, and actual Edit vector entered Edit Path. Escape returned Move and cleared selection. |
| Video and Audio | Each actual properties menu entry retained the matching type header, position and size, Appearance, Effects and named Export sections without overlap. Hiding the inspector then separately double-clicking the media node revealed it. Design and Move remained active. No playback, trim, export or media edit was invoked. |
| Node Graph, 3D, AI and Embed | Each actual typed properties menu entry opened the matching inspector. Hiding the inspector, observing the expanded canvas, then isolated double-clicking the same selected artwork revealed it again. Design/Move, expected positions and 190 × 160 dimensions, Appearance, Effects and full named Export controls remained legible at 320 logical px. No specialized editor, export, playback, network generation or geometry edit was invoked. |

After each sequence, explicit Save and strict readback preserve every persisted
field including timestamp, root/child order and all five asset hashes. The 30
project-file bytes and complete 48-file inventories are also identical to the
frozen baseline. Independent review recomputed all four comparisons and checked
reader/runtime hashes and recorded operator observations. Evidence is
`native-textpath-transitions-20261005/native-group-vector-noop/`,
`native-boolean-vector-noop/` and
`native-node-entry-matrix-20261005/{media-primary-menus,specialist-primary-menus}/`;
the aggregate review is
`native-node-entry-matrix-20261005/completed-entry-review-v2.json`.

The native Group/Boolean entries showed the Edit Path toolbar but no initial
anchor squares. Source inspection found that context-aware ToolShell activation
initializes the node-edit target and then clears overlays, without asking the
tool for its current path overlays. The double-click route also skips the
following tool release event, so activation alone does not populate anchors.
A regression/fix candidate is being prepared; no corrected native result or
vector-point authoring pass is claimed here. Locked/Inspect, mixed-selection,
specialist content authoring and broader drawing remain separate acceptance work.


## Native Inspect/multi-selection and corrected initial anchors

Two further `a3a51b084b` checkpoints use the same immutable post-TextPath baseline.
In Dev/Inspect, selecting the Bitmap and Group background Vector in Layers before
isolated double-click retained their source inspectors without Crop or Edit
Path. The Vector context menu enabled Copy but disabled writes; clicking its
disabled Edit vector entry had no effect. TextPath double-click attempts left no
selection, so they establish no successful TextPath hit/Inspect-target claim.
An initial bitmap artwork click targeted its Crop wrapper; the exact Bitmap
check subsequently used Layers selection. Returning to Design and Save preserves
all typed data/timestamp/assets and all 30 project/48 total files.

The other checkpoint uses canvas Command+A to select nine roots. At 320 logical
px the inspector showed nine layers, mixed X/Y/W/H, arrangement, opacity and
Add auto layout. Right-clicking selected QA PATH retained all nine and showed
only generic selection commands, with no Edit text, Edit vector or Crop entry.
No mutation command was invoked; Escape and Save preserve the strict baseline.
This is one mixed-selection menu/layout state, not a full mixed-value gesture
or applicable-action matrix.

The missing initial anchors were corrected by initializing overlays from the
activated tool after its target is ready. The regression baseline had three
failures and one passing control. Final focused four tests pass with the three
GPUI cases repeated across five seeds; the full editor passes 1,157 tests with
one ignored, and viewer lint passes. Evidence is
`node-edit-activation-anchors-20261005/{final-validation.json,final.patch,red.log,green.log,full-viewer.log}`.

The new dev candidate `8a9a33c35e2e586a533ac0b991b37a1cb13e14c5`, binary
SHA-256 `8d24b09c414c650c26724b616ddf6a200913e4b6942966186dc79a78c7a35ecb`,
ran as PID 78080 in its isolated Editor Guards QA bundle/profile. On both the
Group Vector and Boolean left operand, one isolated pair drilled into the child
in Move and the next pair entered Edit Path with all four corner anchors already
visible. Their actual Edit vector menu entries independently showed four
anchors immediately. Escape cleared anchors/selection and returned Move. No
point was moved. Save retains every baseline field including timestamp, all five
assets, and the 30 project/48 total files exactly. The original `a3` failure is
preserved; this corrected checkpoint proves activation display, not vector
geometry authoring or its Undo/restart behavior.

Evidence is `native-node-entry-matrix-20261005/{inspect-refusals,mixed-selection-menu,latest-path-anchors}/`
and `native-editor-guards-candidate-20261005/{provenance.json,runtime.json}`.
`completed-entry-review-v4.json` independently recomputes all seven node-entry
captures, verifies the separate runtime identities and actual bundle binary
hashes, and retains the stated targeting/authoring limits. The fixture remains
35 nodes and five assets. Locked-node double-click combinations, richer mixed
selections, specialist content authoring and signed installation remain open.

The same source checkpoint contains the separate direct-bound instance-text
guard: seven focused tests, including three mounted GPUI cases across five
seeds; 45 resolver tests; full editor 1,154 passed/one ignored; three CLI tests
and lint passed before the anchor correction. Original refusal, owned-action
and cancellation failures remain in
`bound-instance-text-guard-20261005/integration/`. Its independent review is
pinned to the final patch and validation hashes. Owner-side late-binding
injection exercises retained-draft safety; it is not evidence of a normal
native route that mutates a binding during an owned preview, nor a general
merge for arbitrary override-index changes. Its direct native fixture is
separate and was still pending at this checkpoint.


## Native direct-bound text refusal and editable alias — 8a9a33c35e

The same Editor Guards QA candidate `8a9a33c35e2e586a533ac0b991b37a1cb13e14c5`,
binary SHA-256 `8d24b09c414c650c26724b616ddf6a200913e4b6942966186dc79a78c7a35ecb`,
was exercised on a fresh 24-node, four-definition, seven-variable/two-mode
fixture with no assets and 34 project files. Its frozen oracle allows exactly
one declared sparse Text override on the alias-only B01 instance; direct-bound
A/C instances, component defaults, placed aliases, sibling content, derived
geometry, bindings, modes, topology and every other field stay exact.

| Recorded checkpoint | Native observation and persisted result |
| --- | --- |
| Open | A01/A02 show Default A, B01 Placed B, B02 remains hidden, C01/C02 Derived B. The immutable baseline, timestamp and all 34 project files match. |
| Default-bound refusal | After isolated pairs select A01, double-click shows a notice naming Default alias, keeps Move and opens no caret. The current Design virtual Content row below the distinct Caption property is dim/read-only and opens no input. No typing or Save; strict baseline and 34 files unchanged. |
| Derived-bound refusal | C01 shows Derived B from the placed B pin despite its master context. Double-click identifies Derived caption and refuses; its virtual Content row stays read-only. No typing or Save; strict baseline and 34 files unchanged. |
| Inline alias edit and Undo | B01 enters inline editing; Command+A → QA alias → Escape/Save changes only its exact declared text override plus nondecreasing modified_at. One canvas-focused Undo/Save restores Placed B and removes that override entirely. |
| Design Content edit, Undo and Redo | The editable B01 virtual Content row commits QA alias while Caption remains a variable alias. Single Undo/Save restores the full baseline except timestamp; single Redo/Save restores the exact expected override. Masters, other instances and all variable data remain unchanged. |
| Repeat Save | An additional Save without edits matches saved Redo, including timestamp and all 34 file bytes. |
| Quit/relaunch persistence | PID 78080 exited before the same bundle/profile relaunched as PID 81329. The saved seeds, QA alias and hidden sibling reappeared without another edit/Save. Strict typed fields/timestamp and all 34 files match saved Redo. |

The post-restart **refusal interaction is inconclusive**. Repeated synthetic
click-count-two calls did not advance selection, whereas a single click selected
the containing Frame. No successful post-restart bound-text targeting/refusal is
claimed. This does not invalidate the independent saved-state comparison, and
it is retained for input-route investigation rather than relabeled as a passing
guard check.

All ten captured checkpoints independently recompute to their declared exact
persistence expectations. Five include strict byte comparisons. Oracle and
fixture hashes, snapshot hashes, recorded reader identity, runtime/build record
identity, stage timestamp chronology and same-profile quit/relaunch provenance
were checked. The review used small evidence files during a quiet CPU benchmark;
it did not rehash the large executables, rerun native actions or claim timing.
The binary/reader identities are tied to the previously recorded stable build.

Evidence: `native-bound-instance-text-20261005/native-*`, `ready.json`,
`native-acceptance-review.json` and
`native-editor-guards-candidate-20261005/runtime-bound-cold.json`. The oracle's
46 sensitivity cases and 34-file self-read are preparation checks, not additional
native journeys. The earlier oracle version is retained; the final version pins
baseline/descriptor hashes and excludes only root `.git` from the project-file
manifest. Missing/invalid binding behavior, retained drafts when a binding
arrives during an owned preview, arbitrary concurrent override-index changes,
true nested instances and virtual TextPath remain outside this native fixture.
No comprehensive E2E, signed-installer or performance pass follows.


## Ordered blend and closed-mask renderer API — PR 70

[PR 70](https://github.com/jeanc18rlos/fanta-edit/pull/70) extends the ordered
split-rendering API; it does not enable that API in live canvas dragging.
Complete subtrees and prepared instances stay atomic. Non-normal node and
per-paint blends require ordered drawing; a mask sequence crossing phases,
blended ancestor or background-sampling blur still refuses before drawing.
The sibling-mask partition follows normal forward/reverse paint order,
including consecutive masks and hidden/trailing-mask behavior.

The regression-only runs retained three blend refusals and two closed-mask
refusals, alongside the passing crossing-mask refusal control. Final validation
passes **37 split cases**, the full renderer library **378 tests with two
ignored**, and renderer/viewer lint plus dependency scan. Ordered picture replay
and direct phase painting retain the existing at-most-two-channel-value pixel
threshold, check matrix/clip/save-count preservation, and assert interior
pixels independently. A bounded independent review found no material issue and
matched the source/log fingerprints.

The first green experiment failed an exact cold-versus-warm normal-render cache
comparison even though both split pixel comparisons passed. A separate control
reproduced the same at-most-one-channel-value cold/warm difference without split
rendering. The corrected cache-contamination assertion remains byte-exact
against an independent normal-only frame with the matching cache state, with a
separate one-value cold/warm bound. The original failure and diagnostic are
retained; the split pixel threshold was not relaxed.

Evidence: `renderer-split-blend-masks-20261005/integration/validation.json`,
`independent-node-review.json` and their hashed logs. The tested patch SHA-256 is
`1d46a42f949e2e844d9bc97fc223f22bb2aad9343fe56dad6a68c730f999251d`.
There is no native/Metal parity, Spectrum eligibility or performance result from
these checks. Closed masks can still allocate blank layers in non-owning
phases. Retained-session integration and its matched measurements remain a
separate pending experiment; no live-canvas speedup is claimed.

## Mounted direct-download verifier — PR 71

[PR 71](https://github.com/jeanc18rlos/fanta-edit/pull/71) adds
`script/verify-macos-dmg`, isolated packaging regressions and Check/release
workflow integration. At source checkpoint
`de30030e885d370422e8ab288f4abb119a1432dc`, these are later automated changes:
the recorded native app remains the separate `8a9a33c35e` development build.

The verifier mounts the produced DMG read-only without opening it, compares the
mounted app's complete file/symlink/executable-bit manifest against staging,
checks stable-app metadata, document/URL registrations, bundled executable and
icon requirements, and checks the mounted signature. Notarized mode additionally
requires Developer ID/hardened runtime, a stapled DMG ticket and Gatekeeper
assessment of app and DMG. It verifies that the DMG and staging fingerprints
remain unchanged and always attempts detachment after entering a mount. A
failed check remains a failed report; existing reports are not overwritten.
The current stable bundle-version contract is tested, not the Store build-number
path. The signing TeamIdentifier is validated and reported, not pinned to a
configured expected team.

The isolated v2 suite passes **27 checks**, including an actual tiny inert DMG
create/verify/read-only mount/tamper-detection/detach cycle. The default suite
passes **26 with that one optional mount check skipped**, and root's subsequent
run against the applied shared scripts independently records the same 26/1
result. Tests cover preflight rejection before build tools run, intended
metadata/container types, content/symlink/mode mismatches, cleanup and trust-check
routing. Dummy tools and inert fixture executables do not prove product signing
or application launch.

Independent review caught a real CI false pass: the initial unspecified-shell
`python | tee` step returned success when its test runner exited 7. The corrected
step explicitly selects Bash with pipeline failure propagation. A regression
executes the actual workflow command body with a failing dummy runner and real
log capture; it now retains the log and returns 7. The initial failure, the
missing-notary-guard mutant and v1 artifacts remain preserved. Final independent
review found no remaining material blocker within this verifier's scope.

Evidence: `macos-package-verification-20261005/{validation-v2.json,
independent-node-review-v2.json,shared-ci-results.json}` and their recorded logs.
The v2 patch SHA-256 is
`92c5b5cf7500d1a8e790e16079dbe06fea32f536c18723e9bff47e04d24a9159`.
**No signed Fanta DMG was available or accepted.** Real notarization/stapling,
clean-account/Mac install and update, oldest-supported-OS execution, complete
Mach-O dependency/minimum-OS inventory and symbol matching remain release work.
The synthetic mount, pipeline and metadata passes are not substitutes for those
checks or for native application E2E.


## Post-restart bound-text refusal after input recovery — 8a9a33c35e

The follow-up `native-cold-guard-recovery` checkpoint passes on the same
restarted PID 81329. A plain Layers click had added A01 to its selected parent,
showing two layers; that observation is consistent with a held modifier, but
native modifier events were not captured. A modifier-only reset request was
rejected without input. Shift+Escape, Escape and a blank click then cleared the
selection/zoomed pane state. This does not establish the earlier cause or a
production correction, and the initial inconclusive targeting record remains.

After a fresh screenshot, isolated double-click pairs selected A01 and then
showed the Default alias binding refusal, with Move active and no caret. Its
read-only Content row opened no field. The equivalent C01 pairs showed Derived
caption refusal, retaining Derived B, and its Content row also remained
read-only. Escape and Save retained the exact saved Redo document, timestamp
and all 34 project-file bytes. The fixture has 24 nodes and no assets.

Independent small-file recomputation now passes all **11 recorded checkpoints**,
six with strict byte comparisons, including this recovery. Evidence is
`native-bound-instance-text-20261005/native-cold-guard-recovery/` and
`native-acceptance-review-v2.json`; the prior ten-stage review is retained
unchanged. Runtime and captured reader identities match the existing records.
No new binary hashing, native actions, performance measurement or broad input
reliability claim was performed by this review.


## Retained translation API and Spectrum refusal — PR 72

[PR 72](https://github.com/jeanc18rlos/fanta-edit/pull/72), commit `e09772a`,
adds an opt-in renderer translation session. It passes **eight focused retained
cases**, the full renderer library **386 tests with two ignored**, and
renderer/viewer lint plus dependency scan. These include the prior 37 split
cases. This new API had no meaningful pre-implementation runtime failure;
missing-symbol compilation was not performed or labeled a product regression.
A bounded independent review found no material issue. No live canvas/worker or
Metal activation was added; native acceptance still uses `8a9a33c35e`.

Spectrum run 2 attempted both selected image/instance targets at fit-all and
100% over three rounds: **all 12 preparations safely returned
UnsupportedAncestor**. Navigation and Inputs have explicit paint boxes with
clip_content false, while the current API requires an active ancestor clip.
Normal warmup rendering completed before preparation, but **zero parity runs
and zero timing comparisons executed**. Recorded preparation durations are not
drag benchmarks. The report remains overall_pass false. No pixel-parity,
speedup or below-16-ms claim follows; the separate ancestor-eligibility work is
not included in this checkpoint.

The original harness failures remain: ambiguous runtime-library selection was
rejected before linking, and run 1 rejected a bare NodeId before rendering.
Run 2 corrected those harness inputs rather than rebasing the expected scene.
The authored document stayed unchanged, and all **27,162 files/symlink targets**
covering 166,757,318 regular-file bytes matched the baseline; filesystem times
were not asserted. The API's memory accounting covers its surfaces/frozen
pixels and approximate pictures, not total Skia, registry or prepared-instance
memory or peak RSS. Exact per-frame registry/variable/asset comparisons remain
part of its CPU cost.

Evidence: `retained-worker-plan-20261005/session-candidate/integration/`
`final-validation.json`, `independent-node-review.json`,
`spectrum-cpu-run2/report.json` and `project-unchanged.json`. The tested API
patch SHA-256 is
`a2dc1beda0bab5d3baa82616cd1f6ee0b0735cee1c374ee7367de1c23ba58011`.
The earlier PR 70 results and its cache-oracle failure/control remain separate.

## Instance entry sequence control — mounted input only

`mounted_instance_entry_from_empty_selection` passes **one mounted test with
eight configurations across five scheduler seeds: 40 case executions**. It
checks complete no-modifier click-count-two and count-one/count-two pairs from
empty selection, for top-level and wrapped instances, with and without redraw.
Viewer lint also passes at the same source fingerprint. No production defect
was reproduced or input code changed. This covers the harness dispatch path,
not native OS modifier delivery, and does not establish the cause of the
original post-restart targeting ambiguity. The separate native recovery remains
recorded above. Evidence: `instance-entry-sequences-20261005/integration/`
`validation.json`, `lint-validation.json` and their retained logs.

## Original runtime phases — status at e09772a

The original plan contains Phase 0 and seven implementation phases. Preservation
and UI corrections do not establish completion of all performance work.

| Phase | Implemented or measured | Still outstanding |
| --- | --- | --- |
| 0 — Measure | CPU drag example, historical Spectrum import baseline, completed-worker gesture counters. | Broad UI/import spans, fresh-write sync count, specified synthetic heavy-drag benchmark and final native/release timing. Worker wall time is not input latency or FPS. |
| 1 — Per-frame work | Epoch-change refill suppression and unchanged-resolver pattern retention. | Structural/content revision split, derived inspector/chrome caching and throttling, active-page indexed snapping, cached master membership and gesture-pinned render source. |
| 2 — Components | Cached expansion child/root indexes and reused component roots in layer projection. | Layout/override/root lookup indexes, expanded-child culling and path retention, and component-preview-only worker patches. The measured child-index CPU benefit is modest; RSS was not measured. |
| 3 — Effect reuse | Phase 1 avoids wasteful hot-frame refill. | SceneDelta-based transform-safe eviction and verification; the current epoch still clears all layers on scene revision changes. |
| 4 — Retained move | Ordered split and retained translation APIs with invariant/pixel tests. | Live tool/worker/session integration, eligible Spectrum targets, parity/timing, gesture invalidation, full memory policy and tuning. All 12 recorded real-file preparations refused. |
| 5 — Drop work | No-op layout writes skipped; precise scoped solves, fixed-island preservation and derived Undo/Redo. | Incremental spatial index, persistent clip-path cache, unclipped-group bounds reuse, text-cache LRU and measured native drop latency. |
| 6 — Photos | Existing lazy decode/prewarm and display pyramid. | Display-resolution decode cap, parallel prewarm, upload-copy/mipmap work, byte-budget LRU and avoiding GPUI encoded-byte cloning/hashing. |
| 7 — Import/write | Shared reference tables, precise changed-artifact selection, verified-hash Save reuse; some per-master import caches already exist. | Low-sync fresh-project transaction, open-with-write-report, single-pass serialization, further override/blob indexing, parallel image/instance work and measured complete first-write improvements. |

The external source audit and prior evidence remain under
`performance-phase-audit-20261005/`. The old import baseline's report mislabeled
28 omitted Boolean operands as instance children; later fidelity corrections
are separate and that baseline is not a current import-performance result.
The synthetic Save acceptance median 546.673→292.797 ms measures that phase only.
No final native drag p95 below 16 ms, complete Save improvement or release-profile
memory claim is established.


## Mixed inspector edits and fast-exit scrub failure — 8a9a33c35e

The separate ten-node/five-asset inspector fixture passes **seven bounded native
checkpoints** on `8a9a33c35e` (binary `8d24b09c414c650c26724b616ddf6a200913e4b6942966186dc79a78c7a35ecb`).
At 320 logical pixels, the two-kind selection correctly showed mixed opacity;
typing 75% changed only the original 50% Vector and 100% Bitmap. Single
Undo/Save, Redo/Save and final Undo/Save passed the complete typed oracle.
All unrelated content and five assets remained exact. Equal explicit corners
showed summary 7/All corners; varied corners showed TL1/TR2/BL4/BR3, matching
serialized TL/TR/BR/BL order 1/2/3/4. Four separate equal-corner fields were not
observed or edited. The initial corner-check expectations were revised; the original plan remains recorded.

Open, read-only corner inspection and cold reopening have strict typed/timestamp
and **24-file byte** equality against their respective frozen saved baselines.
The original PID 81329 exited; the same bundle/profile reopened as PID 88491,
without a post-launch Save. These are persistence and observed-layout samples,
not a full inspector visual baseline.

Two attempts to scrub the Vector from 50% to zero remain **FAIL**. The first
started on the prefix icon outside the numeric scrub surface. The second
started on the numeric body and moved straight outside; the hover-only move
handler did not promote the armed input to a scrub. Neither achieved zero.
The later automated correction is described below; no corrected-build native
fix pass is claimed. Held Escape during dragging is covered only by mounted tests because
the native automation sends atomic drags. Evidence: `native-inspector-release-20261005/`
`native-review-v1.json`, each capture/verdict and runtime records. Independent
small-file recomputation confirms the seven passes and both retained failures
in `release-update-ui-spans-pr74-20261005/native-inspector-independent-review.json`.

## Fixed painted ancestors and real-file CPU timings — PR 74

[PR 74](https://github.com/jeanc18rlos/fanta-edit/pull/74), `fed62a3339`, allows
a fixed ancestor paint box whose children overflow without clipping. The
initial focused run retained two genuine UnsupportedAncestor failures and one
passing control; all three then passed. The full renderer library passes
**389 tests with two ignored**, and renderer/viewer lint passes. Earlier PR 72
refusals and setup failures remain evidence for that earlier implementation.

The new development-CPU Spectrum run accepted both declared image/instance
targets at fit-all and 100%, three rounds each. **552 full-frame comparisons are
exact (maximum channel difference zero); 1,440 paired measured frames** use
alternating normal/retained order, three warmup pairs and 120 measured pairs per
round. Nearest-rank quantiles below pool 360 samples per target/zoom:

| Target / zoom | Normal p50 / p95 (ms) | Retained p50 / p95 (ms) |
| --- | --- | --- |
| Instance 4358 / fit-all | 151.837 / 159.209 | 9.513 / 10.444 |
| Instance 4358 / 100% | 9.959 / 11.236 | 10.486 / 11.476 |
| Image 2110 / fit-all | 151.511 / 160.927 | 16.780 / 18.492 |
| Image 2110 / 100% | 1.489 / 2.268 | 9.163 / 10.546 |

The 100% image case regresses substantially, and the 100% instance case does not
improve. This needs a measured cost gate before live activation. Preparation
**114.743–281.855 ms** is excluded from frame timing and must stay off the UI
thread. Exact context validation costs about 3.1 ms median per frame. Counters
for the moving middle omit static traversal/mask work and recorded foreground
replay. Accounted surfaces/frozen pixels/picture bytes are not total memory or
peak RSS. The complete authored document and original project files remain
unchanged; no native worker, GPU presentation, drop, Save or release-build
latency is measured. There is no native below-16-ms or universal speed claim.

Evidence: `renderer-fixed-paint-ancestors-20261005/integration/`
`final-validation.json`, `spectrum-summary.json`, `spectrum-cpu-run1/report.json`,
`run-context.json` and `project-unchanged.json`. The later actual-Metal probe failed its original parity gate, as recorded below;
it is not a passing timing measurement or visible canvas integration.

## Inclusive UI-stage diagnostics — PR 75

[PR 75](https://github.com/jeanc18rlos/fanta-edit/pull/75), `f47763b6ae`, extends
opt-in gesture summaries with bounded counts, total and maximum wall time for
tool dispatch, FigView element construction, Design refresh and owned
ContentPreview postprocessing. Captured view/gesture identity and pending-span
completion keep release and late replies on their original gesture. Inactive
and mismatched nested views cannot contribute to another view's costs. Disabled
spans do not read clocks or allocate samples; small scope bookkeeping remains.
Separate guards time the viewer's six WorkspaceSession::open routes and encoded
thumbnail wrapping through the existing slow-operation threshold.

All **16 focused tests**, including six new pure cases, pass. The actual mounted
pointer lifecycle passes four release/redraw configurations across five seeds.
The full editor suite passes **1,164 tests with one ignored benchmark**, the
benchmark CLI passes three tests, and repository lint/dependency checks and
five-file formatting pass. A pre-existing test-format correction after the full
run is retained as an explicitly whitespace-only diff. Native log capture from
a new candidate remains pending. Evidence: `gesture-ui-spans-20261005/integration/`
`final-validation.json`, tested/final source hashes and retained logs.

These durations are **inclusive and overlap; do not add them together**. Render
means element construction, not later layout/paint. Thumbnail wrapping does not
measure full image decode/upload. Initial press setup and post-release settling
remain outside the gesture boundary. This measures neither CPU time, OS input
latency nor GPU timestamps and makes no performance-improvement claim.

For the phase map above, Phase 0 now has this bounded UI-span implementation
(native output still pending), and Phase 4 now has eligible real-file CPU
parity/timings. Live worker integration, exact invalidation/font ownership,
backend parity, preparation/cost policy and total-memory controls remain open.
Phases 1–3 and 5–7 retain the listed gaps; the original plan is not complete.


## Native four-format exports and simultaneous presets — 8a9a33c35e

The same `8a9a33c35e` inspector candidate exported a 220×160 Vector at 50% opacity
and a 220×160 Bitmap at 100% through actual PNG/JPG/SVG/PDF controls. The Vector
five-output checkpoint (four formats plus a repeated PNG) passes independent
dimensions/decoded appearance and preserves the first filename. The Bitmap
PNG/SVG/PDF checks pass source-derived center-cover appearance. A single
two-preset action then creates a repeated SVG with collision suffix and a
440×320 `@2x` PNG; the original nine files remain byte-identical, and all **24
authored project files remain exact**. The repeated SVG has a new generated clip
ID but exactly matching decoded pixels. Its initial overstrict byte-equality
harness failure remains retained. The 2× PNG checks 51,840 source-derived flat
pixels exactly.

The original Bitmap JPEG verdict remains **FAIL at four pixels** exceeding its
RGB tolerance of 10 by 2–3. Independent analysis identifies Q90/4:4:4 encoding
and reproduces mixed-block ringing with libjpeg; no export code or encoding
was changed. A separately declared, bounded DCT-flat-block criterion checks
**160 complete blocks / 10,240 pixels**, retains tolerance 10 and finds maximum
error 1. Four independent positive controls and seven negative controls pass.
This is not a rewrite of the original failure, a lossless-JPEG claim or general
quality certification. Broader effects, selection batches and visible error
paths remain open. No source Save was invoked during these export actions.

Evidence: `native-export-release-20261005/final-handoff.json`,
`operator-actions-v2.json`, the original `a01-b01-nine-outputs/verdict.json`,
`jpeg-compression-classification/analysis.json`,
`jpeg-v2-block-aware/verdict.json`, and
`two-presets-eleven-outputs-v2/verdict.json`. The operator also observed a plain
click selecting both nodes after the dropdown/export route. Its actual input
flags were not captured, so the selection anomaly remains unexplained.

## Opt-in click delivery and routing diagnostic — automated only

A reviewed `view.rs` diagnostic now records actual GPUI canvas down/up, shared
release and tool Press/Release events with event modifiers, window flags when
available, route/primary-guard outcomes and selection before/after. It is
disabled unless `FANTA_INPUT_DIAGNOSTICS=1`, caps reservations at **256 records
per view**, and includes at most 16 selected IDs plus total count/truncation.
It never records moves or keyboard/text payloads; disabled/capped paths skip
payload and snapshot construction. Nested callbacks can log before enclosing
callbacks, so the sequence denotes entry order, not log-line order.

All **three focused controls pass**, including both mounted tests across five
scheduler seeds: six cached/delivered modifier cases and two pan release orders
per seed. Scene, assets, history and dirty state remain unchanged. The first
pan test incorrectly assumed that the canvas handler always ended pan; its
failure is preserved, and the corrected test explicitly covers the existing
repaint-installed global listener. Production behavior was unchanged.
Formatting and changed-file checks pass. The subsequent combined editor suite
passes **1,171 tests with four ignored**, including these three controls; three
benchmark CLI tests and repository lint also pass. Actual native log capture
remains pending.

Evidence: `native-click-diagnostics-20261005/validation.json`, both focused logs,
`five-seeds.log`, reviewed/final patches and `operator-plan.md`. These controls
prove GPUI routing and modifier semantics, not native NSEvent/automation
delivery and not a fix for the original selection anomaly.

## Actual-Metal parity refusal and localization controls

The first test-only actual-backend probe compares normal `MacGpuRenderer`
rendering against CPU-retained pixels uploaded/composed/flushed through Metal.
Its first immutable Bitmap case at offset (-16,-6) fails: **maximum channel
difference 28 exceeds the predeclared 2**, with 517 differing pixels (405 over
2). **No timing samples are accepted** and the original failed frame/report
remain retained. The full-frame CPU render and CPU-retained render are exactly
equal; both differ from normal Metal at the same pixels.

Three independent Metal-only controls—ordered phases, recorded Above picture,
and same-context GPU Below snapshot with Src/middle/Above composition—each
match normal Metal exactly on that first fixture. This localizes a backend
appearance difference and supports further investigation; it does not validate
the complete retained-session lifecycle, a larger fixture matrix, live canvas
activation or GPU timing. The subsequent test-only GPU matrix now passes **all 40 full frames exactly**
(maximum difference zero): Bitmap and instance text at scales 1/2 plus transparent
Bitmap output, eight translations per fixture, retaining GPU Below and Above
picture. The existing CPU session validates each translation; its CPU image is
discarded and a fresh checked Middle spec is prepared. This intentional
diagnostic redundancy is untimed, not the proposed production fast path. All
authored documents, the test binary and 1,534 source fingerprints remain
unchanged. No performance, context-loss recovery, memory bound or live
activation is established. Evidence: `retained-metal-probe-20261005/integration/`
`metal-run1/failure.json`, `metal-run1/pixel-classification.json`,
`controls-run1/outputs/report.json`, `checkpoint-controls.json`, and
`gpu-matrix-run1/outputs/report.json` with its unchanged-source/run manifests
and `independent-release-review.json`.


## Fast-exit scrub correction and combined validation — native retry pending

The shared numeric input now observes an armed left-button scrub through a
frame-scoped capture listener, so its first movement may already be outside the
readout. The editor uses merged upstream revision
`66dc0f1cfec60e7190e0d3b1ee98925e48e632ca`. Four meaningful mounted regressions
failed on the old pin and pass on this revision; all **six scrub controls pass
across five scheduler seeds**, covering single/mixed values, redraw ordering,
Escape and late release. Existing document, Undo and asset assertions remain.
The two earlier native zero-opacity failures remain historical failures; the
new app's native scrub/Save/Undo/reopen retry has not yet been verified.

The frozen combined source—including click diagnostics and test-only Metal
probes—passes **1,171 editor tests, zero failures and four ignored tests**, plus
**three benchmark CLI tests**, repository lint/dependency checks and formatting.
The ignored tests are the existing layer-projection benchmark and three
explicit Metal diagnostics; their separately executed results retain their own
provenance. Source fingerprints stayed unchanged throughout validation.
Evidence: `numeric-scrub-fast-exit-20261005/editor-validation.json` and its
RED/GREEN, five-seed, full-editor, CLI and lint records. This completes the
automated correction checkpoint, not exhaustive UI acceptance or release
certification. The backend-compatible retained-surface proposal remains
external and is held for the next session; the 40-frame diagnostic is not a
production surface API or a performance result.
