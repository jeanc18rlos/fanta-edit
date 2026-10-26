# Fanta capability guide

For installation and a first workflow, use the [current Public Alpha guide](../alpha/PUBLIC_ALPHA.md), [ten-minute exercise and demos](../alpha/DEMO_GUIDE.md), and [agent quickstart](agent-quickstart.md). The exercise includes reviewed footage of the accepted signed Build45 app, original sample files, screenshots and silent action GIFs. Earlier native QA images retain their original provenance. The signed package has passed notarization, stapling and tested installation, save/reopen and same-build replacement checks. Public availability remains on hold pending website review and publication checks. [Exact package identity and acceptance scope](../alpha/PUBLIC_ALPHA.md#accepted-package-and-publication-status).

Source inventory updated **19 October 2026**; bounded native evidence updated
**11 October 2026**. Acceptance evidence below is dated.
This describes the current editor implementation; release verification is tracked
separately in
[the capability coverage report](../alpha/CAPABILITY_COVERAGE.md). Earlier alpha
reports describe their dated builds and can contain controls that have since
changed. A feature listed here is not a claim that its complete installed-app
journey has passed.

## V2 implementation additions

AI Review and Ultrareview prepare a manual-send draft in the Review profile.
Agents can create anchored findings with severity, category and a review run id;
the pin follows its layer and falls back to its saved position if the layer is
deleted. A suggested design-op fix is applied explicitly from its thread as one
undo step. Reviews do not edit the design automatically.

Variables include enum options and image assets. Prototype actions include
conditional branches, numeric adjustments, boolean toggles and enum cycling.
Motion clips store frame rate, loop mode, scope and transform origins. The
Design menu exports the active clip as Motion JSON, Figma Motion JSON, a Claude
Motion Stage package, an MP4 video, a GIF or PNG frames. Figma Motion JSON lists
unsupported Figma tracks as diagnostics and samples physical springs at the
clip's frame rate. The JSON contains Fanta node ids: a Figma plugin must map them to destination
nodes with the corresponding resting placement before applying [manual tracks](https://developers.figma.com/docs/plugins/api/properties/nodes-applymanualkeyframetrack/).
Fanta renders the MP4, GIF and PNG frames itself, so they match the timeline.
The MP4 needs `ffmpeg` on `PATH` or at `FANTA_FFMPEG`; without it the export
writes the video's frames and the command that encodes them. The Stage package
is a folder whose `index.html` is a `<Stage>` project for Claude Motion and
[claude-video-export](https://github.com/dawoodtrumboo/claude-video-export), and
whose `preview.html` plays offline. Its animated layers are bitmaps moved by a
port of Fanta's keyframe evaluator, and its `README.md` lists what it cannot
represent. Trigger inputs and a state-machine view are deferred.

Draw includes joining, closing and simplifying vector paths, segment bending,
path reversal, breaking at an anchor and curved-anchor deletion. The
[website exporter](website-export.md) packages a saved design with an explicitly
supplied Fanta-JS runtime. Signed updates have settings, menu and About controls;
release setup and end-to-end installation still require a published signed feed.
These source additions have not been exercised in the installed app.

## Projects, pages and source

Create a design from **New Design**, open a Fanta project folder, or import a
`.fig` file or `.fant` snapshot. A saved project contains `fanta.json`, page and
component FNX source, identity sidecars, metadata and assets. See
[project-schema.md](project-schema.md) for the layout.

Keep the manifest, source, identity sidecars and referenced assets together when
copying a project. A received folder can open in Restricted Mode if its `.fanta`
settings or Git configuration could run commands. Review its origin and trust
prompt; do not remove its Git history or enable global trust just to suppress
the prompt. Save As refuses occupied destinations. File reconciliation detects
conflicts, but cannot make a concurrent external write and rename atomic; see
[editing and recovery](project-schema.md#editing-and-recovery) and
[current format limits](project-schema.md#current-limits).

- Figma imports warn when known layer structures are flattened or omitted.
  Supported Boolean operations retain editable operands and their imported
  artwork. Changing an operand or operation recomputes the geometry; Undo can
  restore the imported appearance. Unsupported operations or unavailable
  operands fall back to a baked vector with an explicit import warning. The
  original `.fig` remains unchanged. Warnings do not cover every possible
  rendering or component-override difference.
- Use the left panel to find and switch pages, rename, reorder, duplicate or
  delete pages, select layers, expand nesting, and drag layers into a different
  parent or position. Page roots and component recursion have edit guards.
- Canvas edits support Undo/Redo and debounced autosave. Active canvas text can
  be saved directly; focused numeric-input behavior is qualified below.
  Autosave can persist an edit while a close prompt is
  open; **Don't Save** does not roll back changes already written. Save As creates
  a separate project and rejects occupied or unsafe destinations. Session restore
  records project and page/component scope, but build39 failed closed-tab
  membership: normal Quit and a cold launch without manual Open restored two
  design tabs after all had been closed. Saved project bytes remained exact.
  Subsequent hosted and bounded build42 checks are recorded below; broader
  scoped restore remains unaccepted.
  Unsaved canvas or source content still requires the normal close/save decision.
  A bounded build37 Save/close/warm-reopen journey succeeds, but the original
  reported inert Save/Discard incident is not reproduced or explained. In a
  separate build37 test with writes denied, CmdS displayed a permission error.
  Save in the close prompt retained the dirty draft and logged an error without
  a separate close-error dialog; retrying the prompt and choosing Don't Save
  returned to welcome. Restoring permissions and reopening showed the original
  saved text. In a later check, restoring writes allowed retry Save to finish
  and the new text to reopen; autosave had already persisted it before the
  choice. Active text CmdS and clean Quit/cold reopening also pass on that one
  owned fixture, without proving rollback or the original incident's cause.
  A separate build37 check found that a focused X-field draft was not committed
  by CmdS and was lost on window Close without a prompt. Pressing Enter before
  Save committed the value on that build; this historical failure remains
  recorded. Later bounded build39 checks on an owned fixture passed focused
  numeric CmdS without Enter and valid focused-input Close → Save. Invalid
  input was retained when Save was rejected; Close Without Saving then closed
  the tab, and warm reopening showed the last saved value. Build41 repeats these
  bounded focused-property Save/close results on an owned Text fixture, including
  invalid-input Cancel/Discard and valid Close Save restoring the original FNX.
  These checks do not explain the original Spectrum incident or prove native
  acceptance of later source changes. Both canonical and push hosted Check runs for `37c4073d` on
  11 October pass all 66 required names, including all nine property-draft cases
  for invalid input and shared views. Earlier complete-Check failures remain
  recorded; those exact source checks are green. Hosted success and bounded native
  journeys still do not establish full release readiness. See the
  [acceptance record](../alpha/CAPABILITY_COVERAGE.md), which
  retains the [build37 failure](../alpha/CAPABILITY_COVERAGE.md#current37-native-checks--11-october).
- A separate build41 write-failure check on an owned Text project retains the
  dirty draft on denied Save and close errors. Cancel keeps the draft; closing
  without saving removes the owner tab and its warning. Restoring write access
  permits explicit retry; Undo and Save restore authored content. **Alert polish
  remains open:** manual Save and close alerts expose long internal paths, and
  the tab-close error uses an `untitled` label. Explicit Save All retained the
  draft but showed no action-level dialog; a later autosave notice is separate
  feedback. Concise, named errors and a Save All dialog are now in source;
  their recorded `29890b774` hosted regressions pass. Later build43 native
  samples below verify bounded manual/tab/window error presentation; broader
  failure routes remain pending. These owned fault checks do not establish
  atomic multi-document Save or the original Spectrum incident's cause.
- A later 11 October update records both `a51d57c2` hosted Check runs as failed;
  the earlier `37c4073d` success above remains tied to that source. Subsequent
  `29890b774` results are listed below. Its captured explicit-Open regression
  reaches successful read-failure recovery but fails when repeated Open after
  manifest recreation adds a duplicate tab. The later `ab9faf717` per-view
  entry-identity regression passes in both hosted runs; no matching installed-app
  recreation behavior is claimed. See the
  [dated terminal update](../alpha/CAPABILITY_COVERAGE.md#terminal-a51d-checks-and-captured-open-successor--11-october).
- **Later 11 October acceptance:** internal build42, exact `a51d57c2`, passes
  bounded Spectrum focused Save/Cancel/Discard with full pre-choice comparisons,
  warm reopening and authored restoration. Closing all tabs then a cold launch
  restores zero tabs; explicit Open and repeat Open retain one. Sampled
  light/dark inspectors fit at 320/400 points. Native Crop Apply/Undo/Redo restores
  the document and all six assets; only the restored state is reopened. Later
  disabled-Apply/drag/Cancel, ellipse point/history, mixed-menu, invalid-X
  rejection/Close Without Saving and locked-leaf canvas samples pass within their stated
  scope. Earlier Boolean entry failures remain unexplained despite later traced
  success. Merely focusing TextPath X now reproduces a shifted, dirty preview;
  Escape restores it and saved files remain unchanged. A source repair prevents
  untouched fields from changing values and preserves the precision of real
  position edits. The `ab9faf717` host fractional and transformed-parent tests
  pass. The original dependency untouched-blur failure remains recorded; its
  test-only window activation successor `3e78` now passes all nine required cases.
  Host86c pins that revision but its new Crop diagnostic fails test compilation.
  Later build44 ab9/a795 passes bounded fractional and transformed-parent native
  checks below; it does not validate fa47 or every transform/font case.
  Both published `29890b774` Check runs pass 72/77 and fail five runtime cases.
  Matching lint and Skia source checks pass separately. Build42 does not accept later Save All/error feedback,
  captured-Open or the prepared identity/test corrections. See the
  [dated successor record](../alpha/CAPABILITY_COVERAGE.md#build42-native-samples-and-29890b-test-successor--11-october).
- **Build43 bounded follow-up:** the internal package is exact `1f838bf20`.
  An owned ordinary Text integer field passes focused CmdS, Save All, close Save
  and warm reopening. Denied-write alerts give readable permission guidance;
  tab-close errors name the design file and retain Cancel/Close Without Saving,
  while window-close errors explain that the window remains open. Cancel retains
  the dirty X48 draft; retry Save after restoring permissions persists it without
  retyping. Discard closes the tab, and final authored bytes are restored apart
  from normal `metadata.modified_at`. This does not establish multi-document Save
  All, fractional TextPath, cold restart or later-source acceptance. Sampled
  Text/Frame/Section/Image entry routes work within the recorded scope. A later
  batch adds TextPath text Save/Undo, Group drill and Boolean operand entry with
  four handles, without an anchor edit. Two panes of one design pass later-pane
  focused integer Save All and earlier-pane Undo/Save All restoration; this does
  not establish multi-document failure ordering or shared Discard. Crop
  Cancel clears a pending preview but keeps the tool active; a subsequent Cancel
  or Escape exits it. This is separate from the three failing hosted Crop cases.
  See the [current scoped results](../alpha/CAPABILITY_COVERAGE.md#published-86c-checks-and-rounded-clip-successor--11-october).
- **Rounded-corner targeting repair in fa47:** invisible physical Text, Vector
  and Bitmap children behind a rounded ancestor clip are excluded from authoring
  hits. Vector fallback applies the same filter to every candidate and retains
  a visible path underneath. Existing parent-frame bounds and selected controls
  keep their authoring behavior. Three new geometry/tool/mounted-entry tests and
  the existing transformed-parent TextPath test join the required 81-name gate.
  All four have actual passes in both fa47 Check runs; each has 78/81 required
  passes and the same three Crop failures. Matching native rounded-clip acceptance remains pending.
- **Build44 bounded follow-up (ab9/a795):** untouched and edited fractional
  TextPath inputs, one transformed parent, Group text edit/Save/Undo and Crop
  Apply/history pass on owned fixtures. Full authored restoration is verified
  apart from modification metadata. Boolean point Undo restores canonical
  geometry and properties but may serialize `Vector` instead of `Rect` shorthand;
  byte spelling is separate from semantic Undo, with no demonstrated data loss.
  In a denied-write check, explicit Save/close errors retain X48 through Cancel;
  restored permissions allow close Save without retyping. Separate-document Save
  All, OS-error Discard and cold recovery are not covered by that route. These
  internal QA results do not establish fa47 or shipping acceptance. See the
  [current scoped results](../alpha/CAPABILITY_COVERAGE.md#current-fa47-hosted-results-and-build44-native-samples--11-october).
- The **Code** workspace edits FNX and supported JSON sources. Saving valid
  source updates the canvas; invalid drafts preserve the last valid file and
  scene. Multi-buffer Save is sequential, not atomic: in a bounded build39 check,
  valid FNX was saved before a second invalid JSON draft was rejected. Cancel
  retained the invalid draft; Discard closed it and kept the FNX already saved.
  This is not rollback of earlier writes. Embedded Find searches the active
  FNX/JSON editor; **Discard draft**
  discards only the selected buffer. Source Save preserves authored Undo, and
  ordinary saved-canvas source refresh remains its own undoable text transaction.
  An unsaved source buffer locks conflicting visual edits. Selecting a layer
  reveals its source; agent source-follow can be paused.
  While a draft owns editing, the accepted single layer's properties remain
  readable, including position, size and auto-layout gap. Empty or mixed
  selection asks the user to select one layer; canvas writes remain locked.
  On `a7f16cb245`, a bounded native same-path external-change/Cancel journey
  preserves the full unsaved draft, controlled external file bytes and timestamp,
  and observed last-accepted canvas geometry and source lock. This does not
  establish conflict resolution, recovery Undo, multiple windows or all edits.
  **12 October 2026, signed Build45:** a recorded canvas Save was followed by
  `Could not refresh saved source: oneshot canceled` in Code. The saved FNX
  retained the edit; closing and reopening the project restored a clean source
  view. If this warning appears, verify the saved change and resolve any unsaved
  drafts before closing, then reopen before editing source. Report a persistent
  warning or a saved-source/canvas mismatch. The trigger and frequency are not
  established by this one observed recovery. See
  [current release limitations](../alpha/PUBLIC_ALPHA.md#current-limitations).
- Structural canvas saves preserve future FNX attributes on surviving nodes and
  keep authored comments when inserting or reparenting content. Explicit source
  edits remain authoritative. Deletion or type changes that would erase
  source-only data report a preservation error because typed Undo cannot restore
  it; edit FNX to intentionally remove that data. All 13 node kinds recognize
  their schema defaults, and conversion removes obsolete fields from the old
  kind without discarding common fields. On `bf0650022c`, bounded native checks
  pass plain-sibling Duplicate/Delete, saved Undo/Redo and cold reopen, visible
  Save refusal followed by Undo recovery, and intentional source-field removal
  without resurrection. Refusal protects persisted files; it does not prevent
  the preceding live canvas edit. Hosted CI is tracked separately for each
  published candidate; broader release gates and the scope of earlier
  component-action results remain unchanged.
- FNX printing preserves finite numeric values without rounding small magnitudes
  to zero. PR91 verifies exact node/type/f64-bit and sidecar replay on the
  Spectrum reference; a complete fresh native import/writer journey remains a
  separate gate.
- External source changes reload through the project watcher. Conflicting
  previews, pending saves and changed files are guarded rather than blindly
  overwritten. These protections still require full release-candidate testing.
- Review Changes and the Git commit/sync surfaces operate on the design
  project. Bundled Git and distribution-specific restrictions are covered by
  the separate release workflow.

Representative native checks cover large-project image edits, scoped layout,
source Find and Undo/Redo, invalid-draft recovery, prototype interaction removal,
and saved-content preservation after reopening. These checks apply to specific
builds and fixtures. The [coverage report](../alpha/CAPABILITY_COVERAGE.md) records
those results, retained failures and remaining release gates, including clipboard
preservation and native performance.

Implementation: [document.rs](../../crates/fig_viewer/src/document.rs),
[design_panel.rs](../../crates/fig_viewer/src/design_panel.rs),
[code_workspace.rs](../../crates/fig_viewer/src/code_workspace.rs), and
[project sessions](../../crates/fanta-format/src/project/session).

## Canvas and authoring

| Capability | How it works and its scope |
| --- | --- |
| Navigation | Pan with Hand; zoom, fit content/selection, and use page-specific viewports. Selection, hover and editing overlays follow the active page. |
| Selection and transforms | Move, resize, rotate, proportional Scale, multi-selection, marquee and nested selection. The inspector also offers rotate 90° and horizontal/vertical flip. Multi-selection enables align, distribute and Tidy up; distribution needs at least three nodes. Snapping and guides assist placement. Locked/hidden content and read-only modes constrain edits. |
| Shapes and drawing | Rectangle, ellipse, line, arrow, polygon, star, Pen, Pencil, Brush and Eraser. Edit Path exposes vector anchors/handles; Path Selection operates on paths. Shape parameters appear for compatible nodes. |
| Containers and structure | Frames, sections, groups, slices; reparenting, z-order, duplicate, delete, group/ungroup, frame selection and boolean operations. Canvas clipboard operations work within the same document, including across pages. Cross-document canvas paste is unsupported and rejected before dependencies can dangle. |
| Region selection and crop | Rectangle/ellipse selection, lasso, polygonal lasso and Magic Wand produce a drawing selection. Draw options expose replace/add/subtract/intersect selection, inversion, applicable wand tolerance/contiguity and crop ratio. Crop commits an undoable crop; these controls are not a general bitmap pixel editor. |
| Text | Create and edit inline text, select ranges, apply typography and text paints, and convert text to outlines. Instance text editing creates an override. |
| Text on Path | Convert one eligible vector baseline, then edit its text and path text properties. Rounded/clipped geometry, zero-length paths and unsupported paints are rejected with a message. |
| Media | Use the local picker to place supported raster images and MP4 video. Existing project assets and generation results also support placement of editable SVG and MP3 audio. Media is validated before placement. Supported media previews provide playback and seek; video trim preserves the original asset and supports Undo/Redo. |

Draw exposes tool-specific size, opacity, smoothing and blend controls. Brush
also supports tip, hardness and flow settings; Pencil and Eraser expose their
applicable subsets. The toolbar Color picker opens the selected fill or page
background picker.

In an editable Design canvas, activating Crop also shows **Ratio**, **Cancel**
and **Apply crop** controls. Drag inside the image to create the crop region,
then Apply. On build37, one freeform Bitmap Apply/Undo/Redo/final Undo/Cancel
journey restores full project content except its modification timestamp. The
earlier missing-controls failures remain recorded; other crop modes and cold
reopening are not covered by that journey. On build39, both side panels left a
256-point canvas that clipped the toolbar's leading control; a 336-point canvas
clipped the Crop hint and placed Apply beyond the right edge. Bitmap drill-in,
Crop entry and Cancel passed, without Apply in that journey. Build41 also passes
wrapped-image Crop entry, Apply followed by Undo, and a separate preview Cancel;
Cancel leaves Crop mode active. Its preview moves the action bar, so an old
Cancel click position can instead hit Apply. At an actual 336-point canvas,
Apply, the bottom toolbar and the top Canvas/Variables/Code controls still clip.
The responsive implementation bounds toolbar/Crop controls and positions the
top workspace tabs within the canvas. In both `ab9faf717` Check runs, the mounted
toolbar regression passes its mode, scrolling and workspace-switching checks.
The three mounted Crop regressions at 240, 256 and 336 points execute but fail
at the first intended drag press after disabled Apply; their later preview,
Apply and history assertions are not reached. Both later fa47 Check runs pass
78/81 required names and retain those three failures: raw window delivery
reaches an oversized Crop panel that occludes the canvas drag point; the hint
collapses to zero width at 336. The source-reviewed one-line `flex_none` hint
fix is included in the successor to fa47; matching execution remains pending.
Build44 separately passes bounded Apply/Undo/Redo/restoration, without proving
those CI logical widths or the candidate fix.

On development QA `0f71b3961d`, one real three-second H.264 fixture passes bounded
Play/Pause, paused Seek, Trim Cancel and Trim Apply with single Undo/Redo and
cold saved-state checks. Its poster matches an independent frame reference,
including exact RGBA bytes; Undo retains the added poster asset. The earlier a7
0/0 readiness failure and later stale-display uncertainty remain unresolved.
These results do not establish an activation fix, arbitrary codecs/VFR or
installed-release media behavior. Separate 8 October optimized Audio checks
cover one Play/Pause/seek/Save/cold journey. MP3 placement passes bounded
storage/history checks, but cold Audio previews were absent for the original
MP3 tone and the newly placed one-second MP3; no WAV case was verified. Audio
preview recovery is not established. See the
[coverage report](../alpha/CAPABILITY_COVERAGE.md#bounded-checks-through-8-october).

The project **Assets** list shows saved/imported media, thumbnails and file sizes.
Choose a visible target page to place a supported asset again; missing files or
unsupported formats show a reason and cannot be placed.

Canvas, keyboard and toolbar Duplicate preserve copied component masters and
complete variant sets using the same rules as Layers menu Duplicate. References
to components outside the copied content remain unchanged. Ordinary Paste keeps
existing live component references. If a copied master or complete variant set
has since been removed, Paste restores its captured definitions with fresh IDs
and reconnects the copied instances. Repeated Paste after Cut creates independent
restored definitions.

Keyboard Cut, Duplicate and Delete now respect direct and inherited locks.
Removing a component master preserves supported surviving instance artwork.
Cutting only part of a variant set, unresolved component dependencies, and
detachments that cannot preserve appearance are refused before modifying the
document; failed Cut leaves the clipboard unchanged. The app explains the
refusal. Bounded native checks now pass asset-bearing Duplicate, Copy/Paste,
Cut/Paste, master/final-variant deletion, repeated component-bundle Paste,
Undo/Redo, lock guards and saved-state reopening on `100fe26921`. Earlier visible
refusal checks and the original load-time viewport failures remain recorded
separately. These fixtures do not exhaust every dependency combination. See the
[current acceptance status](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status).

The properties inspector adapts to the selected kind and selection count. Its
supported sections include position, size, rotation, opacity/blend,
fills and gradients, strokes, effects, corners, typography, auto-layout/grid,
variables, components and export. Page selection has page-specific properties. Audio and specialist leaves show
their own type labels and expose supported position, dimensions, appearance,
effects and export controls; these controls preserve the underlying content.
**Video properties**, **Audio properties** and specialist properties entries
reveal this inspector; they do not switch modes or open dedicated media editors.
For a supported single Video or Audio selection, playback controls appear below
the canvas in **Motion** or **Prototype**. Audio preview also requires an active
canvas window; a missing preview is not diagnosed by the Properties action alone.
Booleans expose operation, fill and stroke controls. Paint recoloring retains
the imported geometry; geometry or operation changes recompute it from operands.
Constraint data can be retained by the document, but constraint controls are
currently disabled in the default Design inspector.

For one editable physical Text on Path layer, the Design inspector header's
**Edit object** action opens inline editing with all text selected. It is
unavailable for multiple selections, pages, virtual instance TextPath content,
locked layers or locked ancestors, and read-only states such as Inspect or a
pending Source draft. The command rechecks the current selection and editability;
a stale or newly protected target leaves the document unchanged.

With one layer selected, **Select matching layers** appears when another layer
of the same kind exists on the active page. It selects matching descendants on
that page without editing content or adding an Undo step. Sections match other
Sections rather than ordinary Frames or Groups, regardless of their paint;
component masters retain their component identity. An explicit Frame identity
is not treated as an imported Section. This matching rule does not change the
inspector's property presets.

Single-node opacity, visibility and uniform vector radius display their active
variable values, including aliases and inherited mode pins. Explicit corner
values keep precedence; unresolved bindings show a labeled fallback and remain
detachable. Representative resolved and pinned values, explicit-corner summaries,
and exact Open/Save/cold preservation passed native checks on `100fe26921`.
The Available label is readable in One Dark and One Light; changing themes and
returning to One Dark preserves the document exactly.
On 7 October, the later QA2824 build passes 16 static checks at 320 and 400 px
in One Dark and One Light. Authored independent corners remain four separate
fields even when all values are 7; the precedence explanation wraps in full.
Bound wrapper opacity and mixed Vector/Bitmap values also display correctly.
Thirteen checkpoints preserve all document fields, timestamps, assets and files.
These checks do not invoke bound Detach or establish an edit/Undo journey;
unresolved-binding fallback remains covered by automated tests.
Mixed values must remain identifiable; controls must neither erase unsupported
data nor silently apply to a stale selection. Editable fields support draft
cancellation and undoable commits. Controls that offer live previews restore
the previous value when cancelled; mixed-selection opacity commits once when
the gesture finishes and does not provide that single-node live preview.
Native numeric/hex draft cancellation and Export-label checks passed at a 320 px
inspector width. A two-kind selection's aggregate-opacity display/edit/Undo and
the visible cross-document paste refusal also passed native checks; see the
[coverage report](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status)
for exact builds and remaining gesture checks. A later 320 px fixture passes
differing 50%/100% values edited to 75%, single Undo/Redo, restoration and exact
reopening. A fast drag that leaves the numeric readout before its first move
did not start a scrub in the earlier tested build. The correction now passes
automated regressions. On the `ce8db37c34` QA build, dragging the numeric
percentage readout to 0% now passes Save, one Undo, one Redo and a full quit/reopen
with exact saved content, assets, timestamps and file bytes. Start the drag on
the displayed value. Held-drag cancellation and broader field combinations
remain unverified.

Implementation: [tools.rs](../../crates/fig_viewer/src/tools.rs),
[canvas tools](../../crates/fanta-tools/src),
[design inspector adapter](../../crates/fig_viewer/src/gpui_adapters/design.rs),
[inline text](../../crates/fig_viewer/src/text_edit.rs), and
[media placement](../../crates/fig_viewer/src/generation_media.rs).

### Double-click behavior by node

Use Move in Design mode. The first click selects; the second activates the
selected, visible target under the pointer. Nested content may need a first
double-click to drill into its container before another pair enters editing.

| Selected target | Double-click action | Finish or cancel |
| --- | --- | --- |
| Text or Text on Path | Open inline text editing at the clicked word. | Click away, Escape or Command+Enter commits and exits; Undo reverses the edit. |
| Vector, including primitive shapes and vectors with image fills | Open Edit Path for anchors and handles. | Escape leaves path editing; Undo reverses a committed edit. |
| Bitmap/image node | Activate Crop. Draw the crop region. | Enter applies; Escape cancels the pending crop. |
| Group, frame, section or boolean | Drill into the next selectable child level. | A subsequent double-click can activate the selected leaf. |
| Component instance | Edit a virtual text override when the pointer hits editable instance text; otherwise reveal its properties inspector. | Finish text editing or inspect instance properties. |
| Video or audio | Reveal the properties inspector without changing mode. | Select the supported media in Motion or Prototype for playback controls below the canvas. |
| Node graph, 3D, AI artifact or embed | Reveal the properties inspector. | No specialized editor is implied for placeholder types. |

Locked, hidden, obscured and other-page scene nodes do not enter editing. Shift,
Command or Control keep their selection meaning; Space panning does not activate
an editor. Only the even click in each pair activates or drills in, preventing a
triple-click from accidentally descending another level. Instance text also
respects locked ancestors, overlapping virtual content and clipping, including
rounded corners. Alias-backed component text properties use the placed instance's
active or inherited mode when resolving editable text. Native checks on `a3a51b084b`
cover default and placed aliases, derived text geometry, Save/single Undo/Redo,
hidden-text no-op and strict reopening of the final saved state. Text inside true
nested instances and virtual Text on Path are not recursively targeted. A direct
text-content variable binding remains authoritative; changing its text requires
changing the variable or binding. Native `8a9a33c35e` checks confirm its refusal
notice/read-only Content row while alias-only instance text stays editable from
the canvas and Design inspector, with single Undo/Redo and exact saved-state
reopening. Post-restart refusal also passed after a selection/focus reset; an
earlier targeting attempt and its unproven cause remain recorded.

On `a7f16cb245`, a bounded native plain Text check changes `FRAME TEXT` to
`FRAME EDIT`, commits with one Escape, and passes Save, single Undo/Redo and
fresh-process reopening. Its fixed box, font/style, full 0..10 style range,
other document content and asset remain exact. All 20 project files follow
the declared edit/history states; cold reopening matches saved Redo exactly.
The fixture uses a pre-existing disposable parent Git repository, whose
metadata also remains exact. The earlier no-parent fixture correctly failed
its strict file boundary when the desktop writer initialized Git; that evidence
remains recorded. This check does not establish rich text, IME, multiline input
or held-Escape behavior. A separate 8 October Command-Return/multiline journey
passes bounded Save/history/cold checks, with second-line clipping in its fixed
box. See the [coverage matrix](../alpha/CAPABILITY_COVERAGE.md#bounded-checks-through-8-october)
for its limits and remaining release gates.

Text on Path accepts the shaped text area, including letter
counters and spaces, while rejecting distant empty areas along the baseline.
Inspection keeps exact glyph geometry. Automated regressions cover these cases; native input
verification remains tracked in the release report. A bounded `a3a51b084b` check
passes Bitmap Crop Apply/single Undo/Redo/reopening after observing Crop entry,
and real curved TextPath **Edit text**/Save/single Undo/Redo/reopening preserves
the curve and surrounding artwork. An earlier batched double-click/drag moved
the bitmap instead; its failure and recovery remain recorded. Group/Boolean
drill-in, selected-vector menu entry/Escape and Video/Audio/Node Graph/3D/AI/Embed
properties reveal also preserve the document exactly. That build entered Edit
Path without initial anchors; the corrected `8a9a33c35e` displays four anchors
immediately through both double-click and the actual Edit vector menu, with
unchanged saved content. The `ce8db37c34` QA build also passes Edit vector
menu entry for a Group-contained vector, a precise anchor drag/Save and one
Undo/Redo. All 35 nodes and five assets match the predeclared states; a full
quit/reopen preserves all 30 project file bytes and timestamps exactly. On 7 October,
the same build also passes a two-operand authored Boolean's left-vector anchor
edit through entry/cancel, Save, single Undo/Redo, exit and exact cold reopening.
Its complete 35-node/five-asset oracle and all 30 file bytes remain exact;
other Boolean arrangements and imported baked geometry remain unverified.
Selected Bitmap/Vector Inspect refusals and a nine-layer
generic menu passed separately; TextPath Inspect targeting was inconclusive.
These checks do not establish every node-entry or text-input combination.

### Canvas context menu

Right-click targets the visible layer under the pointer, retaining an existing
selection when the target is already inside it. Pending inspector/text edits are
handled before changing the target; protected drafts are preserved.
Escape dismisses the menu without clearing the selected layer; clicking an
inspector field transfers input to that field.

| Target | Relevant commands |
| --- | --- |
| Text or Text on Path | **Edit text** opens inline editing with its text selected. |
| Vector | **Edit vector** opens Edit Path. |
| Bitmap | **Crop image** activates Crop. |
| Video or audio | **Video properties** or **Audio properties** reveals the inspector. |
| Boolean, node graph, 3D, AI artifact or embed | The corresponding **Boolean properties**, **Node graph properties**, **3D properties**, **AI artifact properties** or **Embed properties** entry reveals the inspector. |
| Main component or instantiable variant-set frame | **Create instance** places an instance beside the component. |
| Instance with an available main component | **Go to main component**, **Detach instance** and **Reset all overrides**. Supported detachments preserve expanded content and can be undone; unsupported appearance combinations explain the limitation and leave content unchanged. |
| Eligible node | **Create component**, **Add auto layout**, **Outline stroke**, **Flatten** and **Use as mask** appear only when applicable, using the same operations as the layers panel. |
| Selection or canvas | Clipboard commands, Duplicate/Delete, Group/Frame selection, Bring to front/Send to back; **Ungroup** appears for a supported container. Empty canvas offers Paste. |

Single-node editor/component commands are hidden for multi-selection. Writes
are disabled in read-only mode or when selected layers are locked; read-only
properties and applicable navigation remain available. Reordering applies to
the selected roots and is undoable. The canvas menu is not shown in Draw mode
or during prototype presentation.

Variant-set instances use the member shown on the canvas for these commands
and the Design inspector, including the set default, variable aliases, active
modes and inherited mode pins. Reset clears instance customizations and restores
the resolved default member's size. If **Go to main component** would switch
scopes while source edits are pending, it keeps the current scope, selection and
camera and explains that you must save or discard those edits first. Eight
focused automated regressions cover these routes. On `67d29ea288`, four native
Go to main component routes resolve the expected default or mode-selected
member and align its artwork with selection. Two checkpoints preserve all
20 nodes, the asset, timestamps and 30 project files. On the later `41becdffe5`
QA build, ten native checkpoints pass Open, two navigation controls, Reset and
Detach with single Undo/Redo saves, and cold reopening of the saved detached
state. The 80 × 40 Small instance becomes a Frame with its expected child;
only declared content/files change, and cold reopening preserves all saved bytes
and the timestamp exactly. Earlier strict failures and broader limits remain in
the [coverage report](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status).

Flatten is unavailable when it would discard images, mixed text colors or
decorations, child effects, clipping, layout, bindings, animation or component
links. Supported simple vector groups and boolean geometry remain convertible.
Flattening an imported Boolean preserves its original baked appearance. A live
Boolean containing another baked Boolean refuses conversion when folding its
operands would change the artwork.
If another layer uses the node or one of its children as a pattern source,
Flatten refuses the operation with an explanation and leaves the document
unchanged. This reference check happens when the command runs.

Implementation: [view_context_menu.rs](../../crates/fig_viewer/src/view_context_menu.rs).

## Components, variables and reusable styles

Create a main component, create/navigate instances, edit supported instance
overrides, define typed component properties and bind them to descendants.
Variant sets support axes, member names and arrangement. Component recursion
and incompatible bindings are rejected. Page and layer duplication clone included
component definitions and complete variant sets, retaining outside references
and redirecting supported internal prototype/variant links to the copies.
Ordinary same-document paste keeps the existing component definitions. These
copy-reference cases have operation and Undo/Redo coverage; their native
acceptance remains tracked in the release report.

The Variables workspace supports collections, modes, typed values, aliases,
rename/delete and compatible property bindings. Variable kinds are color,
float, string, boolean, typography, enum (one of the variable's options, also
bindable to text content) and image (a project asset, bindable to a fill's
artwork); enum and image values edit as text in the workspace, and agents
create them through `create_variable` and `set_variable_options`. Modes can
apply at project, page and parent scopes. Unbinding bakes the resolved value.
Shared styles and imported bindings participate in rendering and source
persistence.

Text can show variables inline. A `{{name}}` token shows the value of the
variable it names: its bare name when only one variable has it, otherwise its
`Collection/Name` path. String, number, boolean and enum variables resolve, in
the text's mode; a token that names nothing shows as typed. The source keeps
the tokens, and the text being edited shows them raw.

A layer can carry template rules, the changes someone filling the template in
may not make: moving, resizing, rotating, selecting, deleting, duplicating,
editing its text and more. The lock button in the canvas controls switches the
window to Fill mode, where the canvas, Delete, Duplicate, text editing and the
Design inspector's edits keep to those rules; arranging or transforming a
multi-selection, variable bindings and grouping do not check them yet. Rules
are set in FNX source (`editing={{…}}`) or through `fanta-api`; the inspector
does not show them yet.

Bound solid fills show their resolved mode color and variable name in the
inspector and picker. Alpha is read-only while the variable controls the full
color; **Detach** bakes the active value and makes it directly editable. Undo
restores the binding.

Representative native checks passed two-mode color binding, project-mode
switching, unbind/Undo and reopening, followed by long-title containment/sidebar
toggling and the resolved picker, Detach/Undo and strict restart. See the
[coverage report](../alpha/CAPABILITY_COVERAGE.md) for exact builds and limits.

Implementation: [component actions](../../crates/fig_viewer/src/component_actions.rs),
[component properties](../../crates/fig_viewer/src/component_properties.rs),
[variants](../../crates/fig_viewer/src/variant_sets.rs), and
[variables](../../crates/fig_viewer/src/variables_workspace.rs).

## Motion, prototypes and design review

- **Motion:** clips, node/property tracks, keyframes, scrubbing, playback,
  looping, timeline zoom, animation presets and custom easing. Live keyframe
  and numeric edits retain their identity and become undoable transactions.
  A clip's loop mode persists with it and is the default the prototype and web
  runtimes play it with; clips can also declare a frame rate, a stage frame, a
  backdrop and per-node transform origins (rotation and scale pivots). The
  seconds-and-degrees motion spec (`fanta_doc::MotionSpec`) is the portable
  form exporters and other motion tools read.
- **Prototype:** starting frames, interactions, destinations, transitions,
  overlays, supported links and property-animation bindings. Presentation
  provides frame navigation/restart and restores the editor viewport on exit.
- **Comments:** canvas pins, threads/replies, resolve and motion anchors;
  supported rich messages and agent attribution persist in the project.
- **Measurements and annotations:** persistent review metadata, editing and
  undo, with unsent annotation drafts preserved through navigation. Review
  overlays do not become artwork in exports.
- **Inspect/Dev:** inspect content and review information with artwork editing
  disabled. Read-only barriers also apply to shortcuts and agent/source paths;
  changing mode must preserve or explicitly finish a pending edit.

In Dev, **Saved Code** is read-only. Measurements and annotations can still be
authored on an editable page, and Undo/Redo is limited to those review marks;
artwork edits remain blocked. **Readiness unavailable** is a disabled control.

Select an editable animation clip, pause playback and enable **Auto key** to
edit the **At playhead** properties. Edits preview live and commit as one
keyframe change. These controls are unavailable without an active clip, during
playback or on a read-only page.

Prototype interactions offer **On click**, **On drag**, **On hover**,
**While pressing**, **After delay** and **On key press** triggers. Choose
**Navigate to**, **Open overlay**, **Scroll to**, **Set variable**,
**Change variant**, **Open link**, **Back** or **Close**, then configure the
applicable destination, value or transition. Supported authoring choices do
not imply that every trigger/action combination has passed native acceptance.

The timeline's **Current time** field accepts a typed time and seeks on Return.
The interaction detail's **X** closes the detail while retaining its interaction;
**Remove** is the separate deletion action.

Representative native checks passed Motion playback, ruler seeking and keyframe
drag/Undo, plus prototype click navigation, Restart, Escape and saved-project
reopening. The corrected time field and non-destructive detail X also passed
native checks with strict saved-content equality after restart. A later native
explicit Remove/Save and single Undo/Save also passed complete comparisons.
That journey's first restart was compared with an outdated baseline after
confirmed user position edits; the failure remains recorded. A later fresh
matching-baseline Save/restart passes on `ac038333e3`. This does not establish
other triggers or overlays. The [coverage report](../alpha/CAPABILITY_COVERAGE.md)
records exact provenance and remaining cases.

Implementation: [timeline](../../crates/fig_viewer/src/timeline.rs),
[motion](../../crates/fig_viewer/src/motion_panel.rs),
[prototype authoring](../../crates/fig_viewer/src/prototype_panel.rs),
[prototype runtime](../../crates/fanta-present/src/fanta_present.rs),
[comments](../../crates/fig_viewer/src/comments_ui.rs),
[measurements](../../crates/fig_viewer/src/view_measurements.rs),
[annotations](../../crates/fig_viewer/src/view_annotations.rs), and
[Dev mode](../../crates/fig_viewer/src/view_dev_mode.rs).

## Export and automation

Export selections with **PNG, JPG, SVG or PDF** presets. Raster sizing,
multi-selection batches and multiple presets are supported. Export operates on
committed artwork, reports completion/failure and refuses an active preview.
Bounded native Vector/Bitmap exports validate dimensions, PNG/SVG/PDF appearance,
Vector JPG, collision preservation and two simultaneous presets, including 2×
PNG. Bitmap JPG retains its original four-pixel failure and separate limited
flat-block pass. The later quality-95 correction passes the hosted regression
using the original fixture, crop, pixel mask and tolerances; quality 90 fails
that same test. A later build37 Bitmap export produces PNG, JPG, SVG and PDF
with unchanged project content and assets. PNG pixels, SVG embedded/rendered
pixels and the PDF embedded bitmap match the source. PDF raster rendering has
retained edge differences; JPEG passes only a declared 1,200-pixel cell-interior
check, not the original regression mask. Exporting the selected Bitmap yields
its full image; this does not establish clipped parent-frame export. General
JPEG fidelity, advanced effects and broader batches remain format-specific
checks, and higher quality can increase file size.

The built-in designer can inspect/edit the canvas, use project `fanta.md`
instructions, prepare design assets, follow source edits and report agent
activity. Plan/Review and editing modes have distinct write permissions. See
[AI_DESIGNER.md](../alpha/AI_DESIGNER.md).

The toolbar's **Replace content**, **Rewrite text**, **Translate text** and
**Rename layers** commands open a draft in the Agent Panel with the current page
and selection context. Review and send that prompt to request the edit.

External agents connect to the local live MCP surface. Core tools include
`get_editor_state`, `batch_get`, `batch_design`, `get_screenshot`,
`get_guidelines`, `read_fnx_source` and `validate_fnx_source`; design-system,
asset-import/preparation, comments and activity tools extend that surface.
Batches are transactional: failure preserves document state, assets, selection
and history. A successful batch is a single undoable edit. Tool screenshots
verify rendered output; they do not prove that a mouse or keyboard control works.

The generation workspace has **Image**, **Video**, **Audio**, **Vector**,
**Design** and **Masks** modes. Select an available model and review its supported
options before submitting. Source images, end frames, masks and voice references
are available only for compatible operations; a mode does not promise that every
model supports those inputs.

- **Vector** offers **Create vectors** and **Trace image**, producing editable
  paths for supported results.
- **Design → Prepare design brief** opens an unsent Agent Panel prompt for the
  active canvas. Review and send it to begin; preparing the brief changes no art.
- **Masks** generates selectable masks from a source. A selected mask can guide
  an image edit; supported background removal can produce a transparent result.
- Preview a completed result, **Save result** to a chosen file, or add a
  supported result to project assets and place it from **Assets**. Canvas/asset
  writes require an editable design; save or discard pending source edits first.
- **Retry submission** and saved **Recover…** entries resume uncertain requests.
  Resolve the saved request before submitting another. Account changes and
  unavailable models are checked during recovery.

Available models come from the signed-in account's catalog. A bounded 9 October
internal build33 Seedream image journey passes generation, placement, Undo/Redo,
Save and reopening, with one matching five-credit usage row. It does not prove
ordinary balance debit, exactly-once upstream execution or other providers.
In-flight recovery, broader generation and payment behavior remain unverified;
mock recovery tests do not establish availability. See the
[dated acceptance scope](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status).

A separate **11 October build37** check completed normal browser sign-in with
the human-selected production account and loaded 18 image models. One
FLUX1.1Pro request quoted six credits, returned a Ready image and showed the
balance change from 344 to 338. Native download produced a decoded 1024×768 JPEG;
Refresh and Open preview retained the matching result. After clean Quit and a
new process, the account restored without browser sign-in, the balance remained
338, and Refresh returned the same three Ready creations and 18 models. Opening
the existing preview made no new generation request. This is one bounded
account/model journey, not database exactly-once, network-loss recovery, Agent,
other-model or Add-to-project acceptance. The owned project remained unchanged.

A later build37 check used **Add to project** once for that existing image in
an independent project clone. Save and warm reopen retained the library image;
one placement and Save retained its matching bytes, and Undo and Save restored
the original page while keeping the library image. The original project stayed
unchanged and the balance remained 338. Cold import restoration and other models
remain unverified.

Implementation: [export](../../crates/fig_viewer/src/export.rs),
[agent operations](../../crates/fig_viewer/src/agent_surface.rs),
[live MCP tools](../../crates/fig_viewer/src/live_mcp.rs),
[generation workspace](../../crates/fig_viewer/src/generation_workspace.rs), and
[recovery journal](../../crates/fig_viewer/src/generation_journal.rs).

## Accounts, models and connected tools

Use **Sign in to Fanta**, then open **Settings → Credits & billing** to view the
account's balance, usage and purchase history. Available actions depend on the
account and distribution; a displayed control is not proof of a completed
payment or restore. The Agent Panel model chooser lists available Fanta models
and their displayed price information. Choose the model and an appropriate
agent profile before sending a request. Tool permissions, retries and auxiliary
requests matter: a model's output limit or a restrictive profile is not a total
spending cap. See [AI_DESIGNER.md](../alpha/AI_DESIGNER.md) for canvas permissions.

The current source limits each native Agent turn to 64 model dispatches shared
by automatic continuations, retries, fallback and automatic compaction. Reaching
the limit stops the turn with a visible message while preserving completed work;
a new user turn gets a fresh limit. Manual compaction, title generation, tools,
subagents and provider-internal retries are outside this limit. This is not a
dollar or token limit. Both canonical and push Check runs for `37c4073d` on
11 October pass all 66 required names, including the nine request-limit
regressions. Earlier failed runs remain recorded. Paid native Agent
behavior remains unverified. A positive account balance does not reserve the
full turn's potential cost, and safe billing of repeated chat requests is not
established. See the [coverage report](../alpha/CAPABILITY_COVERAGE.md) for the
separate test and production limits.

There are two distinct MCP directions:

- The **local live MCP** server lets external agents inspect/edit this editor,
  using the transactional tools described above.
- **Settings → MCP** manages servers used by the agent. The configured Fanta
  hosted server uses the signed-in account when its endpoint matches the account
  server and no explicit `Authorization` or `X-API-Key` header overrides it.
  The stable default enables generation tools at `/mcp`; `fanta-design` at `/v2/mcp` is disabled in
  stable and enabled by the separate development/preview configuration. Custom
  HTTP or local-command servers have their own configuration and access rules;
  local commands are restricted in Mac App Store builds.

Earlier bounded native checks observed an existing account, billing reads and model
rows. They do not establish fresh browser/callback sign-in, trustworthy current
prices/caps for a paid request, live managed-MCP execution, payments or general
account recovery. The bounded image-generation result above does not establish
Agent chat or a total spending cap.
The later build37 sign-in, six-credit image debit and cold session/history check
above are separate positive evidence; they do not establish those remaining
service or spending guarantees.

Implementation: [account and billing UI](../../crates/fanta/src/fanta/settings_modal.rs),
[MCP settings](../../crates/fanta/src/fanta/settings_mcp.rs),
[account-backed MCP](../../crates/project/src/context_server_store.rs), and
[service defaults](../../assets/settings/default.json).

## Boundaries

Node-graph, 3D, generic AI-artifact and embed node types exist in the format, but
the canvas renderer currently draws placeholders for those types. The 3D card
shows a studio cube, not the stored model payload. Their properties entries open
the shared inspector, not dedicated content editors.
Existing 3D payloads and assets are preserved when unrelated canvas changes are
saved and reopened. Neither storage support nor a node enum constitutes a
finished user workflow.

Large-page rendering work remains incomplete. The retained renderer has a Metal
API and an opt-in macOS canvas worker, enabled only with
`FANTA_RETAINED_TRANSLATION=1`; normal rendering remains the default. Seven
actual-Metal API tests and one IOSurface-worker test pass their bounded
correctness and fallback checks. Earlier CPU fit-all gains do not establish
native responsiveness. Standalone development Metal timings retain a 100% image
regression and a slower 100% instance result; fit-all instance timings were
rejected because the target produced no visible pixel motion. The sampled
fit-all image p95 remains above 16 ms. Matched native gesture, complete Save and
memory measurements are still required; the below-16-ms native drag target
remains unproven. The user's 11 October hands-on Spectrum check accepts its
current performance; this qualitative result is separate from those unmeasured
benchmarks and does not request another optimization pass. See the
[coverage report](../alpha/CAPABILITY_COVERAGE.md) for exact evidence and earlier
failures.

Mac App Store builds restrict local process execution, external agents and
related inherited editor features; see [MAC_APP_STORE.md](../alpha/MAC_APP_STORE.md).
The selected release target is a direct-download Mac app. Packaging now checks
the app inside the produced DMG against the staged bundle and enforces the
configured signature/notarization checks. Earlier accepted internal **build41**,
exact `c46a1b41bcd00222cbdba5e9a1ce450fc0658c10`, passes hosted package/source,
checksum, local mounted-package, CLI and isolated persistent-QA preparation
checks. Its bounded native focused-property Save/close, TextPath/Group editing,
Boolean vector entry, Bitmap Crop and sampled inspector layouts pass; narrow
336-point controls still clip. Build41 predates the fully green `37c4073d` source
checks and its Settings Escape/audio changes, and the responsive-controls
and closed-tab restoration fixes in this release candidate. Earlier build39 at `0732a74e` retains its
scoped native journeys and closed-tab restart failure; build41 did not retest
that cold-session path. Both internal packages are unsigned/ad-hoc QA, not
Developer ID/notarized releases. Earlier **build37**, exact
`22f734193844eab66485e3cb322b529c2965e297`, now passes hosted and local package,
checksum, CLI and isolated-QA verification. Its bounded 11 October native checks
cover selected inspector, Crop, media playback, export and save/close routes;
see the [latest acceptance record](../alpha/CAPABILITY_COVERAGE.md#current37-native-checks--11-october).
It remains ad-hoc signed, not a Developer ID/notarized release. The following
**build33** result is historical, not validation of later fixes. That internal optimized artifact, from
`79cc2ef8c094149e8496476c1cb53e2945b8b54a`, passes hosted and
local mounted-package verification, CLI checks and isolated QA derivation. It
is an arm64, ad-hoc-signed artifact (`signed=false`), not a Developer ID/notarized
release. These checks do not replace native capability acceptance. The
[coverage report](../alpha/CAPABILITY_COVERAGE.md#final-source-and-internal-package--9-october)
retains its exact source and evidence links.
The current signed **Build45** package has passed signing, notarization, stapling,
Gatekeeper, tested installation, Save/Quit/reopen and same-build manual
replacement checks. [Current package identity and acceptance scope](../alpha/PUBLIC_ALPHA.md#accepted-package-and-publication-status).
Public availability remains **preparing / on hold**. Anonymous public download,
clean-account installation, migration from an older version and rollback remain
unverified.
A bounded check of the approved production service, using an isolated profile
and unchanged development `0f71b3961d` binary, shows an existing account, loaded
billing data and persistence through one cold process restart. It does not
establish fresh sign-in, recovery, general Settings navigation, AI/credits, live
MCP or payment; no purchase or AI request was made. The recorded stale Settings
display remains unresolved. Media breadth, native performance, installed
capability coverage, anonymous public download and remaining authenticated-service journeys
are still release gates. Deliberately
disabled inherited services are recorded in
[disabled-services-binnacle.md](disabled-services-binnacle.md).
