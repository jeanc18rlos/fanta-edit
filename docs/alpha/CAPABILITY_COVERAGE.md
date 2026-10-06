# Release capability coverage — 7 October 2026

**Release readiness is not established.** The target window is Monday
**5 October through Sunday 11 October 2026**, Europe/Madrid. The immediate user
reports are degraded inspector UI, double-clicks with no useful node action,
failures or missing content, and slow editing on large pages. Treat those as
release blockers until reproduced and verified on the candidate build.

The selected distribution is a **direct-download Mac app**. Final acceptance
must use the signed, notarized DMG and the app installed from that exact DMG.
The isolated development QA bundles and Mac App Store checks are separate
evidence; neither establishes direct-download installation readiness.

The integrated main commit `6ac89e6a2193603d45f7ce517d52493f2386884f` now
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
| Code source editing | **Bounded native PASS:** Find and source Save/repeat Save/single Undo/Redo on `3e3f275fcd`; partial FNX Save with invalid JSON → selected Discard → Canvas Save, repeat Save and strict quit/relaunch on `ac038333e3`. The later instance-text source checkpoint passes 1,147 editor library tests (1 ignored) and viewer lint. The three benchmark CLI tests retain their earlier viewport/binding checkpoint, and the eight-stage run retains its `938807f801` identity. | Native recovery Undo, deliberately delayed own-write events, genuine concurrent external changes and multiwindow source editing remain unverified. The original false-conflict failure is retained; no focus production change is claimed. |
| Clipboard guards and master preservation | **Bounded native PASS on `100fe26921`: all 41 harness checkpoints**, including asset Duplicate/Copy-Paste/Cut-Paste, master/final-variant deletion, repeated component-bundle Paste, Undo/Redo, all seven route restarts and direct/inherited lock guards. Complete typed/asset oracles pass; restarts include exact timestamp/file bytes. Earlier three visible refusal paths retain their `938807f801` evidence. | Original viewport-mutation failures remain retained. Broader dependency, partial-set and appearance combinations are not exhausted. Cross-document Paste remains unsupported; partial-set Cut and unsafe detach cases refuse without mutation. |
| Prototype interaction removal | **Bounded native PASS:** explicit Remove/Save and single Undo/Save on `a82f90f87b`; a fresh matching user-edited baseline passes Save and verified process restart with exact typed content and all 19 files on `ac038333e3`. | The older strict restart failed against a stale baseline after confirmed user position edits and remains retained. Remove/Undo and playback were not repeated on the newer build; broader triggers/overlays remain open. |
| Inspector and per-node actions | **Bounded native PASS on `100fe26921`:** wrapper opacity 40%, bound visibility/radius, explicit-corner summaries, ancestor-pinned 70%/visible/26 values and readable Available labels in One Dark and One Light. Open/Save/cold reopen and a theme roundtrip preserve all 28 nodes and 25 files exactly. Earlier paint, mixed-opacity, Export and per-node checks retain their own provenance. | B02's four separate corner controls and the precedence label were not observed; no native mode-change/Undo or exhaustive custom-theme check in this fixture. The later `a3a51b084b` native fixture passes alias/mode-aware text editing, hidden-text no-op and saved-state restart; direct-bound refusal/read-only Content and alias-only canvas/Design edits now pass the separate `8a9a33c35e` fixture, including Undo/Redo and strict saved-state reopening. A later post-restart refusal check passes after selection/focus reset; the earlier targeting attempt and its unproven cause remain recorded. Seven later `8a9a33c35e` inspector checkpoints pass: mixed 50%/100% → 75%, single Undo/Redo/restoration, corner inspection and exact cold reopen. Both earlier zero-opacity attempts remain failed. The updated UI pin passes four fast-exit regressions and six scrub controls across five seeds. Corrected `ce8db37c34` now passes a numeric-readout scrub to 0%, Save, single Undo/Redo and strict process restart with exact saved bytes. The later icon-targeted no-op is retained separately. The broader per-kind, gesture and keyboard matrix remains incomplete. |
| Large-page responsiveness | Correctness tests and bounded CPU diagnostics pass; Save acceptance hashing improves one measured phase. PR 75 (`f47763b6ae`) adds inclusive per-gesture UI costs; focused tests, five-seed lifecycle, 1,164 editor tests (1 ignored), three CLI tests and lint pass; native log capture is pending. PR 74 (`fed62a3339`) accepts fixed painted ancestors: 389 renderer tests pass (2 ignored); 12 development-CPU Spectrum runs pass 552 exact parity frames and 1,440 paired timings. Fit-all improves, but 100% image performance regresses and the 100% instance case does not improve. The API remains outside live canvas dragging; earlier PR 72 refusal evidence is retained. The first actual-Metal comparison fails (maximum channel difference 28 versus threshold 2), so no timings are accepted. CPU-full/retained match exactly; three Metal-only composition controls match normal Metal on the first fixture. A separate test-only GPU matrix now matches all 40 frames exactly across five fixtures, with unchanged semantic proof and no timing. Production backend ownership/lifecycle and live activation remain open. | Native full Save remains slow in the development candidate; matched complete Save, pointer/frame p50/p95 and memory measurements on the final build are open. Instrumentation is not native input-latency evidence. No result establishes the drag p95 target below 16 ms. |
| Final artifact and services | **All eight default automated stages PASS on `938807f801`**, with stable source fingerprints. Earlier component visual, MCP and distribution evidence retains its own build identity. Integrated main `6ac89e6a21` now passes exact-head hosted Check/Store jobs; native-video was skipped. Direct-download macOS is the selected release target. PR 71 adds mounted-DMG verification: 27 isolated checks pass with a tiny real test DMG; the applied default run passes 26 with that optional check skipped. | No signed product DMG has been verified. Remaining native journeys, current visual/private-fixture gates, live services, Developer ID signing, DMG notarization/stapling and clean-account/Mac installation checks remain open. |

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
| Save/autosave/Save As/reopen | `fanta-format` full suite; `fig_viewer::document::tests`, `view::tests`, `view::serialization::tests` (`canvas_session_*`, `save_generation_*`, `a_failing_autosave_is_reported_until_a_save_succeeds`, `save_as_*`) | Verify file hashes and node/assets inventory across edit, cancel, save, restart and Save As. Inject write failures and concurrent edits; original content must survive. |
| Source ↔ canvas | `fig_viewer::code_workspace::tests`, `editor_session::tests`; FNX/format source-order tests; PR 52 watcher-output Save/reopen and own-write ownership regressions | Earlier Save/repair/restoration/restart pass on `a82f90f87b`; Find and source Save→Undo/Redo pass on `3e3f275fcd`. The false own-write conflict in that build is retained; partial Save → selected Discard → Canvas Save/repeat Save/strict process restart now passes on `ac038333e3`. Small clean external reload/autosave/cold-process checks pass separately. Recovery Undo, multiple tabs/windows, concurrent external edits and agent source-follow remain. |
| Pages/layers/structure | Viewer design-panel, structure, layer-context and clipboard tests; `canvas_menu_reorder_entries_execute_and_undo` confirms both ordering entries. Primary menu cases retain a selected boolean operand; mounted asset-subtree menu Duplicate/Cut control passes. | All 41 bounded native clipboard checkpoints pass on `100fe26921`, including image-subtree copy/cut/duplicate, component/variant preservation, lock guards and all route restarts. Original viewport-mutation failures remain retained; broader structure/dependency combinations remain open. Earlier asset-free keyboard Duplicate/Paste/Undo/Redo and Paste restart passed on `9ca127c1be`; bitmap cross-page and visible cross-document refusal passed separately. |
| Navigation/selection/transforms | `fanta-canvas` hit-test/snap tests and `tests/end_to_end.rs`; `fanta-tools::select::tests`, `scale::tests`; viewer toolbar adapter tests | Pan, zoom, nested selection, rapid drag/release, resizing and scaling in a dense imported page. Check focus and pointer capture. |
| Double-click by node | Viewer `canvas_double_click_*`, `mounted_curved_text_path_*`, mounted crop/drill/guard cases and `mounted_instance_entry_from_empty_selection` (eight configurations × five seeds), existing standalone/wrapped text cases; tools `rapid_clicks_drill_once_per_pair_and_do_not_enter_leaf_nodes`, `extending_double_click_toggles_the_container_without_drilling`, `double_click_at_container_resize_handle_still_drills_into_child` | Bounded native Group/Boolean drill-in, selected-vector menu entry/Escape and Video/Audio/Node Graph/3D/AI/Embed menu plus double-click inspector reveal pass with strict unchanged content on `a3a51b084b`. The later `8a9a33c35e` shows four initial anchors through actual menu and double-click routes. Selected Bitmap/Vector Inspect guards and a nine-root generic menu also pass; TextPath Inspect targeting was inconclusive. Complete the [per-kind contract](../fanta/capabilities.md#double-click-behavior-by-node), including other locked/Inspect and mixed-selection combinations, other Boolean arrangements/path editing, feedback and Undo. One `ce8db37c34` Group-contained vector now passes menu entry, authored anchor drag/Save and single Undo/Redo with strict 35-node/five-asset oracles and exact 30-file cold reopening. The same build now passes one authored two-operand Boolean anchor journey through entry/cancel, Save, single Undo/Redo, exit and exact cold reopening. Specialist content authoring is not implied by its properties entry checks. |
| Properties inspector | `fig_viewer::gpui_adapters::design::tests`, `properties_panel::tests`, `properties_ops::tests`, `properties_snapshot::tests`; mounted `view/properties_inspector.rs::layout_tests` checks geometry fields at minimum width and the composed inspector with annotation/measurement lists | Numeric-readout drag out of the field, Save, single Undo/Redo and strict process restart pass on `ce8db37c34`; held-drag cancellation remains open. Native `8a9a33c35e` now passes differing 50%/100% → 75%, single Undo/Redo, restoration and cold reopen, plus bounded corner inspection at 320 px. Two attempts to scrub to zero failed and remain retained. Earlier draft cancellation, invalid-draft switching and Export-label passes retain their own builds. Themes, scrolling/popovers and broader property combinations remain; mounted layout bounds are not a full visual baseline. |
| Drawing/path/region/crop | `fanta-tools` full suite, including `tests/end_to_end.rs` and `ink_oracle.rs`; viewer toolbar adapter tests | One axis-aligned Bitmap Crop now passes native Apply, single Undo/Redo and strict saved-state restart on `a3a51b084b`; Cancel has strict typed-content evidence only. An earlier batched entry moved the bitmap and remains a failed input checkpoint. One `ce8db37c34` Group-contained vector also passes authored anchor drag/Save and single Undo/Redo with strict content/assets and exact cold reopening. The same build also passes one authored two-operand Boolean point journey with exact saved-state reopening. Other tools, rotated/nested crop, masks, different Boolean arrangements/path edits and shortcuts remain open. |
| Text/text on path | `fanta-text`, `fanta-tools::text_path::tests`, renderer text/text-path tests; viewer `text_edit`, `instance_text` (18 cases including placed modes/aliases/visibility and derived geometry) and design adapter tests | Native `a3a51b084b` passes default/placed instance aliases, derived geometry, hidden no-op and saved-state restart. A real curved TextPath also passes canvas **Edit text**, ASCII replacement, single Undo/Redo and strict restart with its curve/style/other content intact. The separate `8a9a33c35e` fixture passes direct-bound refusals/read-only Content, alias-only inline/Design edits, Undo/Redo, repeat Save and strict reopening. Post-restart refusals also pass after selection/focus reset; the earlier targeting ambiguity remains retained. Inline ranges, rich styles, multiline/Unicode/IME input, fonts, conversion errors, virtual TextPath and broader exports remain. |
| Layout/paints/effects/rendering | `fanta-doc` layout tests; format omission/complete-geometry roundtrips; complete `fanta-render` library and bitmap/SVG/compose/golden integration suites | The corrected Spectrum image journey passes page visits/Save, an isolated nested image move/Save, Undo/Save and strict restart of the saved Undo state. Instance flow-child move/Undo and one source-spacing Save/repair journey also pass; instance restart remains inconclusive. The separate seven-node draft recovery Save/restart passes on `ac038333e3`. PR 52 now has bounded native fixed-layout/allocation Save/Undo/Redo and clean external reload/autosave passes, with identical cold-process reopen of all three small fixtures. The transient dirty state was not observed before autosave. Also verify visual parity for gradients, masks, booleans, clipping, shadows/blur, blend modes, auto-layout/grid and imported instances under edits. |
| Variables/styles | `fig_viewer::variables_workspace::tests`, `variable_binding::tests`, `agent_surface::tests`; document resolve/render tests | Rename/delete, aliases and other types/bindings. Whole-node resolved/pinned opacity, visibility and radius displays plus exact Open/Save/cold preservation pass on `100fe26921`, with corner-label and optional mode-mutation limits recorded above. One two-mode color binding/unbind/Undo/restart passed on `ee80ec48c6`; header containment/toggle, resolved bound row/picker, read-only alpha explanation, picker Detach/Undo and strict restart passed on `0a0d3cd1c4`. |
| Components/variants | Viewer component-property, variant-set, clipboard and agent tests; `fanta-doc` instance resolution tests; importer overrides tests | Asset-bearing master/final-variant deletion and repeated component-bundle Cut/Paste now pass native Save/Undo/Redo/restart on `100fe26921`; partial-set and unsafe appearance refusals retain separate earlier evidence. Alias/mode-aware text overrides now pass the separate `a3a51b084b` native fixture. Master ↔ instance updates, other typed properties, variant switching and nested components still need broader native checks. |
| Motion/timeline | Viewer `motion_panel`, `motion_edit`, `timeline`, toolbar adapter tests; document/render motion tests | Easing edit/cancel, clip switching, duration and mode changes. Representative playback/ruler seek and keyframe drag/Undo passed on `ee80ec48c6`; time-field typing/seek with retained selection and strict restart passed on `0a0d3cd1c4`. Broader property coverage remains open. |
| Prototypes | `fanta-present`; viewer `prototype_panel`, `prototype_player` and view tests | Native explicit Remove/Save and single Undo/Save pass on `a82f90f87b`; its restart mismatch is retained against a stale baseline after confirmed user position edits. A fresh matching baseline passes Save/strict process restart on `ac038333e3`. Other pointer/key/time triggers, overlays, transitions and safe links remain. Click navigation/Restart/Escape passed on `ee80ec48c6`; detail X retained reaction/card/wire with unchanged Save/restart snapshots on `0a0d3cd1c4`. |
| Comments/review/Dev | Viewer `comments`, `comments_ui`, `view_annotations`, `view_measurements`, `view_dev_mode` and export tests | Pin/reply/resolve, draft preservation, mode transitions, keyboard ownership and read-only protection; no review overlays in artwork exports. |
| Local image/SVG/video/audio | Viewer `generation_media`, `video_playback`, document/view and media tests; renderer live-media tests | Video/Audio properties menu entry, type/geometry/Appearance/Effects/Export sections and isolated double-click inspector reveal pass on `a3a51b084b`, without playback or document changes. Place/play/seek/trim/replay, corrupted files, missing source, poster/orientation, audible output and saved asset bytes after restart remain. |
| Export | Viewer `export::tests` and inspector export tests; renderer integration suites | Native `8a9a33c35e` Vector and Bitmap exports pass bounded PNG/SVG/PDF, Vector JPG, collision preservation and two simultaneous presets including 2× PNG checks; all 24 project files remain exact. Bitmap JPEG retains its original four-pixel FAIL; a separate DCT-flat-block oracle passes without raising tolerance. Broader fidelity/selection batches and visible export-error routes remain; no general JPEG-quality claim. |
| Designer/MCP | Viewer `agent_surface` (including style projection), `live_mcp` and `plan_build` tests; `script/smoke-mcp` | Final app stdio/socket connection, real agent tool selection, one undo per successful batch, rollback on failure, source validation and screenshot inspection. |
| Generation/recovery | Viewer `generation_workspace`, `generation_journal`, `generation_media`; account/provider CI tests | Signed-in final build with a real provider: submit/poll/save/place, timeout/retry, restart, sign-out/account switch and exactly-once recovery. Mock responses do not prove production availability. |
| App shell/distribution/accounts | Existing `Check` jobs: sidebar, path prompt, agent toggle, Git, auth, Store restrictions; `script/test-macos-release` preflight/manifest/trust-routing cases and release workflow `script/verify-macos-dmg` | Menu/keyboard discovery, clean-profile launch, install/quarantine, signature/notarization, Keychain, sandbox file access, billing/restore and backend release compatibility. |
| Large-page performance | Import scaling, renderer cache/culling, ignored latency/acceptance benchmarks and profiling examples; matched synthetic acceptance median 546.673→292.797 ms; worker and inclusive UI-stage gesture tests; PR 74 retained API CPU parity/timings (fit-all benefit, 100% image regression, no live integration) | Full native Save and gesture timings on Spectrum remain required; acceptance-only development timing and mounted instrumentation tests do not establish application latency. Measure frame p50/p95/max and memory; the drag p95 target below 16 ms is unproven. |

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
| 5 — Source editing and recovery | Preserve `/tmp/fanta-release-source-layout-20261005` and `/tmp/fanta-recovery-source-layout-20261005` with their evidence. Find/Save/Undo/Redo pass on `3e3f275fcd`; partial FNX Save, invalid JSON Discard, Canvas Save/repeat Save and strict restart pass on `ac038333e3`. Next test recovery Undo and a genuine external same-path edit during a pending source draft, including Cancel, then tabs/windows during async Save. | Retain canvas lock and visible errors; Cancel must preserve the external bytes, last valid scene and draft. Require exact complete readback and unchanged unrelated files/assets. Delayed owned-event and genuine external-conflict cases pass mounted tests; they are not independent native race-injection evidence. Clean external reload/autosave/cold-process reopening passes separately on the small PR 52 fixture. |

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
