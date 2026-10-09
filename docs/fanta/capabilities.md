# Fanta capability guide

Source inventory updated **9 October 2026**; acceptance evidence below is dated.
This describes the current editor implementation; release verification is tracked
separately in
[the capability coverage report](../alpha/CAPABILITY_COVERAGE.md). Earlier alpha
reports describe their dated builds and can contain controls that have since
changed. A feature listed here is not a claim that its complete installed-app
journey has passed.

## Projects, pages and source

Create a design from **New Design**, open a Fanta project folder, or import a
`.fig` file or `.fant` snapshot. A saved project contains `fanta.json`, page and
component FNX source, identity sidecars, metadata and assets. See
[project-schema.md](project-schema.md) for the layout.

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
- Canvas edits support Undo/Redo and debounced autosave. Save commits active
  valid property edits; failures remain visible. Save As creates a separate
  project and rejects occupied or unsafe destinations. Clean canvas tabs restore
  their project and page/component scope when the workspace reopens. Unsaved
  canvas or source content still requires the normal close/save decision.
- The **Code** workspace edits FNX and supported JSON sources. Saving valid
  source updates the canvas; invalid drafts preserve the last valid file and
  scene. Embedded Find searches the active FNX/JSON editor; **Discard draft**
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
| Video or audio | Reveal the properties inspector. | Use the supported media or preview controls for playback; double-click keeps the current mode. |
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
rename/delete and compatible property bindings. Modes can apply at project,
page and parent scopes. Unbinding bakes the resolved value. Shared styles and
imported bindings participate in rendering and source persistence.

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
That journey's restart was compared with an outdated baseline after confirmed
user position edits; no matching-baseline restart pass is claimed. The
[coverage report](../alpha/CAPABILITY_COVERAGE.md) records exact provenance and
remaining cases.

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
PNG. Bitmap JPG has a separate limited flat-block pass; its original four-pixel
comparison failure remains recorded. Imported or advanced effects, broader
batches and general JPEG quality still need format-specific checks.

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

Available models come from the signed-in account's catalog. Real-provider
submit/poll/save/place, restart recovery, credits and payment behavior still
require service acceptance; mock recovery tests do not establish availability.

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

Bounded native checks have observed an existing account, billing reads and model
rows. They do not establish fresh browser/callback sign-in, trustworthy current
prices/caps for a paid request, live managed-MCP execution, payments or general
account recovery. No paid AI request is accepted by this coverage record.

Implementation: [account and billing UI](../../crates/zed/src/zed/settings_modal.rs),
[MCP settings](../../crates/zed/src/zed/settings_mcp.rs),
[account-backed MCP](../../crates/project/src/context_server_store.rs), and
[service defaults](../../assets/settings/default.json).

## Boundaries

Node-graph, generic AI-artifact and embed node types exist in the format, but the
canvas renderer currently draws placeholders for those types. A 3D node has a
renderer hook; this inventory does not establish a complete shipped 3D editor.
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
remains unproven. See the [coverage report](../alpha/CAPABILITY_COVERAGE.md) for
exact evidence and earlier failures.

Mac App Store builds restrict local process execution, external agents and
related inherited editor features; see [MAC_APP_STORE.md](../alpha/MAC_APP_STORE.md).
The selected release target is a direct-download Mac app. Packaging now checks
the app inside the produced DMG against the staged bundle and enforces the
configured signature/notarization checks. Isolated verifier tests and the
8 October internal optimized DMG verification pass. That arm64 artifact is ad-hoc signed (`signed=false`), with a separately
verified isolated QA derivative; it is not a Developer ID/notarized release.
Public download, clean-Mac installation, update and rollback remain unverified.
A bounded check of the approved production service, using an isolated profile
and unchanged development `0f71b3961d` binary, shows an existing account, loaded
billing data and persistence through one cold process restart. It does not
establish fresh sign-in, recovery, general Settings navigation, AI/credits, live
MCP or payment; no purchase or AI request was made. The recorded stale Settings
display remains unresolved. Media breadth, native performance, installed
capability coverage, distribution and remaining authenticated-service journeys
are still release gates. Deliberately
disabled inherited services are recorded in
[disabled-services-binnacle.md](disabled-services-binnacle.md).
