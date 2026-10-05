# Fanta capability guide

Source inventory updated **5 October 2026**. This describes the current editor
implementation; release verification is tracked separately in
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
| Media | Place supported local images, editable SVG, MP4 video and MP3 audio, or generation results. Files are validated before placement. Canvas media controls include playback and seek; video trim preserves the original asset and supports Undo/Redo. |

Draw exposes tool-specific size, opacity, smoothing and blend controls. Brush
also supports tip, hardness and flow settings; Pencil and Eraser expose their
applicable subsets. The toolbar Color picker opens the selected fill or page
background picker.

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
Single-node opacity, visibility and uniform vector radius display their active
variable values, including aliases and inherited mode pins. Explicit corner
values keep precedence; unresolved bindings show a labeled fallback and remain
detachable. Representative resolved and pinned values, explicit-corner summaries,
and exact Open/Save/cold preservation passed native checks on `100fe26921`.
The Available label is readable in One Dark and One Light; changing themes and
returning to One Dark preserves the document exactly.
The separate four-field equal-corner control and precedence label were not
observed; unresolved-binding fallback remains covered by automated tests.
Mixed values must remain identifiable; controls must neither erase unsupported
data nor silently apply to a stale selection. Editable fields support draft
cancellation and undoable commits. Controls that offer live previews restore
the previous value when cancelled; mixed-selection opacity commits once when
the gesture finishes and does not provide that single-node live preview.
Native numeric/hex draft cancellation and Export-label checks passed at a 320 px
inspector width. A two-kind selection's aggregate-opacity display/edit/Undo and
the visible cross-document paste refusal also passed native checks; see the
[coverage report](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status)
for exact builds and remaining gesture checks.

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
| Video or audio | Reveal the properties inspector. | Playback remains in the appropriate Motion/Prototype controls; double-click does not change mode. |
| Node graph, 3D, AI artifact or embed | Reveal the properties inspector. | No specialized editor is implied for placeholder types. |

Locked, hidden, obscured and other-page scene nodes do not enter editing. Shift,
Command or Control keep their selection meaning; Space panning does not activate
an editor. Only the even click in each pair activates or drills in, preventing a
triple-click from accidentally descending another level. Instance text also
respects locked ancestors, overlapping virtual content and clipping, including
rounded corners. Alias-backed component text properties use the placed instance's
active or inherited mode when resolving editable text. The later correction is
covered by automated tests; native verification remains pending. Text inside true nested
instances and virtual Text on Path are not recursively targeted. A direct
text-content variable binding remains authoritative; changing its text requires
changing the variable or binding. Text on Path accepts the shaped text area, including letter
counters and spaces, while rejecting distant empty areas along the baseline.
Inspection keeps exact glyph geometry. Automated regressions cover these cases; native input
verification remains tracked in the release report.

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
Imported or advanced effects still need format-specific fidelity checks.

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

The generation workspace offers image, video, audio and vector creation, plus
supported source/mask workflows, preview, save, placement and durable recovery.
Vector creation and image tracing can yield editable paths. Available models
come from the signed-in account's catalog; network failures and uncertain jobs
have recovery paths. Live provider availability, credits and payment behavior
require separate service validation.

Implementation: [export](../../crates/fig_viewer/src/export.rs),
[agent operations](../../crates/fig_viewer/src/agent_surface.rs),
[live MCP tools](../../crates/fig_viewer/src/live_mcp.rs),
[generation workspace](../../crates/fig_viewer/src/generation_workspace.rs), and
[recovery journal](../../crates/fig_viewer/src/generation_journal.rs).

## Boundaries

Node-graph, generic AI-artifact and embed node types exist in the format, but the
canvas renderer currently draws placeholders for those types. A 3D node has a
renderer hook; this inventory does not establish a complete shipped 3D editor.
Existing 3D payloads and assets are preserved when unrelated canvas changes are
saved and reopened. Neither storage support nor a node enum constitutes a
finished user workflow.

Mac App Store builds restrict local process execution, external agents and
related inherited editor features; see [MAC_APP_STORE.md](../alpha/MAC_APP_STORE.md).
Account sign-in, billing, cloud generation and distribution have separate
release gates. This guide makes no current production-status claim. Deliberately
disabled inherited services are recorded in
[disabled-services-binnacle.md](disabled-services-binnacle.md).
