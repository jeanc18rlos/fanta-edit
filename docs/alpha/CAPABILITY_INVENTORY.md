# Fanta release capability inventory

This is the release-facing inventory for desktop commit `17de5eb`, the exact
first-milestone candidate `9d92b30` built on top of it, and source-only
candidate `895b6b7` identified below. It separates
implemented behavior from test evidence and production readiness so a source
implementation is not mistaken for a shipped, verified capability.

Status terms:

- **Yes** — the capability exists for the stated scope.
- **Partial** — useful behavior exists, but the visible product surface is
  incomplete or contains a known gap.
- **No** — the capability is absent, intentionally unavailable, or blocked.
- **Unknown** — that verification has not been completed.
- **N/A** — production services are not involved.

“Native” means the packaged desktop app or a native QA bundle was exercised.
It does not mean production credentials, inference, billing, signing, or
notarization were used.

## Release matrix

| Surface | Implemented | Automated | Native | Production | Release boundary |
|---|---:|---:|---:|---:|---|
| Canvas selection and drawing | Yes | Yes | Partial | N/A | Move, Hand, Scale, Path Select, Node Edit, shapes, Pen, Pencil, Frame, Section, Slice, Text, Comment, and the source-only Text on Path work are wired. Text on Path conservatively converts one eligible selected vector in place, enters inline editing, and uses cluster-derived curved caret, hit, selection, and visual-bounds geometry. Scale and Path Select have native history/reopen evidence; Text on Path has no native or final-artifact evidence. |
| Toolbar chrome and editing commands | Yes | Yes | Partial | N/A | Resources, Actions, zoom, panel toggles, undo/redo, clipboard, duplicate/delete/select-all, Group, Ungroup, Frame Selection, Present, Design/Motion, and AI routes are wired. The exact `9d92b30` native pass verified centered Design and Motion layouts, responsive zoom collapse, panel-toggle recentering, flyout event isolation, explicit Ask AI, and searchable structure/text-AI Actions. The remaining editing-command and menu matrix still needs the final artifact pass. |
| Roadmap toolbar controls | Partial | Yes | Unknown | N/A | Text on Path and contextual Motion keyframing are implemented only in source candidate `895b6b7`. Arrow, direct Image/Video placement, Annotation, Measure, Dev mode/tools, auto-keyframe, and time comments still report that they are unavailable; Motion Path is hidden and unimplemented. They are not release claims. |
| Application menus | Yes | Partial | Unknown | N/A | Fanta, File, Edit, View, Window, and Help are declared and routed in source. The packaged-installer menu pass is outstanding. |
| Design inspector | Partial | Yes | Partial | N/A | Geometry, opacity, corners, stroke geometry, supported effects, single-axis auto layout, typography, component properties, masks, transforms, and alignment are wired. The source-only TextPath inspector projects its exact kind/name; font family, Regular/Italic style, weight, size, line height, pixel letter spacing, start/center/end alignment, and underline/strikethrough; side/orientation; and validated API-debug start segment/position controls. Its synthetic glyph Fill permits RGB and opacity edits only; paint add, style, visibility, remove, and reorder actions are gated. Direction is preserved and rendered but has no typed UI action, and disabled non-applicable typography controls remain visible. The coherent candidate source gate passed, including all 708 viewer tests. In exact `9d92b30` native QA, fill/stroke visibility changed the document and Undo restored Fill; switching Fill to Linear produced a finite rendered gradient that exported successfully. Text on Path and the remaining paint/blend matrix still need final-artifact native coverage. |
| Prototype mode | Yes | Yes | Unknown | N/A | Triggers, navigation/overlay/scroll actions, variables/components, transitions, flows, and presentation runtime are implemented. |
| Motion mode and timeline | Partial | Yes | Unknown | N/A | The production `TimelineShell` supports clip creation/selection/rename/duration, property-track keyframes, move/delete, interpolation/easing, entrance presets, playback, looping, scrub, and zoom. The contextual toolbar's Keyframe chooser shares the timeline's seven-property catalog and adds or replaces one keyframe at the playhead in a single undoable transaction; Animation Style applies an undoable entrance preset and synchronizes the timeline. The contradictory primary Motion flyout is removed. Auto-keyframe, motion paths, and time comments remain unavailable. |
| Comments | Yes | Yes | Unknown | N/A | Pins, threads, replies, resolve/delete, mentions, attachments, and skill routing are implemented. |
| Variables workspace | Yes | Yes | Unknown | N/A | Collections, variables, modes, values, scopes, binding, preview, and undo are implemented. Typography-variable editing is read-only. |
| Code workspace | Yes | Yes | Unknown | N/A | FNX and JSON follow the current selection and source. Both panes are intentionally read-only. |
| Export engine | Partial | Yes | Partial | N/A | PNG, JPEG, SVG, and PDF engines and 1×/2×/4× presets exist. On exact `9d92b30`, toolbar Export visibly reported success without changing the Inspector and wrote a valid 418×354 `Shape@2x.png`; automated coverage also exercises the unsaved-canvas failure. The default inspector still does not expose preset configuration, and native failure feedback remains to be driven. |
| Image generation/editing | Yes | Yes | Yes | Unknown | Prompt/model/source/inpaint controls plus submit, poll, save, reuse, and place passed deterministic native QA. Authenticated production inference is not verified. |
| Video generation | Yes | Yes | Partial | Unknown | Generation, save/place, and native playback controls exist; deterministic client/native flow passed. Production inference and full frame-specific playback validation remain open. |
| Vector generation/trace | Yes | Yes | Yes | Unknown | Prompt-to-SVG, validation/sanitization, save/place, and source-required Trace paths exist. Production Create is unverified; production Trace is blocked until an SVG worker is configured. |
| Masks and background removal | Yes | Yes | Yes | Unknown | Include/exclude points, strength, segmentation, edit-mask handoff, normalization, and local background removal passed deterministic QA. Production segmentation is unavailable/unverified. |
| Design AI workspace | Partial | Yes | Yes | N/A | “Prepare unsent Agent brief” creates a local draft using the registered `design_state`, `design_edit`, and `design_screenshot` tools. It no longer requires sign-in/model discovery, and request identity controls stay locked while other generation is active. It still does not provide generation progress, review, acceptance, or one-step Undo for an accepted design. |
| Agent toolbar attachments and voice | Partial | Partial | Unknown | N/A | When every selected node belongs to the active page or component root, the toolbar attachment control creates an immutable attach-time JSON snapshot as a named embedded resource in an unsent Agent draft. It includes exact node ids and stable document/path/project/root/scope identity, caps the resource at 64 nodes and 128 KiB, and bounds descriptive strings; mixed-root and off-root selections are rejected. With no canvas selection it opens the existing Agent Add Context menu. The user must review and submit the draft, and rapid requests queue without sending automatically. Focused snapshot, identity, request-framing, URI, draft-insertion, and queue checks passed in the current source tree; native validation has not occurred. Voice recording, permission flow, transcription, and cancellation are unfinished, so Voice input still declines explicitly. |
| Managed backend and workers | Partial | Yes | Partial | No | Public health is up, but it reports no configured media worker families. Authenticated production GPU paths are not verified. |
| Billing catalog | Partial | Yes | No | No | The public catalog still exposes the legacy Free/Pro/Team values. The proposed launch catalog is documented in [`PRICING_PROPOSAL.md`](PRICING_PROPOSAL.md) but is not approved or activated. |
| Hosted macOS installer | Partial | Yes | No | No | Desktop CI and exact `17de5eb` Apple Silicon packaging pass. The downloaded DMG's checksum, embedded revision, arm64 binaries, strict ad-hoc signature, and bundled Git passed non-launch inspection. Developer ID signing, notarization, Gatekeeper, launch, and clean-Mac/second-account verification remain release gates. |

## Remaining verified product gaps

These are implementation defects, not cosmetic polish. A visible control must
change state, produce output, or give an explicit unavailable message.

1. Export runs through a legacy inspector entity while the default inspector
   hides its Export section. Toolbar export now relays progress and results to
   the canvas, but preset configuration remains outside the visible product
   surface.
2. The production timeline and contextual toolbar can author keyframes and
   entrance presets without the former duplicate primary flyout. Auto-keyframe
   and time comments remain visible but explicitly unavailable; Motion Path is
   hidden and has no document implementation.
3. Text on Path direction is serialized and honored by rendering, but it is not
   exposed by the typed inspector UI. Caret positions inside one shaping
   cluster that contains multiple graphemes use equal subdivisions rather than
   exact glyph-internal caret positions. Glyphless source intervals are retained
   when Skia exposes cluster geometry; otherwise rendering and edit geometry
   fail closed rather than inventing a caret interval. Color/bitmap glyph runs
   cannot provide an exact vector silhouette, so path-silhouette-dependent
   inner-shadow and background-blur effects are skipped instead of using a
   false rectangle.
4. Dev mode and its inspect, measure, annotation, and readiness controls remain
   unavailable. The Text on Path fields in the Design inspector are not Dev mode.
5. Voice input remains unavailable; no recorder, permission, transcription, or
   cancellation lifecycle is implemented.

## Inspector fixes in the current source line

- Fill and stroke visibility preserves and restores paint alpha through
  undoable edits.
- Supported solid payloads, finite unbound gradients with an identity transform,
  and per-paint blend modes produce undoable document edits.
- Pattern, Image, Video, and Shader payloads, bound paint payloads, and
  unsupported or nonidentity-transform gradients now raise the existing
  unavailable notice instead of silently doing nothing.
- Pass Through works. Linear Burn and Linear Dodge are explicitly unavailable
  because the document engine has no matching blend modes.
- Shadow blend is read-only because the document model has no representation
  for it.

These changes have automated coverage. Exact `9d92b30` native QA additionally
verified Fill and Stroke visibility, Fill Undo, and a finite Linear gradient;
the remaining payload/blend matrix stays a final-artifact release gate.

The source-only TextPath inspector permits edits only to document-backed fields.
It supports font family, Regular/Italic style, weight, size, line height, pixel
letter spacing, start/center/end alignment, underline/strikethrough, and glyph
RGB/opacity across the base style and its runs. Its single synthetic Fill is
color-and-opacity-only, while the shared UI keeps non-applicable typography
controls visibly disabled. Start segment/position controls are explicitly
labelled as API debug data; a completed interaction commits as one undo step,
and side/orientation flips preserve direction. While any inspector content
preview is active, autosave and direct persistence are blocked, relevant
external changes wait for reconciliation, and cancel or selection change
restores and reprojects the document before persistence resumes. Direction
remains unexposed, and this inspector work has not been exercised in a native
build.

The coherent source-only gate passed all 708 `fig_viewer` tests; 116
`fanta-text` unit tests and one doctest, with one network test ignored; 21
focused `fanta-render` TextPath tests; five focused `fanta-doc` TextPath tests;
297 `fanta-tools` tests (290 unit, six end-to-end, and one ink oracle); and 597
`fanta-gpui` checks (594 unit and three architecture), with one doctest ignored.
The package-scoped `./script/clippy` gate is also green for `fanta-text`,
`fanta-render`, `fanta-doc`, `fanta-tools`, `fanta-gpui`, `fig_viewer`,
`acp_thread`, `agent`, and `agent_ui`. Focused Agent evidence includes two
canvas-selection request-framing checks; three attachment, one external-context,
and nine queue-filter Agent UI checks; 49 mention checks; and one
canvas-selection URI round trip. These are source tests, not native or release
artifact validation.

## Exact first-milestone native checkpoint

The release build embedded exact commit
`9d92b30d437ad7ce3cfa72b9843284d26cdce83f` and completed in 7m14s. A distinct
ad-hoc-signed `dev.fanta.ToolbarMilestoneQA` bundle used a fresh profile and a
local project. Native observations covered:

- the one-row Design toolbar and two-row Motion toolbar centered within the
  current canvas bounds, including after panel changes and a narrower window;
- responsive zoom hiding at the narrow tier, an explicit Ask AI control, and
  the Motion toolbar clearing the production timeline;
- shape flyout selection without creating a canvas node, plus searchable
  Group/Ungroup, Frame Selection, Replace Content, Rewrite Text, Translate
  Text, and Rename Layers entries in the toolbar Actions palette;
- a signed-out Ask AI suggestion becoming a local unsent draft;
- document-backed Fill/Stroke visibility, Fill Undo, and a finite Linear
  gradient; and
- toolbar Export preserving sidebar state, showing a success notice, and
  writing a valid 418×354 PNG.

The candidate quit with exit code zero. Its screenshots, exported PNG, bundle,
profile, and verification record are retained under
`/tmp/fanta-release-qa-20260909/toolbar-milestone-9d92b30`. Starting this QA
bundle displaced the previously running stable-channel Fanta process despite
the distinct bundle ID and profile; after QA, that prior untouched bundle was
restarted with its original profile. Parallel native instances must therefore
not be assumed safe.

## Intentionally gated behavior

These controls already decline explicitly, are deliberately read-only, or stay
hidden because their document behavior does not exist. They should stay out of
release claims, but they are not silent-handler bugs:

- Dev mode/tools, direct media placement tools, Annotation, Measure,
  auto-keyframe, and time-anchored comments decline explicitly. Motion Path is
  hidden and unimplemented.
- Text on Path conversion of rounded, clipped, degenerate, hidden/locked, or
  ambiguously/unsupported-painted vectors; those cases decline without changing
  the document. Direction editing, exact glyph-internal carets within one
  multi-grapheme shaping cluster, structural edits to the synthetic glyph Fill,
  and color/bitmap-glyph path-only inner or background effects remain
  unavailable. Missing source-cluster geometry fails closed.
- Layout Grid, inspector Export and Selection sections, aspect lock,
  constraints, single-node arrange, resize-to-fit, grid auto layout, layout
  guides, unsupported effect families, Pattern/Image/Video/Shader paint
  payloads, bound paint payloads, unsupported or nonidentity-transform
  gradients, Linear Burn, and Linear Dodge. Shadow blend is read-only.
- Typography-variable editing and the FNX/JSON code panes.
- Voice input. Canvas-selection attachment is source-implemented and its
  framing, snapshot, identity, URI, draft, and queue automated gates pass;
  removal interaction and native interaction passes remain open.

## Cosmetic and presentation work

These items can be evaluated independently of functional release gates:

- Toolbar density, shadow/radius, bottom inset, label weight, and responsive
  reflow.
- Inherited command-palette naming and remaining Zed-labelled entries.
- Whether roadmap controls should be hidden rather than explaining their
  unavailable status.

## Evidence and remaining gates

The detailed test and native-run record is in
[`RELEASE_VALIDATION.md`](RELEASE_VALIDATION.md). The public-backend, installer,
and production-auth limitations are tracked in [`LAUNCH.md`](LAUNCH.md).
Before release sign-off:

1. Complete final-artifact native coverage for the remaining inspector
   paint/blend matrix and keep unsupported choices explicitly unavailable or
   read-only.
2. Expose Export preset configuration in the default product surface and
   native-verify both success and failure feedback.
3. Native-verify the contextual seven-property Keyframe chooser, its single-step
   Undo behavior and rejected requests that author no keyframe, and the absence
   of the contradictory primary Motion flyout.
4. Native-verify Text on Path create/edit/Undo/Redo/save/reopen/export behavior,
   including multi-grapheme clusters, glyphless intervals, inspector preview
   cancellation/persistence barriers, and explicit rejection paths. Add a typed
   direction control before claiming the full requested scope.
5. Native-verify the canvas-selection attachment preview/removal flow, including
   page and component roots, identity checks, mixed-root rejection, both caps,
   and rapid queued requests; then finish the file-attachment and voice
   recording/transcription lifecycles.
6. Implement Dev mode's inspect, measure, annotation, and readiness controls.
7. Extend the now-honest Design AI draft handoff into the agreed end-to-end
   progress, review, acceptance, and Undo workflow.
8. Complete authenticated production Image, Video, Vector, and Mask checks
   only after worker families are ready; do not spend production credits merely
   to probe an unready service.
9. Complete the hosted, signed/notarized installer and Gatekeeper pass.
10. Run the manual smoke checklist against that exact artifact.
