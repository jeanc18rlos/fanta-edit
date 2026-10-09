# Release capability coverage — 9 October 2026

**Release readiness is not established.** The target window is Monday
**5 October through Sunday 11 October 2026**, Europe/Madrid. The immediate user
reports are degraded inspector UI, double-clicks with no useful node action,
failures or missing content, and slow editing on large pages. Treat those as
release blockers until reproduced and verified on the candidate build.

The selected distribution is a **direct-download Mac app**. Final acceptance
must use the signed, notarized DMG and the app installed from that exact DMG.
Internal ad-hoc-signed optimized/development QA bundles and Mac App Store checks
are separate evidence; none establishes direct-download installation readiness.

The earlier integrated main checkpoint `6ac89e6a2193603d45f7ce517d52493f2386884f`
passes the hosted Check and Mac App Store runtime jobs in
[run 37534261732](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37534261732).
The push checkout tree matches that exact main commit. The native-video job
was skipped; this result closes hosted CI for this checkpoint, while exact
installer and remaining native acceptance gates stay open.

The [capability guide](../fanta/capabilities.md) inventories current source.
This report maps that inventory to executable tests and outstanding acceptance
work. The dated evidence appendix records completed checks; the capability map
identifies their scope and the remaining acceptance work. Historical evidence in
[RELEASE_VALIDATION.md](RELEASE_VALIDATION.md) applies only to the builds and scope
recorded there.

## Current acceptance status

### Final source and internal package — 9 October

Main `79cc2ef8c094149e8496476c1cb53e2945b8b54a`, tree
`bd69de45f92675b55d32f647d6c8c147da237537`, passes exact-main Check and Store
in [run 37884589268](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37884589268).
All six verification stages pass: **1,212 editor tests, 405 renderer tests and
four drag-example tests**, with zero failures; two editor and two renderer tests
remain ignored. The totals include 22 layer-cache cases. The same tree passed
the candidate's separate [twelve-crate release lint](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37881418740).
This is not full-workspace lint or a new main-push lint execution. Ordinary
native video was skipped; private Spectrum and native performance are separate.

Effect-layer invalidation preserves unaffected cached layers and uses conservative
fallbacks for changed dependencies. Exact cache-on/off pixel tests pass; the
9,012-definition comparison timings are unoptimized diagnostics, not native drag
latency. Retained translation remains disabled. The merged Bitmap JPEG quality-95
fix passes the original regression without changing its mask or tolerances;
the quality-90 failure remains preserved. Latest native export still needs its
own result.

Internal optimized [build33](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37884728571)
uses that exact main source. Installer/checksum acquisition, hosted and local
mounted-bundle verification, CLI and isolated QA derivation pass. The public
release job was skipped. This arm64 artifact is ad-hoc signed (`signed=false`);
Developer ID/notarization, quarantine installation, public download, update and
rollback remain unaccepted. Broader native editor, Spectrum, service and
performance gates remain open; this checkpoint does not supersede their dated
failures.

Evidence under the local QA archive's `integration-20261007/`:

- `effect-layer-invalidation-20261009/canonical-ci-integrated/root-terminal-review.json`
- `final-main-macos-artifact-20261009/independent-monitor/terminal-37884589268/root-terminal-review.json`
- `final-main-macos-artifact-20261009/coherent-suite-v2/actual-acquisition-run1/result.json`
- `final-main-macos-artifact-20261009/coherent-suite-v2/local-qa/actual-local-run1/result.json`
- `bitmap-jpeg-quality-q95-publication-20261009T0208Z/canonical-ci/terminal-review/root-terminal-q95-review.json`

The following earlier checkpoints retain their original source identities and
acceptance limits.

### Combined-main automated checks — 9 October

Exact main `7ee4525f7578add2dcd32cd4b35b62edcdbf805b`, tree
`5324a22114499eb7ded0acb7587d1fb5e8e9d3f7`, passes hosted Check and Mac App Store
runtime in [run 37866707698](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37866707698).
Both job checkouts match that revision. All six release-verification stages
pass: engine, import, source-order, editor, workspace and UI. The editor stage
passes **1,211 tests plus four drag-example tests**, with zero failures and two
ignored diagnostics: the actual Metal/CoreVideo retained-worker check and the
component-heavy layer-action CPU benchmark.

The logs include both new Audio lifecycle tests, both semantic Section-matching
tests, all three TextPath inspector-header tests, and the strengthened
`canvas_menu_primary_entries_activate_the_matching_editor` test. The latter
starts the inspector collapsed and checks actual properties-menu reveal for
seven node kinds; it also covers TextPath **Edit text** entry. These are automated
regressions, not new native acceptance or a fix for the unresolved Audio preview.

The native-video job was **skipped**, and the private Spectrum import-preservation
stage was not executed. This Check does **not** run `script/clippy`; no full lint
pass is claimed. Installer/signing, native persistence, performance and production
service gates remain open. The independently reviewed run, source identities,
logs and ten-file artifact are retained at
`integration-20261007/final-main-resumed-20261009/independent-terminal-review.json`.

### Bounded checks through 8 October

[PR 97](https://github.com/jeanc18rlos/fanta-edit/pull/97) merged as
`658c75a6534be16ce470c0255b9bb6518ceb0703`, tree
`196297d81accd33120602ff699eb4d3a4c6fbd66`. Exact-tree hosted Check/Store passed,
including five managed-MCP failure-path regressions and 1,204 editor tests
(two ignored). These simulated-provider tests do not prove production MCP.

The [integrated optimized build](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37817617449)
passed hosted packaging and mounted verification. Its downloaded installer,
checksums, local bundle verification, CLI and isolated QA derivative also passed.
It is an internal arm64, ad-hoc-signed artifact (`signed=false`), not a
Developer ID/notarized public release. Latest-build native observations include
an existing signed-in account, 12 visible model rows and a rendered Spectrum
page; they do not establish paid AI, accepted prices/caps, live MCP, full editor
persistence or performance.

The following native journeys used the **older optimized `dddcc367d8` build**
([run 37658174007](https://github.com/jeanc18rlos/fanta-edit/actions/runs/37658174007))
and its isolated QA derivative. Its editor/media code matches the integrated
tree; the later MCP guard does not retroactively make these latest-binary tests.
All earlier failures remain recorded.

| Bounded journey | Accepted scope | Still excluded or unresolved |
| --- | --- | --- |
| Multiline text | Command-Return/Save, one Undo/Redo and cold reopen; 10 nodes, one asset, 20 files. | The fixed 48-point single-style box clips the second line. No auto-resize, full-glyph fit, rich text, IME or selection restoration claim. |
| Alternate B bindings | Project mode change/Save, Undo/Save and cold; 28 nodes/25 files, ancestor-pinned controls unchanged. | Redo, variable-definition edits and general mode/binding coverage. |
| Source conflict Discard | Unsaved draft plus an external change, Workspace Discard and cold; external source, seven nodes and 19 files preserved. | No Fanta Save followed resolution. Recovery Undo and multiwindow/async races remain. |
| Save As refusal | Cancel, occupied-folder refusal, actual original-target text edit/Save and cold; 10 nodes, one asset, 20 files and destination sentinel preserved. | Successful Save As to a new destination and Undo/Redo. Autosave may precede explicit Save. |
| Existing Audio | Explicit Play/Pause, paused midpoint seek, Save and cold; 35 nodes, five assets, 48 files. Reselection/Motion reopening were explicit. | Audibility, sample precision, history availability, restoration and performance. |
| Corrupt/unsupported media | Corrupt-MP4 and loose-MP3 refusal, no-edit Save and cold; exact typed content, 48 files and both Git boundaries. | History availability and other codecs/missing-media cases. Loose-MP3 refusal is not project-Assets placement. |
| Project-Assets MP3 | Placement/Save, Undo/Save, Redo/Save and cold storage/geometry; final 36 nodes, six assets and 49 files match saved Redo. | **Cold preview readiness is unaccepted:** the whole Audio preview was absent for the original MP3 tone and the newly placed one-second MP3; no WAV case was verified. The storage oracle did not check that panel; this is not a codec conclusion or an Audio fix. |
| Fresh Spectrum first-open archive | Qualified composition of full mapped-document/raw-source checks and the permission/inventory tail on the same archive: 73,925 nodes, 9,012 definitions, 23 pages, 41 assets, 27,162 files/9,049 directories. | Not a newly passed end-to-end capture or full raw-Kiwi fidelity proof; the original offline terminal position was transcribed from tool output. Schema-proven f32 fields compare exact bits; fresh files use source-defined 0600 permissions. Original numeric/permission refusals remain. **No fresh-import Save, cold or performance acceptance.** |

Evidence is retained under the local QA archive's `integration-20261007/`:

- `managed-mcp-cache-guard-20261007/merge-20261008/merge-confirmed.json`
- `optimized-latest-mcp-artifact-20261008/local-package-acceptance/runs/actual-20261008-run1/result.json`
- `native-multiline-command-return-acquisition-v5-20261008/actual-chain-review-20261008.json`
- `native-bindings-alternate-optimized-acquisition-v3-20261008/actual-chain-review-20261008.json`
- `native-source-conflict-discard-optimized-acquisition-v3-20261008/independent-cold-review-20261008.json`
- `native-save-as-refusal-acquisition-v3-20261008/actual-through-cold-review-20261008.json`
- `native-media-noedit-acquisition-v3-20261008/independent-audio-through-cold-review-20261008.json`
- `native-media-noedit-acquisition-v3-20261008/actual-corrupt-through-cold-review-20261008.json`
- `native-mp3-assets-placement-optimized-acquisition-v3-20261008/peer-through-cold-qualified-20261008.json`
- `fresh-spectrum-import-strict-v4-20261008/historical-first-open-archived-acceptance.json`
- `integrated-production-isolation-v1-20261008/independent-native-signin-catalog-review-20261008.json`

**Release gates stay open:** broader editor/IME/held-input coverage; missing Audio
preview and media breadth; Spectrum Save/cold/native latency and memory;
Developer ID/notarization, public download/install/update/rollback; and live
production AI/credits/MCP/recovery. Pricing retrieval failed before SQL; visible
model price chips do not establish a safe total paid-request bound. No paid AI
request was submitted in these checks. Prepared adapters and source-only test
patches are not native acceptance or product fixes.

### Earlier bounded checks — 7 October

- **Inspector display/layout PASS on QA2824:** 16 static checks cover four
  controls at 320/400 logical pixels in One Dark/One Light. Already-authored equal
  independent corners show four fields; the precedence explanation is readable
  in full, bound wrapper opacity shows 40%, and the selected Vector/Bitmap pair
  shows Mixed. Thirteen strict snapshots preserve complete typed content,
  timestamps, assets and files (bindings: 28 nodes, 25 files; mixed: 10 nodes,
  5 assets, 24 files). Independent review verifies 144 linked hash records.
  Bound Detach was visible but not invoked; this journey does not establish
  authored edits, Undo/Redo, scrub cancellation or cold restart.
- **Component navigation PASS on `67d29ea288`:** four actual Go to main component
  routes resolve Large/Small/Large/Small and align artwork with selection.
  Two strict checkpoints preserve all 20 nodes, 1 asset, timestamps and 30 files.
  The earlier four alignment failures remain recorded. This is navigation
  acceptance, not a saved Reset/Detach or cold-reopen claim.
- **Frame/Section Escape PASS on `67d29ea288`:** each native drill-in enters
  inline text, then one Escape exits before a later blank click. Eight
  screenshots and independent input-state review support the two routes;
  all 10 nodes, 1 asset, timestamps and 20 files remain unchanged. Three mounted
  Escape cases and a bound-text cancellation control pass across five scheduler
  iterations. Edited text, IME and TextPath are outside this native check.
- **Variant action persistence PASS on `41becdffe5`:** ten native checkpoints
  cover Open, two navigation controls (default Large and pinned Small), Reset,
  single Undo/Redo saves, restoration before Detach, Detach, its single Undo/Redo
  saves, and cold reopening. The complete typed oracle and file boundaries pass:
  20 original nodes, 1 asset and 30 files, with exactly the expected 21-node
  detached state. C01 becomes an 80 × 40 Frame with Small content. Only declared
  files change; verified process exit and exact-binary reopening preserve the
  saved Redo payload, timestamp and every file byte without another Save.
  The earlier strict Detach failure remains retained: it duplicated four
  component-owned nodes into unrelated page source and dropped page attributes.
  The ownership correction (`2c7a9d1cd0`) and scope index (`41becdffe5`) pass
  four ownership regressions, three index regressions, 234 format tests
  (2 ignored), source-order tests (98 FNX plus 234 format), 1,196 editor tests
  (2 ignored), and the required ten-crate lint. These fixtures do not establish
  native edits of unknown source attributes or arbitrary nested detachments.

- **Retained GPU/worker correctness PASS; performance acceptance remains open.**
  The current code includes the Metal retained-session API and an opt-in macOS
  canvas worker. `FANTA_RETAINED_TRANSLATION=1` enables that path; normal rendering
  remains the default. On the code lineage retained by `bf0650022c`, seven
  actual-Metal API tests and one actual Metal/CoreVideo IOSurface-worker test
  pass, including bounded parity, ownership/invalidation and fallback controls.
  These runs establish neither native drag latency nor default enablement.
  Separate development-profile Spectrum Metal diagnostics retain all failures:
  image fit-all p95 improves from 150.59 to 25.68 ms but remains above 16 ms;
  at 100% it regresses from 2.55 to 10.46 ms. The later 100% instance case is
  slower (6.32 to 7.21 ms, with 196.37 ms preparation). Fit-all instance timing
  is rejected because the moving target changes no pixels. The planned six-run
  timing acceptance did not complete. These are standalone backend measurements,
  not native input/presentation or final-release measurements.
  Evidence: `integration-20261007/current-cut-platform-checks/run1`,
  `metal-spectrum-partial-assessment.json` and
  `metal-spectrum-instance-continuation-assessment.json`.

- **Structural FNX source preservation: automated and bounded native PASS on
  `bf0650022c`.** Hosted CI is tracked separately for each published candidate.
  Surviving future attributes and
  comments remain intact through structural saves. Explicit source removals stay
  removed. Unsupported destructive changes report a preservation error rather
  than silently printing a lossy replacement. All 13 node kinds recognize known
  defaults; conversion removes only old-kind fields absent from the new schema.
  Instance/media now receive non-fatal unknown-attribute warnings.
  The initial five source-loss failures and three conversion failures remain
  retained. Final validation passes 13 focused regressions, 401 document tests
  (1 ignored), 98 FNX tests, 248 format tests (2 ignored), source-order runs of
  98 FNX plus 248 format tests, 1,196 editor tests (2 ignored), and the required
  ten-crate lint/dependency audit with actual macro-load proof.
  Nineteen native checkpoints pass: ten plain-sibling Duplicate/Delete states,
  three visible refusal/recovery states, and six intentional source-removal
  states. Each checks complete typed content, source, 1 asset and all 30 files.
  The three cold journeys match saved Redo payloads, timestamps and file bytes;
  duplicated identities survive Redo and cold reopen. A protected Text deletion
  first changes the live canvas, then Save visibly refuses and preserves every
  persisted byte; one Undo and Save restore the original content. No pre-command
  live atomicity is claimed. Removing one future field in FNX remains effective
  through later Duplicate/Undo/Redo/cold; other fields and comments remain exact.
  Four original capture-parser failures remain retained. A separately reviewed
  literal-only `fnxColor` parser reevaluates the frozen captures without changing
  expected data or file boundaries; it executes no JavaScript.
  Evidence: `integration-20261007/structural-source-fields-20261007`, including
  `green-broad-run3`, `native-positive-independent-review`,
  `native-source-removal-independent-review`, and
  `native-fixture/refusal-native-independent-aggregate.json`.
  Native type conversion/reparent, arbitrary JavaScript/future grammar, camera
  restoration and complete installed-app journeys are not established. Retained
  rendering remains off. Signing/notarization, authenticated capabilities and
  physical large-page performance remain separate release gates.

- **Source-draft inspector and bounded conflict/Cancel PASS on `a7f16cb245`.**
  The isolated development app is built from the exact validated source. Two
  focused regressions pass, including five GPUI scheduler iterations for the
  mounted source-lock journey; all 1,198 editor tests pass (2 ignored), as do
  workspace formatting and the viewer repository lint/dependency audit.
  Single-node accepted properties stay read-only and visible, including world
  position, size and layout gap; empty/mixed selection receives a plain message.
  Mounted controls retain write guards, draft contents and document/history
  while selections change, and restore editing only after draft discard.
  Four native checkpoints pass: Open, unsaved FNX draft, a controlled same-path
  external rename, and actual Workspace Save → Cancel after the watcher reports
  conflict. Each compares all 7 persisted nodes, 0 assets, 19 files and the exact
  timestamp with its declared state. Cancel preserves the external snapshot
  byte-for-byte. Complete native Copy → owned TextEdit Paste/Save captures retain
  the exact unsaved draft; observed accepted row/child geometry, Gap 8 and source
  lock remain unchanged. Disk readback does not stand in for those live observations.
  Independent final native review passes all four stages and the complete draft
  acquisition evidence. No draft or external change is
  claimed to have been adopted by the canvas; overwrite/discard resolution,
  recovery Undo, multiwindow races and cold conflict recovery remain untested.
  The original `bf0650022c` draft failure remains recorded: saved content and the
  full draft passed, but the internal inspector placeholder blocked three live
  geometry checks, so external write/Cancel were not attempted on that build.
  Evidence: `integration-20261007/source-lock-inspector-validation-run4-20261007`,
  `source-lock-inspector-package-v3-20261007`,
  `native-source-conflict-cancel-v2-20261007/native-cancel-aggregate.json`, its
  `independent-final-cancel-review.json`, and the preserved original
  `native-source-conflict-cancel-20261007`.

- **Fixed-box plain Text edit/history/cold PASS on `a7f16cb245`.**
  Five actual native checkpoints pass: Open, change `FRAME TEXT` to `FRAME EDIT`
  and commit with one Escape followed by Save, single Undo/Save, single
  Redo/Save, and a fresh-process reopening. Every checkpoint checks the full
  10-node document, 1 asset and all 20 project files. The fixed 300 × 48 box,
  font/style and one complete 0..10 style range remain exact; only the declared
  text content and save timestamp may change. Cold reopening matches saved Redo
  exactly, including its timestamp and every project byte. Ordinary autosave
  may persist a committed edit before the explicit Save command; these checks
  do not attribute persistence exclusively to that command.
  The pristine fixture was placed inside a disposable parent Git repository
  initialized before native actions. Its complete Git metadata remains unchanged
  at all five checkpoints, and the project file oracle excludes no paths. This
  is an explicit precondition, not a waiver of the earlier fixture failure:
  that first run preserved the intended text payload but failed its file
  boundary when the desktop writer intentionally initialized Git. Its Undo,
  Redo and cold stages were not attempted. Both records remain immutable.
  Final independent review passes all five captures, their parent metadata and
  the retained earlier failure. This bounded
  ASCII/plain-style journey does not establish rich text, IME, Unicode or
  multiline editing, held-Escape gestures, TextPath, or camera restoration.
  Evidence: `integration-20261007/native-plain-text-escape-v3-20261007`
  (`ready.json`, `seed-result.json`, all five `captures/*/capture.json`,
  `native-text-aggregate.json` and `independent-final-native-review.json`),
  alongside `native-plain-text-escape-20261007/independent-edit-file-boundary-failure-review.json`.

- **Exact FNX numeric preservation PASS on `6eb5758bb2` (merged in PR91).**
  Five focused regressions failed before the correction and pass afterward;
  all 98 FNX tests pass in both default and preserve-order configurations,
  with formatting and required lint/dependency checks. The immutable Spectrum
  reference passes exact node-field/type/f64-bit and sidecar replay for 73,925
  nodes across 23 roots, including signed zero; both emissions match byte-for-byte.
  This verifies the codec boundary, not a fresh native `.fig` import or complete
  project/header/asset-writer roundtrip. The historical 15,278-field numeric
  failure and the first validation run's external artifact-selector refusal
  remain retained. [PR91](https://github.com/jeanc18rlos/fanta-edit/pull/91) merged
  with the exact tested tree and normal hosted Check/Store passes.
  Evidence: `integration-20261007/fnx-exact-small-numbers-continuation-v2/terminal-release-coverage-review.json`
  and `pr91-ci-6eb5758/terminal-release-coverage-review.json`.

- **One real H.264 fixture: ten native playback/trim checkpoints PASS on
  development QA `0f71b3961d` / binary `80316a2101`.** Five checkpoints cover
  ready controls, visibly advancing timestamped frames, Pause, one paused seek,
  Trim Cancel/Save and cold reopening, preserving all 35 nodes, five assets and
  48 files. Five further checkpoints cover a 0.6–2.4-second Trim Apply, single
  Undo/Redo and cold reopening. Only the declared three Video fields, new poster
  asset and timestamp change. The complete 320 × 180 poster RGBA matches the
  independent pre-input 600,000-microsecond reference with zero tolerance; Undo
  retains that asset, and Redo/cold preserve its identity and bytes. All 49 final
  files and six assets, both Git boundaries and declared modes/mtimes match.
  Cold uses explicit Video reselection; no automatic selection restoration is
  claimed. The old a7 0/0 readiness failure, later stale-display uncertainty and
  missing-screenshot-role capture failure remain recorded. The later case uses
  the same MP4/project payload and was already ready before foreground switching;
  no playback-code fix or activation recovery is established. Audio, other
  codecs/VFR, missing/corrupt media and installed-release behavior remain open.
  Evidence: `integration-20261007/native-real-video-a7-20261007/`:
  `play-seek-trim-followup-v1/independent-native-followup-review.json`,
  `trim-commit-capture-v1/independent-native-trim-v2-review.json`, and
  `a7-to-0f-readiness-comparison-v1/comparison.json`.

- **Approved-production account reads and one cold session: bounded PASS.**
  A separate profile using the unchanged development `0f71b3961d` binary and
  `https://api.fantaisa.net` shows an existing authenticated account and loaded
  billing data. A normal Quit/reopen returns that account context and settled
  billing page without another Sign-in action. All 37 saved blank-project file
  records and settings stay exact; independent review covers 68 manifest records
  and 38 recorded actions. This does not establish which sign-in branch ran,
  fresh browser authentication or general Settings navigation: the initial
  Sign-in footer and stale billing display remain recorded. No credential,
  account identifier or financial value is published; no purchase or AI request
  was made. Recovery, AI/credit, live MCP and payment acceptance remain open.
  Evidence: `integration-20261007/production-account-native-review-20261007T123645Z/independent-review.json`
  and `production-api-profile-setup-20261007/native-sign-in-v1/completed-account-session-result.json`.

The combined [PR92](https://github.com/jeanc18rlos/fanta-edit/pull/92) head
`f140915203` passes its own push/PR Check and Store jobs, including 1,204 editor
passes, zero failures and two ignored diagnostics in each normal run. Ordinary
native-video jobs are explicitly skipped; the separate dedicated run passes
3/15/15 tests with traced and untraced playback. Actual push/dedicated and PR
synthetic checkouts share tree `5a49a53962`. PR92 merged at 13:34:34 UTC as
`26429e3453` with that exact tree. PR90 was automatically marked merged/closed
by ancestry two seconds later; its earlier cache-timeout failure remains recorded.
The two added activation tests use FakeSession; they do not diagnose native
readiness failures. Evidence: `integration-20261007/native-real-video-a7-20261007/`
`native-video-ci-combined-main-v1/ci-terminal-review/independent-final-terminal-review.json`
and `pr92-root-merge-20261007/result.json`.
These bounded additions leave five release groups open: unresolved media
behavior and breadth; fresh import and physical performance; installed capability
coverage; Developer ID/notarization/install/update/rollback; and remaining
authenticated-service journeys. Retained rendering stays off by default.

Evidence is archived under `native-inspector-ui-candidate-20261007`,
`native-variant-instance-menu-20261007-v4` (the `scope67-*` and `vdaf-*` records,
plus the retained `native-detach` failure), `native-frame-section-text-20261007` (`scope67-*`), and
`variant-detach-artifact-ownership-20261007` (`green-broad-run2` and
`root-index-green-run1`). These development checks do not validate the final
signed/notarized DMG or native large-page performance. Earlier dated results
and failures follow without being reclassified.

These results apply to the named builds and bounded journeys. Earlier
native journeys in the appendix used `a82f90f87b`. The source-editor candidate
`3e3f275fcd9d721dfdca7de715ccf350166caec6` builds successfully with stable source
fingerprints and binary SHA-256
`68b8f94d837361c6fb689dcd8b572fbbf39e0d055ec54be72b4919fc4d58b38c`;
its source Find, Save/Undo and scoped-layout native checks pass. Its draft
recovery failure is retained below. It includes the scoped-layout correction in
[PR 52](https://github.com/jeanc18rlos/fanta-edit/pull/52), production commit
`e6362d8a1f` with the viewport regression added in `6e97d01eed`.
The later recovery candidate `ac038333e39fffb908093b64951a10847f10316d`, binary
SHA-256 `7c77bfed9fdbc89644be6c6a0d624759d17577fe95ef8725b559a9e0e1ff8e4b`,
passes the partial-source-save/discard/canvas-save journey and strict restart.
The clipboard corrections and complete default automated runner now pass on
`938807f801ac249b5ec7418ef0473eb794a11413`, with unchanged source fingerprints.
Native locked-node guards and three visible refusal paths pass on that build.
Native asset Duplicate and master deletion exposed a separate load-time vector
viewport mutation; their strict failures remain retained. The import-boundary
correction (`1d03cb877b`) and bound inspector values (`a5ab8d436b`) now pass
automated regressions. The corrected `100fe269210ad4f7adea22be10e0199efd7237db`
dev binary (`4d688bd5c27661a07baa4e2d456bc71bd13143264c7d1bf42c6f7f167c2d7fc3`)
passes all 41 declared clipboard checkpoints and four binding Open/Save/cold/theme-return
checkpoints. Later renderer `b1774b9ec4` and instance-text `a3a51b0` changes pass
automated validation. The `a3a51b084b` dev candidate also passes native default-alias,
placed-value and derived instance-text Save/Undo/Redo, hidden-text no-op and
strict reopening of its final saved state. The same binary also passes a bounded
Bitmap Crop Apply/Undo/Redo/restart and real TextPath context-menu text
Save/Undo/Redo/restart. A prior batched double-click/drag moved the bitmap instead
of entering Crop; that failed entry and its exact Undo recovery remain retained.
Group and Boolean drill-in, selected-vector menu entry/Escape, and Video/Audio
properties-menu and inspector-reveal checks, followed by Node Graph/3D/AI/Embed
properties entry and reveal, also preserve the complete saved document and file
bytes. Selected Bitmap/Vector Inspect refusals and a nine-root generic menu also
pass bounded checks. The original missing vector anchors are retained as an
`a3a51b084b` failure; corrected `8a9a33c35e` now shows four anchors immediately
through double-click and actual Edit vector menu entry, with strict unchanged
content/files. Its separate bound-text fixture passes refusal/read-only Content,
alias-only canvas/Design edits, single Undo/Redo, repeat Save and strict reopening.
After a selection/focus reset, post-restart bound refusals also pass with exact
content/files. The earlier targeting attempt remains inconclusive; no original
modifier-state cause or production fix is established. A bounded opt-in click
diagnostic now passes three controls and five-seed mounted checks; the original selection anomaly's native
flags remain unobserved. Its separate inspector
fixture now passes seven bounded mixed-value/edit/Undo/Redo/corner/reopen
checkpoints, but two attempted opacity scrubs failed; the second exposed a
fast-exit gesture gap now corrected in automated tests. On 6 October, the
`ce8db37c34` QA build passes a numeric-readout drag to 0%, Save, one Undo/Save,
one Redo/Save and strict process restart. All ten nodes, five assets and unrelated
content match the declared states; reopening preserves every project byte and
timestamp. A separate icon-targeted attempt produced no edit and remains retained.
The combined editor suite passes 1,171 tests (four ignored), three CLI
tests, lint and formatting on the updated UI pin. These results do not validate the signed
release artifact.

| Gate | Confirmed result | Still required |
| --- | --- | --- |
| Large-project content preservation | **Bounded native PASS:** page visits/Save, image move/Save, single Undo/Save and restart of the saved image Undo state. Instance flow-child move/Save and Undo/Save also pass complete typed comparisons. | **Instance restart INCONCLUSIVE** after possible concurrent input; repeat against its exact saved Undo snapshot. Broader node/layout/visual journeys remain unverified. |
| Scoped layout and external reload | **Bounded native PASS on `3e3f275fcd`:** fixed nested geometry survives outer spacing edits; actual allocation changes reflow; Save and single Undo/Redo pass. External reload automatically persists computed output. All three fixtures reopen identically in a separate same-binary process/fresh profile. | The transient dirty/manual-only checkpoint was not observed before autosave; its strict failure remains retained. This is cold-process reopening, not restarting the original app. Broader layout combinations remain open. |
| Code source editing | **Bounded native PASS:** Find and source Save/repeat Save/single Undo/Redo on `3e3f275fcd`; partial FNX Save with invalid JSON → selected Discard → Canvas Save, repeat Save and strict quit/relaunch on `ac038333e3`. The later instance-text source checkpoint passes 1,147 editor library tests (1 ignored) and viewer lint. The three benchmark CLI tests retain their earlier viewport/binding checkpoint, and the eight-stage run retains its `938807f801` identity. | The `a7f16cb245` four-stage native same-path external rename/Cancel check above now passes. Native recovery Undo, deliberately delayed own-write events, other external-conflict resolution paths and multiwindow source editing remain unverified. The original false-conflict failure is retained; no focus production change is claimed. |
| Clipboard guards and master preservation | **Bounded native PASS on `100fe26921`: all 41 harness checkpoints**, including asset Duplicate/Copy-Paste/Cut-Paste, master/final-variant deletion, repeated component-bundle Paste, Undo/Redo, all seven route restarts and direct/inherited lock guards. Complete typed/asset oracles pass; restarts include exact timestamp/file bytes. Earlier three visible refusal paths retain their `938807f801` evidence. | Original viewport-mutation failures remain retained. Broader dependency, partial-set and appearance combinations are not exhausted. Cross-document Paste remains unsupported; partial-set Cut and unsafe detach cases refuse without mutation. |
| Prototype interaction removal | **Bounded native PASS:** explicit Remove/Save and single Undo/Save on `a82f90f87b`; a fresh matching user-edited baseline passes Save and verified process restart with exact typed content and all 19 files on `ac038333e3`. | The older strict restart failed against a stale baseline after confirmed user position edits and remains retained. Remove/Undo and playback were not repeated on the newer build; broader triggers/overlays remain open. |
| Inspector and per-node actions | **Bounded native PASS on `100fe26921`:** wrapper opacity 40%, bound visibility/radius, explicit-corner summaries, ancestor-pinned 70%/visible/26 values and readable Available labels in One Dark and One Light. Open/Save/cold reopen and a theme roundtrip preserve all 28 nodes and 25 files exactly. Earlier paint, mixed-opacity, Export and per-node checks retain their own provenance. | B02's four separate corner controls and the precedence label were not observed in that earlier fixture; the 7 October static checks above now cover their display. The 8 October Project Alternate B/Undo/cold sample above now covers one mode change; other modes and exhaustive custom-theme checks remain open. The later `a3a51b084b` native fixture passes alias/mode-aware text editing, hidden-text no-op and saved-state restart; direct-bound refusal/read-only Content and alias-only canvas/Design edits now pass the separate `8a9a33c35e` fixture, including Undo/Redo and strict saved-state reopening. A later post-restart refusal check passes after selection/focus reset; the earlier targeting attempt and its unproven cause remain recorded. Seven later `8a9a33c35e` inspector checkpoints pass: mixed 50%/100% → 75%, single Undo/Redo/restoration, corner inspection and exact cold reopen. Both earlier zero-opacity attempts remain failed. The updated UI pin passes four fast-exit regressions and six scrub controls across five seeds. Corrected `ce8db37c34` now passes a numeric-readout scrub to 0%, Save, single Undo/Redo and strict process restart with exact saved bytes. The later icon-targeted no-op is retained separately. The broader per-kind, gesture and keyboard matrix remains incomplete. |
| Large-page responsiveness | Correctness tests and bounded CPU diagnostics pass; Save acceptance hashing improves one measured phase. PR 75 (`f47763b6ae`) adds inclusive per-gesture UI costs; focused tests, five-seed lifecycle, 1,164 editor tests (1 ignored), three CLI tests and lint pass; that checkpoint did not establish native latency. PR 74 (`fed62a3339`) accepts fixed painted ancestors: 389 renderer tests pass (2 ignored); 12 development-CPU Spectrum runs pass 552 exact parity frames and 1,440 paired timings. Fit-all improves, but 100% image performance regresses and the 100% instance case does not improve. At that checkpoint the API was outside live canvas dragging; earlier PR 72 refusal evidence is retained. The first CPU-upload comparison against actual Metal failed (maximum channel difference 28 versus threshold 2), so its timings were rejected. CPU-full/retained match exactly; three Metal-only composition controls match normal Metal on the first fixture. A later test-only GPU matrix matched all 40 frames exactly across five fixtures, with unchanged semantic proof and no timing. The subsequent Metal API and opt-in canvas worker pass the seven API tests and one IOSurface-worker test recorded above; normal rendering remains the default, and native activation/performance acceptance remains open. | Earlier development native full Save was slow; matched complete Save, pointer/frame p50/p95 and memory measurements on the final build are open. Instrumentation is not native input-latency evidence. No result establishes the drag p95 target below 16 ms. |
| Final artifact and services | **All eight default automated stages PASS on `938807f801`**, with stable source fingerprints. Earlier component visual, MCP and distribution evidence retains its own build identity. Integrated main `6ac89e6a21` now passes exact-head hosted Check/Store jobs; native-video was skipped. Direct-download macOS is the selected release target. PR 71 adds mounted-DMG verification: 27 isolated checks pass with a tiny real test DMG; the applied default run passes 26 with that optional check skipped. | Internal ad-hoc-signed optimized DMGs now pass the 8 October verification above; no Developer ID/notarized product DMG has been verified. Remaining native journeys, current visual/private-fixture gates, live services, Developer ID signing, DMG notarization/stapling and clean-account/Mac installation checks remain open. |

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
content outside ordinary or rounded clips. Later alias/mode targeting uses the
placed instance's context, with 18 instance-text tests and the full 1,147-test
editor run passing. Native default/placed aliases and derived text now pass
Save, single Undo/Redo and strict saved-state reopening on `a3a51b084b`; hidden
virtual text stays unedited. True nested instances and virtual TextPath remain separate limitations. Direct
text-binding refusal and retained-draft protection now pass seven focused tests
(including three GPUI cases across five seeds), 45 resolver tests and a full
1,154-test editor run. The subsequent activation-overlay correction passes
1,157 editor tests (one ignored). Native `8a9a33c35e` passes direct-bound
refusals/read-only Content, alias-only canvas and Design edits, Undo/Redo,
repeat Save and strict content/file reopening. A later post-restart refusal
check passes after selection/focus reset; the original inconclusive targeting
attempt remains retained. A mounted entry-sequence regression passes eight
no-modifier configurations across five seeds (40 case executions), with viewer
lint passing and no production input fix. Late-binding draft safety remains
automated-only. Boolean operands also support explicit drill-in
and vector editing with Undo preserving the boolean container. Native Boolean
drill-in samples are recorded in the appendix. On 7 October, `ce8db37c34` also
passes one authored Boolean operand's exact point edit through entry/cancel,
Save, single Undo/Redo, exit and cold reopening: all 35 nodes, five assets,
30 file bytes and timestamps match their declared states. This fixture has two
operands and no imported baked geometry; other Boolean arrangements remain open.
On `ce8db37c34`, one Group-contained vector passes actual
Edit vector menu entry, four visible anchors, an exact point drag/Save and one
Undo/Redo with all 35 nodes and five assets matching the declared states. Its
cold restart preserves all 30 project file bytes and timestamps exactly; the
full hit-guard/input matrix remains incomplete.

## Completed checkpoints

The [dated evidence appendix](RELEASE_EVIDENCE_2026-10-05.md) preserves exact
build identities, automated results, native journeys, performance diagnostics
and original failure records. The status table above summarizes the latest
bounded results. Do not treat an earlier checkpoint's pending or failed result
as a result for a later build.

## Capability-to-test map

Paths below are relative to the repository. The recorded native samples
cover parts of this map; no row implies exhaustive native release-candidate
coverage. The final column highlights the remaining acceptance work.

| Capability | Existing automated evidence to run | Remaining acceptance work |
| --- | --- | --- |
| Create/open/import | `fig_viewer::new_design::tests`, `document::tests`; `fanta-fig-interop` mapping/parser tests | New project and `.fig`/`.fant` import, collision handling, malformed file and missing asset UI; compare representative imported pages with reference renders. |
| Save/autosave/Save As/reopen | `fanta-format` full suite; `fig_viewer::document::tests`, `view::tests`, `view::serialization::tests` (`canvas_session_*`, `save_generation_*`, `a_failing_autosave_is_reported_until_a_save_succeeds`, `save_as_*`) | The 8 October Save As Cancel/refusal/original-target/cold check above passes. Successful Save As to a new destination, write failures and concurrent edits remain; compare complete files, nodes and assets. |
| Source ↔ canvas | `fig_viewer::code_workspace::tests`, `editor_session::tests`; FNX/format source-order tests; PR 52 watcher-output Save/reopen and own-write ownership regressions | Earlier Save/repair/restoration/restart pass on `a82f90f87b`; Find and source Save→Undo/Redo pass on `3e3f275fcd`. The false own-write conflict in that build is retained; partial Save → selected Discard → Canvas Save/repeat Save/strict process restart now passes on `ac038333e3`. Small clean external reload/autosave/cold-process checks pass separately. The `a7f16cb245` native same-path external rename during an unsaved draft and Workspace Cancel now pass the bounded check above. The 8 October external-change/Workspace Discard/cold check above also passes. Recovery Undo, multiple tabs/windows, other external-conflict paths and agent source-follow remain. |
| Pages/layers/structure | Viewer design-panel, structure, layer-context and clipboard tests; `canvas_menu_reorder_entries_execute_and_undo` confirms both ordering entries. Primary menu cases retain a selected boolean operand; mounted asset-subtree menu Duplicate/Cut control passes. | All 41 bounded native clipboard checkpoints pass on `100fe26921`, including image-subtree copy/cut/duplicate, component/variant preservation, lock guards and all route restarts. Original viewport-mutation failures remain retained; broader structure/dependency combinations remain open. Earlier asset-free keyboard Duplicate/Paste/Undo/Redo and Paste restart passed on `9ca127c1be`; bitmap cross-page and visible cross-document refusal passed separately. |
| Navigation/selection/transforms | `fanta-canvas` hit-test/snap tests and `tests/end_to_end.rs`; `fanta-tools::select::tests`, `scale::tests`; viewer toolbar adapter tests | Pan, zoom, nested selection, rapid drag/release, resizing and scaling in a dense imported page. Check focus and pointer capture. |
| Double-click by node | Viewer `canvas_double_click_*`, `mounted_curved_text_path_*`, mounted crop/drill/guard cases and `mounted_instance_entry_from_empty_selection` (eight configurations × five seeds), existing standalone/wrapped text cases; tools `rapid_clicks_drill_once_per_pair_and_do_not_enter_leaf_nodes`, `extending_double_click_toggles_the_container_without_drilling`, `double_click_at_container_resize_handle_still_drills_into_child` | Bounded native Group/Boolean drill-in, selected-vector menu entry/Escape and Video/Audio/Node Graph/3D/AI/Embed menu plus double-click inspector reveal pass with strict unchanged content on `a3a51b084b`. The later `8a9a33c35e` shows four initial anchors through actual menu and double-click routes. Selected Bitmap/Vector Inspect guards and a nine-root generic menu also pass; TextPath Inspect targeting was inconclusive. Complete the [per-kind contract](../fanta/capabilities.md#double-click-behavior-by-node), including other locked/Inspect and mixed-selection combinations, other Boolean arrangements/path editing, feedback and Undo. One `ce8db37c34` Group-contained vector now passes menu entry, authored anchor drag/Save and single Undo/Redo with strict 35-node/five-asset oracles and exact 30-file cold reopening. The same build now passes one authored two-operand Boolean anchor journey through entry/cancel, Save, single Undo/Redo, exit and exact cold reopening. Specialist content authoring is not implied by its properties entry checks. |
| Properties inspector | `fig_viewer::gpui_adapters::design::tests`, `properties_panel::tests`, `properties_ops::tests`, `properties_snapshot::tests`; mounted `view/properties_inspector.rs::layout_tests` checks geometry fields at minimum width and the composed inspector with annotation/measurement lists | Numeric-readout drag out of the field, Save, single Undo/Redo and strict process restart pass on `ce8db37c34`; held-drag cancellation remains open. Native `8a9a33c35e` now passes differing 50%/100% → 75%, single Undo/Redo, restoration and cold reopen, plus bounded corner inspection at 320 px. Two attempts to scrub to zero failed and remain retained. Earlier draft cancellation, invalid-draft switching and Export-label passes retain their own builds. Themes, scrolling/popovers and broader property combinations remain; mounted layout bounds are not a full visual baseline. |
| Drawing/path/region/crop | `fanta-tools` full suite, including `tests/end_to_end.rs` and `ink_oracle.rs`; viewer toolbar adapter tests | One axis-aligned Bitmap Crop now passes native Apply, single Undo/Redo and strict saved-state restart on `a3a51b084b`; Cancel has strict typed-content evidence only. An earlier batched entry moved the bitmap and remains a failed input checkpoint. One `ce8db37c34` Group-contained vector also passes authored anchor drag/Save and single Undo/Redo with strict content/assets and exact cold reopening. The same build also passes one authored two-operand Boolean point journey with exact saved-state reopening. Other tools, rotated/nested crop, masks, different Boolean arrangements/path edits and shortcuts remain open. |
| Text/text on path | `fanta-text`, `fanta-tools::text_path::tests`, renderer text/text-path tests; viewer `text_edit`, `instance_text` (18 cases including placed modes/aliases/visibility and derived geometry) and design adapter tests | Native `a3a51b084b` passes default/placed instance aliases, derived geometry, hidden no-op and saved-state restart. A real curved TextPath also passes canvas **Edit text**, ASCII replacement, single Undo/Redo and strict restart with its curve/style/other content intact. The separate `8a9a33c35e` fixture passes direct-bound refusals/read-only Content, alias-only inline/Design edits, Undo/Redo, repeat Save and strict reopening. Post-restart refusals also pass after selection/focus reset; the earlier targeting ambiguity remains retained. The `a7f16cb245` fixed-box plain Text edit/one-Escape/Save/single Undo/Redo/cold journey above also passes, preserving its one 0..10 style range. The 8 October multiline Command-Return/history/cold check above passes with clipping and single-style limits. Other inline ranges, rich styles, Unicode/IME input, fonts, conversion errors, virtual TextPath and broader exports remain. |
| Layout/paints/effects/rendering | `fanta-doc` layout tests; format omission/complete-geometry roundtrips; complete `fanta-render` library and bitmap/SVG/compose/golden integration suites | The corrected Spectrum image journey passes page visits/Save, an isolated nested image move/Save, Undo/Save and strict restart of the saved Undo state. Instance flow-child move/Undo and one source-spacing Save/repair journey also pass; instance restart remains inconclusive. The separate seven-node draft recovery Save/restart passes on `ac038333e3`. PR 52 now has bounded native fixed-layout/allocation Save/Undo/Redo and clean external reload/autosave passes, with identical cold-process reopen of all three small fixtures. The transient dirty state was not observed before autosave. Also verify visual parity for gradients, masks, booleans, clipping, shadows/blur, blend modes, auto-layout/grid and imported instances under edits. |
| Variables/styles | `fig_viewer::variables_workspace::tests`, `variable_binding::tests`, `agent_surface::tests`; document resolve/render tests | The 8 October Project Alternate B/Undo/cold sample above passes; Redo, rename/delete, aliases and other types/bindings remain. Whole-node resolved/pinned opacity, visibility and radius displays plus exact Open/Save/cold preservation pass on `100fe26921`, with corner-label and optional mode-mutation limits recorded above. One two-mode color binding/unbind/Undo/restart passed on `ee80ec48c6`; header containment/toggle, resolved bound row/picker, read-only alpha explanation, picker Detach/Undo and strict restart passed on `0a0d3cd1c4`. |
| Components/variants | Viewer component-property, variant-set, clipboard and agent tests; `fanta-doc` instance resolution tests; importer overrides tests | Asset-bearing master/final-variant deletion and repeated component-bundle Cut/Paste now pass native Save/Undo/Redo/restart on `100fe26921`; partial-set and unsafe appearance refusals retain separate earlier evidence. Alias/mode-aware text overrides now pass the separate `a3a51b084b` native fixture. Master ↔ instance updates, other typed properties, variant switching and nested components still need broader native checks. |
| Motion/timeline | Viewer `motion_panel`, `motion_edit`, `timeline`, toolbar adapter tests; document/render motion tests | Easing edit/cancel, clip switching, duration and mode changes. Representative playback/ruler seek and keyframe drag/Undo passed on `ee80ec48c6`; time-field typing/seek with retained selection and strict restart passed on `0a0d3cd1c4`. Broader property coverage remains open. |
| Prototypes | `fanta-present`; viewer `prototype_panel`, `prototype_player` and view tests | Native explicit Remove/Save and single Undo/Save pass on `a82f90f87b`; its restart mismatch is retained against a stale baseline after confirmed user position edits. A fresh matching baseline passes Save/strict process restart on `ac038333e3`. Other pointer/key/time triggers, overlays, transitions and safe links remain. Click navigation/Restart/Escape passed on `ee80ec48c6`; detail X retained reaction/card/wire with unchanged Save/restart snapshots on `0a0d3cd1c4`. |
| Comments/review/Dev | Viewer `comments`, `comments_ui`, `view_annotations`, `view_measurements`, `view_dev_mode` and export tests | Pin/reply/resolve, draft preservation, mode transitions, keyboard ownership and read-only protection; no review overlays in artwork exports. |
| Local image/SVG/video/audio | Viewer `generation_media`, `video_playback`, document/view and media tests; renderer live-media tests | Earlier `a3a51b084b` properties/reveal checks remain unchanged. The ten bounded H.264 playback/seek/Trim Cancel and Apply/history/cold checkpoints above now pass on `0f71b3961d`; exact poster and asset persistence are covered for that fixture. The earlier readiness and stale-display observations remain unresolved. The 8 October existing-Audio and corrupt/loose-MP3 refusal checks above pass. Project-Assets MP3 placement/history/cold storage passes, but cold Audio preview for the original and newly placed MP3 remains unaccepted. Other codecs/VFR, missing sources, orientation, audible output and broader installed-app journeys remain. |
| Export | Viewer `export::tests` and inspector export tests; renderer integration suites | Native `8a9a33c35e` Vector and Bitmap exports pass bounded PNG/SVG/PDF, Vector JPG, collision preservation and two simultaneous presets including 2× PNG checks; all 24 project files remain exact. Bitmap JPEG retains its original four-pixel FAIL and separate DCT-flat-block pass. The later quality-95 fix passes the unchanged original hosted regression; latest native retest, broader fidelity/selection batches and visible export-error routes remain. No general JPEG-quality claim. |
| Designer/MCP | Viewer `agent_surface` (including style projection), `live_mcp` and `plan_build` tests; `script/smoke-mcp`; managed-account failure-path regressions in `project::context_server_store` | Final app local stdio/socket connection, real agent tool selection, one undo per successful batch, rollback, source validation and screenshots. Separately verify hosted account-managed MCP discovery/tool/auth/recovery; local editor MCP tests do not cover that service. |
| Generation/recovery | Viewer `generation_workspace`, `generation_journal`, `generation_media`; account/provider CI tests | Signed-in final build with a real provider: submit/poll/save/place, timeout/retry, restart, sign-out/account switch and exactly-once recovery. Mock responses do not prove production availability. |
| App shell/distribution/accounts | Existing `Check` jobs: sidebar, path prompt, agent toggle, Git, auth, Store restrictions; `script/test-macos-release` preflight/manifest/trust-routing cases and release workflow `script/verify-macos-dmg` | The bounded production existing-account/billing read and cold-session check above passes; fresh sign-in, session recovery, billing/payment/restore and backend release compatibility remain. Menu/keyboard discovery, installed clean-profile launch, quarantine, signature/notarization, Keychain and sandbox file access also remain. |
| Large-page performance | Import scaling, renderer cache/culling, ignored latency/acceptance benchmarks and profiling examples; matched synthetic acceptance median 546.673→292.797 ms; worker and inclusive UI-stage gesture tests; PR 74 retained API CPU parity/timings (fit-all benefit and 100% image regression at that earlier pre-worker checkpoint); current opt-in Metal API/worker correctness (seven actual-Metal API tests plus one IOSurface-worker test) | Full native Save and gesture timings on Spectrum remain required; acceptance-only development timing and mounted instrumentation tests do not establish application latency. Measure frame p50/p95/max and memory; the drag p95 target below 16 ms is unproven. |

## Next bounded native checks

These areas retain bounded follow-up work; completed parts are identified
below. Use disposable fixtures without replacing their existing evidence.
Record the exact candidate binary, initial typed document and asset hashes,
native inputs and final comparison.

| Priority | Fixture and bounded journey | Required evidence and limit |
| --- | --- | --- |
| 1 — Inspector gestures | Preserve `/tmp/fanta-inspector-native-20261005` and its immutable baseline. Differing-value keyboard edit/Undo/Redo and cold reopen now pass on `8a9a33c35e`; the corrected `ce8db37c34` numeric-readout scrub now passes Save, single Undo/Redo and exact process restart. Next cover held-drag cancellation and broader field combinations. | The first older zero-opacity attempt began on the icon; the second began on the numeric body and still did not scrub. Preserve both failures and the 6 October icon-targeted no-op. Start numeric drags on the value readout; its adjacent icon is rendered separately. All ten nodes/five assets and unrelated files must match the predeclared oracle; atomic native drags do not establish Escape while held. Broader corner/property/theme checks remain. |
| 2 — Remaining component boundaries | The 41 asset clipboard/structure checkpoints on `100fe26921` are complete. Preserve those fixtures and their exact baselines; future sampling should target partial-set/dependency/appearance combinations outside that harness. | Do not repeat completed guards or seven route restarts as pending work. Unsupported Cut/Detach must refuse visibly before mutation; supported edits must preserve full typed content, references and asset bytes. Direct text bindings, true nested instance editing and virtual TextPath remain separate limitations. |
| 3 — Large-page save and drag | Preserve `/tmp/fanta-spectrum-layout-acceptance-20261005`, its post-image-Undo state, the pristine reconstruction and the earlier failure project. The page-visit and image-filled frame (import index 2110) journey passed on `a82f90f87b`. Instance index 4358 move/Save and one Undo/Save also passed against the post-image-restart baseline. Next repeat its inconclusive strict saved-Undo restart. PR 52's three small layout/reload fixtures now pass native checks and separate cold-process reopening; this does not replace dense-page acceptance. | The image journey retained parent/order and all unrelated geometry/assets, with only exact predicted revision/timestamp changes; its saved Undo state survived restart strictly. The instance check allowed only predicted local Y1.5→2 and exact revisions2/4 plus timestamp; restart must have zero differences against its saved Undo state. Do not waive geometry/numeric differences or reuse the earlier offline conversion's 15,278 normalizations as exceptions. Reconfirm persisted target IDs; fresh-import IDs are not stable. Quiet-machine timings and whole-page visual fidelity remain separate gates. |
| 4 — Variables, Motion and Prototype | Preserve `/tmp/fanta-release-variables-prototype-20261005`, the original source-prototype evidence and `/tmp/fanta-release-prototype-current-baseline-20261005`. Next sample easing cancellation, another prototype trigger and overlays, then aliases or a different binding type. | Earlier two-mode, Motion and prototype samples passed on `ee80ec48c6`; bound controls, title, time-field seek and detail X passed on `0a0d3cd1c4`. Explicit Remove/Undo passed on `a82f90f87b`; matching user-edited baseline Save/restart now passes on `ac038333e3`. Its earlier stale-baseline failure remains retained. Compare complete typed content/assets; none proves all triggers, easing, aliases or variants. |
| 5 — Source editing and recovery | Preserve `/tmp/fanta-release-source-layout-20261005` and `/tmp/fanta-recovery-source-layout-20261005` with their evidence. Find/Save/Undo/Redo pass on `3e3f275fcd`; partial FNX Save, invalid JSON Discard, Canvas Save/repeat Save and strict restart pass on `ac038333e3`. A controlled same-path rename during an unsaved FNX draft and actual Workspace Cancel now pass on `a7f16cb245`. The 8 October explicit external-change/Workspace Discard/cold sample above also passes. Next test recovery Undo, other conflict decisions and tabs/windows during async Save. | Retain canvas lock and visible errors; Cancel must preserve the external bytes, last valid scene and draft. Require exact complete readback and unchanged unrelated files/assets. Delayed owned-event and genuine external-conflict cases pass mounted tests; they are not independent native race-injection evidence. Clean external reload/autosave/cold-process reopening passes separately on the small PR 52 fixture. |

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
restrictions. The separate packaging preflight suite fails when a test runner
fails even through log capture; its optional tiny-DMG check exercises actual
read-only mounting and cleanup. The direct-download release workflow verifies
the mounted produced app against staging, then checks its signature and, when
notarization is required, its ticket and Gatekeeper assessment. The signed
product artifact itself remains unverified. Verify both hosted jobs for the exact
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
