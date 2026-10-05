# Release capability coverage — 5 October 2026

**Release readiness is not established.** The target window is Monday
**5 October through Sunday 11 October 2026**, Europe/Madrid. The immediate user
reports are degraded inspector UI, double-clicks with no useful node action,
failures or missing content, and slow editing on large pages. Treat those as
release blockers until reproduced and verified on the candidate build.

The [capability guide](../fanta/capabilities.md) inventories current source.
This report maps that inventory to executable tests and outstanding acceptance
work. The local checkpoint below records completed checks; the capability map
identifies their scope and the remaining acceptance work. Historical evidence in
[RELEASE_VALIDATION.md](RELEASE_VALIDATION.md) applies only to the builds and scope
recorded there.

## Evidence levels

| Level | What it proves | What it does not prove |
| --- | --- | --- |
| Engine | Document operations, serialization/recovery, tool events, geometry, rendering pixels and prototype evaluation. Includes files named `end_to_end.rs`. | Native controls, OS event delivery, installer behavior or live services. |
| GPUI integration | Editor entities, real view trees and, where explicitly simulated, pointer/key input, focus, previews, UI callbacks and file effects in a test harness. | Every visible control, the real platform input path or the final bundled app. Some GPUI tests call handlers directly. |
| Native component visual | Screenshot comparison of inspector, animation-panel and timeline component galleries on macOS. | The assembled properties inspector, every node/selection state, text clipping at all widths, or complete app journeys. |
| Live MCP integration | A running app's protocol, document mutations, rendered screenshots and autosave reaching disk. | Toolbar/menu usability, double-click routing or drag responsiveness. |
| Native app E2E | A built app driven through its actual controls, with visible results and saved/reopened content verified. | Other untested node types, modes, devices or installers. |

The Gherkin files in [features/](../../features/README.md) are acceptance
specifications; there is no Cucumber/feature-file runner. The interactive
`fanta_gpui_smoke` example is a manual component demo. Neither is passing E2E
evidence on its own.

Virtual instance text now rejects locked text/ancestors, occluded targets and
content outside ordinary or rounded clips. Four targeted regressions passed in
the local editor checkpoint. Boolean operands also support explicit drill-in
and vector editing with Undo preserving the boolean container. These harness
results still require native app acceptance.

## Local checkpoint — 5 October

The Boolean/import candidate is `a4adca9570`, followed by the behavior-preserving
lint cleanup `0bec851a8a`. Its engine, importer and
source-order checks are in `2026-10-05-preservation-engine/`; the editor and
remaining assembled-app checks are in `2026-10-05-preservation-app/`, all under
`target/release-verification/`. The current completed results are 1,764 engine
tests (6 ignored), 282 importer tests (11 ignored), 310 source-order tests,
1,040 editor tests (1 ignored), and 3 benchmark CLI tests, with zero failures.
Eight focused standalone-document tests additionally cover the final JSON
precision dependency. Source-file hashes are retained with the app run.
The same app run passed all 234 workspace tests, 9 UI tests and app/CLI
compilation. Existing component baselines also passed without regeneration:
animation panel 99.973%, timeline 99.989%, inspector 99.956% (minimum 99.95%).
The initial lint failure is retained; the result-conversion cleanup passed the
full repository lint gate in `2026-10-05-preservation-lint/`. The current source
has a passing result for all eight default stages plus component visuals. The
final native rebuild is recorded separately; historical native results below
do not substitute for it.

The current isolated native QA bundle was built at
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
regression reproduces that failure; the focus correction still requires a new
native build before acceptance.

Native inspection on `6fd579b408` also caught an Export-label defect: the label
could collapse to an ellipsis despite the passing component visual baselines.
The upstream Fanta UI PR #8 correction still needs integration and a native
recheck of the assembled inspector. The earlier long-name overflow fix and
component screenshot pass do not close this defect.

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

## Capability-to-test map

Paths below are relative to the repository. Every row still needs a native
release-candidate journey; the final column highlights the most important gap.

| Capability | Existing automated evidence to run | Remaining acceptance work |
| --- | --- | --- |
| Create/open/import | `fig_viewer::new_design::tests`, `document::tests`; `fanta-fig-interop` mapping/parser tests | New project and `.fig`/`.fant` import, collision handling, malformed file and missing asset UI; compare representative imported pages with reference renders. |
| Save/autosave/Save As/reopen | `fanta-format` full suite; `fig_viewer::document::tests`, `view::tests`, `view::serialization::tests` (`canvas_session_*`, `save_generation_*`, `a_failing_autosave_is_reported_until_a_save_succeeds`, `save_as_*`) | Verify file hashes and node/assets inventory across edit, cancel, save, restart and Save As. Inject write failures and concurrent edits; original content must survive. |
| Source ↔ canvas | `fig_viewer::code_workspace::tests`, `editor_session::tests`; `fanta-fnx` and format tests with and without `serde_json/preserve_order` | FNX and JSON typing/saving, invalid drafts, watcher reload, source lock, multiple tabs/windows and agent source-follow on the actual app. |
| Pages/layers/structure | Viewer design-panel, structure, layer-context and clipboard tests; `canvas_menu_reorder_entries_execute_and_undo` confirms both ordering entries. Other menu GPUI cases confirm primary text/vector/bitmap/video/audio entries and retain a selected boolean operand. | Layer drag/drop, all per-kind context actions, page deletion/duplication, copy/paste and undo with components/assets. Test active-page and hidden/locked rules in the native app. |
| Navigation/selection/transforms | `fanta-canvas` hit-test/snap tests and `tests/end_to_end.rs`; `fanta-tools::select::tests`, `scale::tests`; viewer toolbar adapter tests | Pan, zoom, nested selection, rapid drag/release, resizing and scaling in a dense imported page. Check focus and pointer capture. |
| Double-click by node | Viewer `canvas_double_click_*`, `mounted_curved_text_path_*` and mounted crop/drill/guard cases, existing standalone/wrapped text cases; tools `rapid_clicks_drill_once_per_pair_and_do_not_enter_leaf_nodes`, `extending_double_click_toggles_the_container_without_drilling`, `double_click_at_container_resize_handle_still_drills_into_child` | Native pass of the [per-kind contract](../fanta/capabilities.md#double-click-behavior-by-node), including selected/unselected, nested, locked/Inspect states. Check entry, feedback, Escape, Undo and unrelated content. |
| Properties inspector | `fig_viewer::gpui_adapters::design::tests`, `properties_panel::tests`, `properties_ops::tests`, `properties_snapshot::tests`; mounted `view/properties_inspector.rs::layout_tests` checks geometry fields at minimum width and the composed inspector with annotation/measurement lists | Assembled inspector screenshots and input journeys for empty/single/mixed selection, narrow panel, light/dark themes, scrolling, popovers and page switches. Mounted layout bounds are not a full visual baseline; legacy-panel tests alone do not cover the default inspector. |
| Drawing/path/region/crop | `fanta-tools` full suite, including `tests/end_to_end.rs` and `ink_oracle.rs`; viewer toolbar adapter tests | Every visible tool, path/anchor editing, brush/eraser, region operation, crop Apply/Cancel and their keyboard shortcuts. |
| Text/text on path | `fanta-text`, `fanta-tools::text_path::tests`, renderer text/text-path tests; viewer `text_edit`, `instance_text` and design adapter tests | Inline range selection, rich styles, multiline/Unicode/IME input, fonts, path conversion errors, instance overrides, save/reopen and exports. |
| Layout/paints/effects/rendering | `fanta-doc` layout tests; complete `fanta-render` library and bitmap/SVG/compose/golden integration suites | Visual parity for gradients, masks, booleans, clipping, shadows/blur, blend modes, auto-layout/grid and imported instances under edits. |
| Variables/styles | `fig_viewer::variables_workspace::tests`, `variable_binding::tests`, `agent_surface::tests`; document resolve/render tests | Create/rename/delete, modes/aliases, compatible bindings, unbind and undo through UI, then reopen and compare. |
| Components/variants | Viewer component-property, variant-set, clipboard and agent tests; `fanta-doc` instance resolution tests; importer overrides tests | Master ↔ instance updates, virtual text edit, typed properties, variant switching, detach/duplicate and nested components without disappearing descendants. |
| Motion/timeline | Viewer `motion_panel`, `motion_edit`, `timeline`, toolbar adapter tests; document/render motion tests | Keyframe drag/scrub/play, easing edit/cancel, clip switching, duration and mode changes, then exact save/reopen. |
| Prototypes | `fanta-present`; viewer `prototype_panel`, `prototype_player` and view tests | Pointer/key/time triggers, navigation, overlays, transitions, safe links, restart/exit and viewport restoration. |
| Comments/review/Dev | Viewer `comments`, `comments_ui`, `view_annotations`, `view_measurements`, `view_dev_mode` and export tests | Pin/reply/resolve, draft preservation, mode transitions, keyboard ownership and read-only protection; no review overlays in artwork exports. |
| Local image/SVG/video/audio | Viewer `generation_media`, `video_playback`, document/view and media tests; renderer live-media tests | Place/play/seek/trim/replay, corrupted files, missing source, poster/orientation, audible output and saved asset bytes after restart. |
| Export | Viewer `export::tests` and inspector export tests; renderer integration suites | PNG/JPG/SVG/PDF from UI, multiple presets/selections, names/dimensions, layout fidelity, visible failures and opening the resulting files. |
| Designer/MCP | Viewer `agent_surface` (including style projection), `live_mcp` and `plan_build` tests; `script/smoke-mcp` | Final app stdio/socket connection, real agent tool selection, one undo per successful batch, rollback on failure, source validation and screenshot inspection. |
| Generation/recovery | Viewer `generation_workspace`, `generation_journal`, `generation_media`; account/provider CI tests | Signed-in final build with a real provider: submit/poll/save/place, timeout/retry, restart, sign-out/account switch and exactly-once recovery. Mock responses do not prove production availability. |
| App shell/distribution/accounts | Existing `Check` jobs: sidebar, path prompt, agent toggle, Git, auth, Store restrictions; release packaging workflow | Menu/keyboard discovery, clean-profile launch, install/quarantine, signature/notarization, Keychain, sandbox file access, billing/restore and backend release compatibility. |
| Large-page performance | Import scaling tests, renderer cache/culling tests, ignored format latency gate and profiling examples | Matched before/after import, save and gesture timings on Spectrum plus a synthetic scene. Measure frame p50/p95/max and memory, not just load completion. |

## Next bounded native checks

These four checks remain pending. Use fresh disposable copies of the fixtures
below, preserving their existing evidence. Record the exact candidate binary,
initial typed document and asset hashes, native inputs and final comparison.

| Priority | Fixture and bounded journey | Required evidence and limit |
| --- | --- | --- |
| 1 — Inspector drafts | Copy `/tmp/fanta-release-node-matrix-20261005`. Cancel a numeric scrub and color preview with Escape; edit a mixed selection and Undo; attempt a page switch with an invalid numeric draft. Repeat the relevant controls at minimum panel width. | Cancelled edits leave typed content unchanged; the committed edit takes one Undo; invalid input is retained or explicitly rejected without applying to another selection. Save/reopen and compare all unrelated nodes and asset bytes. Include the collapsed Export-label recheck after its fix is integrated. |
| 2 — Clipboard and structure | Copy `/tmp/fanta-release-qa-20261005` for its main component and instances, and `/tmp/fanta-release-node-matrix-20261005` for its bitmap. In each copy, copy/paste across pages, then Undo/Redo; attempt a cross-project paste. | Verify supported insertion or a visible refusal, correct active-page placement, one history step and preserved component/asset dependencies. Save/reopen and compare original masters, source nodes and asset bytes. A rejected unsupported transfer is not a successful cross-project-copy capability. |
| 3 — Large-page save and drag | Import the unchanged `/Users/jeanrojas/Desktop/untitled folder/Adobe Spectrum Design System .fig` into a new disposable project. On page index 9, move instance index 4358 and image-filled frame index 2110, then Save, Undo, Save and restart. | Compare the full typed document and every asset byte, accounting only for intended edits and documented persistence normalization. Record native pointer/frame timing and first/subsequent saves on a quiet machine. `fig_to_project` checks only page/asset counts; the 12-Boolean subtree oracle and CPU `drag_bench` do not establish whole-project preservation or native drag latency. |
| 4 — Variables, Motion and Prototype | In another copy of `/tmp/fanta-release-node-matrix-20261005`, create one color variable with two modes and bind a shape; switch modes, unbind and Undo. Add two small frames, one motion track and one prototype navigation link through the UI. | Verify the displayed bound values, undoable edits, scrub/play and navigation/restart/exit with restored editor viewport. Save/reopen and compare bindings, track/keyframes and reactions. This is one representative native journey per subsystem, not all triggers, easing types, aliases or variant combinations. |

## Reproducible local verification

```sh
./script/verify-fanta-release --list
./script/verify-fanta-release
# Run a focused stage after changing that area:
./script/verify-fanta-release --stage editor
# Separate native component screenshot comparison on macOS:
./script/verify-fanta-release --stage visual
# Explicit representative-file node-structure gate:
FANTA_FIG_FIXTURE=/path/to/design.fig ./script/verify-fanta-release --stage import-preservation
```

The runner uses `--locked`, executes public engine integration suites as well
as libraries, tests the default GPUI editor, benchmark CLI tests and UI primitives, checks app/CLI
compilation, and invokes the repository's `./script/clippy`. It continues after
a stage failure and exits nonzero if any stage failed. Each run writes
`environment.txt`, stage logs and `results.tsv` to a new
`target/release-verification/` directory. `--output-dir` selects a fresh
destination; `--stage` can be repeated. Preserve these artifacts with the
candidate revision and dirty-tree state. A compile-only result is not a test
pass; a component snapshot pass is not app E2E.

The runner rejects `FANTA_REGEN_GOLDEN`, `UPDATE_BASELINE` and
`UPDATE_BASELINES`: regenerating the expected image is not validation. Review
intentional visual changes before updating baselines separately.

The importer stage runs synthetic library fixtures. The private Spectrum
integration binaries are deliberately excluded: they return early when
`FANTA_FIG_FIXTURE` is missing and can appear as Cargo passes without exercising
the fixture. Run them explicitly with the correct local Spectrum fixture and
save the fixture identity, dimensions/counts and output with the report. Normal
test runs also skip `#[ignore]` benchmarks and the network font-download test.
The explicit `import-preservation` stage fails if its fixture is missing or if
the importer reports flattened Boolean operations, omitted operands, other
non-container losses, or skipped/malformed nodes. It checks structure reports;
it does not establish rendered parity or complete instance override fidelity.

For live MCP, run `script/smoke-mcp <candidate-binary> <disposable-project>`
against an explicitly isolated QA app/project: it writes a rectangle and can
reuse a running app. Set `FANTA_USER_DATA_DIR` to the candidate profile and use
`--screenshot-path` to retain its PNG. Retain its assertions and screenshot alongside native UI
evidence. It is intentionally outside the default runner.

The `Check` workflow runs the runner's engine, import, source-order, editor,
workspace and UI stages and uploads their logs/results, including after a failed test stage.
The default runner also includes build and lint. Workspace coverage exercises
serialization ordering, immediate Quit, Hot Exit and tab restoration alongside
the canvas-specific journeys in the editor suite.
It also covers account/provider, sidebar, Git and Mac App Store execution
restrictions. Verify both hosted jobs for the exact
candidate revision; local editor suites do not replace them. Run the optional
native-video job when diagnosing platform playback.

## Release gates for this week

| Target | Required outcome |
| --- | --- |
| Mon 5–Tue 6 Oct | Establish the integrated candidate, reproduce inspector and double-click failures, preserve content baselines, land focused regression tests and make all public automated suites green. |
| Wed 7 Oct | Complete per-node and inspector acceptance matrices; run save/restart/undo and failure-injection journeys. Every missing-content report has a reproduction or a documented unresolved blocker. |
| Thu 8 Oct | Measure import, photo/instance drags, drop/save and memory on representative large pages. Apply measured fixes with render parity and persistence checks. |
| Fri 9 Oct | Freeze a candidate; run the complete suite, native UI journeys, component screenshots, live MCP and production/account checks against that same build. |
| Sat 10–Sun 11 Oct | Verify signed installer on a clean account/Mac, resolve release blockers, refresh capabilities/known issues/release notes, and make a release decision from retained evidence. |

This is an acceptance schedule, not a claim of completed work or a scheduled
background job. The performance handoff's target is drag **p95 below 16 ms** on
the Spectrum page; no current measurement here establishes that target.
`session_latency_gate` measures a document/session operation, not displayed
frame latency. `first_save_profile` writes into its input project, so use a
disposable copy. The `drag_bench` example provides read-only `.fig` imports and CPU raster timings
at fit-all and 100% zoom, including image/instance selection and renderer metrics:

```sh
cargo run --locked -p fig_viewer --example drag_bench -- \
  /path/to/Spectrum.fig --page-index 9 --node 4358 --frames 120
# Image-filled frame from the recorded fixture:
cargo run --locked -p fig_viewer --example drag_bench -- \
  /path/to/Spectrum.fig --page-index 9 --node 2110 --frames 120
```

The node argument is a depth-first index within the page; imported node IDs are
regenerated. Record the fixture hash, page/node identity, pixel size and build
mode with every result. Its JSON reports CPU work; it does not measure native pointer delivery,
GPUI composition or Metal presentation.

The candidate is releasable only when every advertised capability has a recorded
pass or an explicit documented limitation; there are no unresolved data-loss,
crash, ignored-input or severe-performance blockers; and the exact artifact has
passed the distribution gates. For each run record revision/build ID, date,
machine, fixture, action, expected/actual result, log/screenshot and follow-up.
Old passing counts and unexecuted specifications must never fill an evidence gap.
