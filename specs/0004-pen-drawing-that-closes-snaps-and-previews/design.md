# Design: Pen drawing that closes, snaps and previews

## Approach

The work stays in the layers that own it ([crate map](../../docs/architecture/crate-map.md)):
the pen's state machine in `fanta-tools` (Interaction), snapping in
`fanta-canvas` (Interaction), overlay painting and key routing in `fig_viewer`
(Desktop editor), and the Draw toolbar row in `fanta-ui`. No layer rule
changes, so no decision record is needed.

### Closing (R1–R4)

- Replace `CLOSE_DIST` (8 canvas units) with `CLOSE_PX = 10.0` screen pixels,
  converted with the current zoom (`ctx.snap.zoom`) on every event.
- Test closeness against the **raw** pointer position, before snapping.
  Inside the distance, the anchor position is exactly the first anchor and no
  other snapping runs.
- `PointerEvent::Move` computes the same test while drafting, so the hover
  state and the press always agree.
- A press inside the distance no longer commits at once. The tool enters a
  `Closing` state; a drag sets the first anchor's incoming tangent (and its
  outgoing one, mirrored per the handle mode); release commits the closed
  path. A press and release without a drag closes with the tangents already
  there, as today.

### Snapping (R2)

- `fanta_canvas::SnapCandidates` gains `push_point(DVec2)`, which adds a
  point's x and y to the axis candidates. `ToolContext` gains
  `snap_point_with_points(world, exclude, points)`, which collects the usual
  scene candidates and adds the draft's anchors when the "Snap to path points"
  option is on.
- The first anchor's close test runs before this snap, so it always wins.

### Feedback and preview (R3, R5)

- New `ToolOverlay::PreviewPath { world_points, closed, filled }`: the draft
  flattened to a polyline, including the segment from the last anchor to the
  snapped cursor as the cubic a press would create. `filled` is set while the
  close test passes.
- New `ToolOverlay::PenCloseTarget { world }`: a ring on the first anchor.
- The pen reuses `PathAnchor` for placed anchors and `PathHandle` for the last
  anchor's handles, which the canvas already paints for the node-edit tool.
- New `CursorHint::Pointer`, mapped to GPUI's pointing-hand cursor, while the
  close test passes. GPUI has no pen-close cursor; the ring carries the
  meaning.
- Painting lives beside the existing overlay arms in
  `crates/fig_viewer/src/canvas.rs` (`PreviewLine`, `PathAnchor`,
  `PathInsertHint`).

### Modifiers (R6, R7)

- Pointer events already carry `ModifierKeys`. With Shift, the new anchor's
  direction from the last anchor (or a dragged handle's direction) snaps to
  the nearest multiple of the angle step, keeping the distance.
- With Alt during a handle drag, only the outgoing handle follows the pointer.
- Space is not a tool key today, and holding Space pans the canvas
  (`space_pan` in `view.rs`). Add `LogicalKey::Space`. While the pointer is
  held during a pen drag, the view sends Space to the tool, which then moves
  the anchor and its handles with the pointer; otherwise Space keeps panning.

### Continuing, closing and joining open paths (R8)

- On a press with no draft, the pen looks for an endpoint of an open vector
  path within `CLOSE_PX` (scope per Q2). A hit loads that path's anchors into
  the draft in world space, reversed when the press is on the start, and
  records `continuing: NodeId`.
- Committing a continued draft replaces that node's path in one operation
  instead of creating a node. A press on the other endpoint closes it through
  the existing `node_math::close_path`; a press on another open path's
  endpoint merges the two through `node_math::join_paths` and removes the
  second node, in one undo step.

### Undo while drafting (R9)

- `Tool` gains `fn undo_draft(&mut self) -> bool`, defaulting to `false`.
  `FigView::undo` (`view.rs`) asks the active tool first and returns when it
  handled the undo. The pen pops its last anchor; with no anchors left it
  returns `false`, so Undo reaches document history again.

### Options and hints (R10)

- `PenOptions { snap_to_path_points, angle_step_degrees, handle_mode }` lives
  on `FigView` for the window and reaches the tool through `ToolContext`.
  `handle_mode` is `Mirrored` (angle and length), `Aligned` (angle only) or
  `Independent`, which sets how a new smooth anchor's two handles relate.
- The Draw toolbar row in fanta-ui shows a pen section while the pen is
  active: a "Snap to path points" toggle, a 15°/45° segmented control, a
  handle-mode menu, and a hint line: "Click for a corner · Drag for a curve ·
  Shift snaps the angle · Alt breaks the handle · Click the first point to
  close · Enter finishes · Esc cancels". This needs a fanta-ui pull request
  and a pin bump here (Q3).

## Interfaces and data

```rust
// fanta-canvas
impl SnapCandidates { pub fn push_point(&mut self, point: DVec2); }

// fanta-tools
pub struct PenOptions { pub snap_to_path_points: bool, pub angle_step_degrees: f64, pub handle_mode: HandleMode }
pub enum HandleMode { Mirrored, Aligned, Independent }
impl ToolContext<'_> { pub fn snap_point_with_points(&self, world: DVec2, exclude: &[NodeId], points: &[DVec2]) -> SnapResult; }
pub trait Tool { fn undo_draft(&mut self) -> bool { false } /* … */ }
pub enum ToolOverlay { /* … */ PreviewPath { world_points: Vec<[f64; 2]>, closed: bool, filled: bool }, PenCloseTarget { world: [f64; 2] } }
pub enum CursorHint { /* … */ Pointer }
pub enum LogicalKey { /* … */ Space }
```

No file format, design op or public API changes. The document only sees the
same `VectorNode` operations the pen creates today.

## Alternatives

- **Scale the old constant by zoom.** The same as a screen distance, but
  harder to read; rejected for clarity.
- **Close on any anchor of the draft**, as Graphite does. It makes loops
  easy, but in a quick sketch it also closes paths by accident; the draft's
  other anchors align instead (R2).
- **Rely on the Close Path command.** It exists, but only after the fact, and
  it is the workaround users fall back on today.

## Risks

- **Snapping precedence surprises.** The first-anchor magnet only applies to
  the pen's draft, so node-edit and the other tools keep their snapping.
- **Space conflicts with panning.** Scoped to a held pen drag; a regression
  test covers both meanings.
- **The fanta-ui change lands separately.** T6 is the only task that needs
  it; T1–T5 ship without it.

## Verification

| Criterion | Check |
| --- | --- |
| A1, A2 | `fanta-tools` pen tests at zoom 0.25, 1 and 8; a `fanta-canvas` test for point candidates |
| A3 | `fanta-tools` tests on the overlays and cursor returned by hover events |
| A4–A6 | `fanta-tools` pen tests for the closing drag, Shift, Alt and Space |
| A7 | `fanta-tools` tests that continue, close and join paths in a `Doc` |
| A8 | `fanta-tools` test for `undo_draft`, and a mounted `fig_viewer` test that Undo pops an anchor |
| A9 | A mounted `fig_viewer` test of the Draw row with the pen active |
| A10 | Manual check by the founder in a release build at 25%, 100% and 800% |
