# Fanta release capability inventory

This is the release-facing inventory for desktop commit `17de5eb` and the
current release candidate work on top of it. It separates implemented behavior
from test evidence and production readiness so a source implementation is not
mistaken for a shipped, verified capability.

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
| Canvas selection and drawing | Yes | Yes | Partial | N/A | Move, Hand, Scale, Path Select, Node Edit, shapes, Pen, Pencil, Frame, Section, Slice, Text, and Comment are wired. Text-on-path is unavailable. Scale and Path Select have native history/reopen evidence; the complete visible set still needs the manual toolbar pass. |
| Toolbar chrome and editing commands | Yes | Yes | Partial | N/A | Resources, Actions, zoom, panel toggles, undo/redo, clipboard, duplicate/delete/select-all, Group, Ungroup, Frame Selection, Present, Design/Motion, and AI routes are wired. The structure and four text-AI commands are registered and covered by exact palette-action tests; the complete native toolbar pass remains open. |
| Roadmap toolbar controls | No | Yes | Unknown | N/A | Arrow, direct Image/Video placement, Annotation, Measure, Text-on-path, Dev mode/tools, auto-keyframe, motion path, and time comments report that they are unavailable. They are not release claims. |
| Application menus | Yes | Partial | Unknown | N/A | Fanta, File, Edit, View, Window, and Help are declared and routed in source. The packaged-installer menu pass is outstanding. |
| Design inspector | Partial | Yes | Partial | N/A | Geometry, opacity, corners, stroke geometry, supported effects, single-axis auto layout, typography, component properties, masks, transforms, and alignment are wired. In the current candidate, fill/stroke visibility, supported solid and finite unbound identity-transform gradient payloads, and per-paint blend modes produce undoable document edits. Unsupported paint payloads and node blend modes decline explicitly; shadow blend is read-only. Exact-artifact native coverage of these changes is outstanding. |
| Prototype mode | Yes | Yes | Unknown | N/A | Triggers, navigation/overlay/scroll actions, variables/components, transitions, flows, and presentation runtime are implemented. |
| Motion mode and timeline | Partial | Yes | Unknown | N/A | The production `TimelineShell` supports clip creation/selection/rename/duration, property-track keyframes, move/delete, interpolation/easing, entrance presets, playback, looping, scrub, and zoom. Toolbar Animation Style only echoes local UI state, both Add Keyframe faces fail to reach the real timeline-wide property menu, and the duplicate primary Motion flyout contradicts working secondary controls. Auto-keyframe, motion paths, and time comments remain unavailable. |
| Comments | Yes | Yes | Unknown | N/A | Pins, threads, replies, resolve/delete, mentions, attachments, and skill routing are implemented. |
| Variables workspace | Yes | Yes | Unknown | N/A | Collections, variables, modes, values, scopes, binding, preview, and undo are implemented. Typography-variable editing is read-only. |
| Code workspace | Yes | Yes | Unknown | N/A | FNX and JSON follow the current selection and source. Both panes are intentionally read-only. |
| Export engine | Partial | Yes | Unknown | N/A | PNG, JPEG, SVG, and PDF engines and 1×/2×/4× presets exist. Toolbar Export uses the same engine and now mirrors running, success, and failure feedback to the canvas without changing sidebar state. The default inspector still does not expose preset configuration. |
| Image generation/editing | Yes | Yes | Yes | Unknown | Prompt/model/source/inpaint controls plus submit, poll, save, reuse, and place passed deterministic native QA. Authenticated production inference is not verified. |
| Video generation | Yes | Yes | Partial | Unknown | Generation, save/place, and native playback controls exist; deterministic client/native flow passed. Production inference and full frame-specific playback validation remain open. |
| Vector generation/trace | Yes | Yes | Yes | Unknown | Prompt-to-SVG, validation/sanitization, save/place, and source-required Trace paths exist. Production Create is unverified; production Trace is blocked until an SVG worker is configured. |
| Masks and background removal | Yes | Yes | Yes | Unknown | Include/exclude points, strength, segmentation, edit-mask handoff, normalization, and local background removal passed deterministic QA. Production segmentation is unavailable/unverified. |
| Design AI workspace | Partial | Yes | Yes | N/A | “Prepare unsent Agent brief” creates a local draft using the registered `design_state`, `design_edit`, and `design_screenshot` tools. It no longer requires sign-in/model discovery, and request identity controls stay locked while other generation is active. It still does not provide generation progress, review, acceptance, or one-step Undo for an accepted design. |
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
2. The production timeline can author keyframes and entrance presets, but its
   toolbar faces are inconsistent: Animation Style does not mutate the document,
   Add Keyframe does not open the real property menu, and the duplicate primary
   Motion flyout declines actions that work in the contextual row.

## Inspector fixes in the current candidate

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

These changes have automated coverage; exact-artifact native verification is
still a release gate.

## Intentionally gated behavior

These controls already decline explicitly or are deliberately read-only. They
should stay out of release claims, but they are not silent-handler bugs:

- Text-on-path, Dev tools, direct media placement tools, Annotation, Measure,
  auto-keyframe, motion path, and time-anchored comments.
- Layout Grid, inspector Export and Selection sections, aspect lock,
  constraints, single-node arrange, resize-to-fit, grid auto layout, layout
  guides, unsupported effect families, Pattern/Image/Video/Shader paint
  payloads, bound paint payloads, unsupported or nonidentity-transform
  gradients, Linear Burn, and Linear Dodge. Shadow blend is read-only.
- Typography-variable editing and the FNX/JSON code panes.
- Toolbar file attachment and voice input.

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

1. Complete exact-artifact native coverage for the inspector paint fixes and
   keep unsupported choices explicitly unavailable or read-only.
2. Expose Export preset configuration in the default product surface and
   native-verify both success and failure feedback.
3. Connect toolbar Animation Style and Add Keyframe to the existing production
   timeline operations, then remove or unify the contradictory primary flyout.
4. Extend the now-honest Design AI draft handoff into the agreed end-to-end
   progress, review, acceptance, and Undo workflow.
5. Complete authenticated production Image, Video, Vector, and Mask checks
   only after worker families are ready; do not spend production credits merely
   to probe an unready service.
6. Complete the hosted, signed/notarized installer and Gatekeeper pass.
7. Run the manual smoke checklist against that exact artifact.
