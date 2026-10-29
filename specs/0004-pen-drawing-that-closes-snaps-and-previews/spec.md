---
status: draft
date: 2026-10-29
owner: Jean Rojas
decisions: []
---

# Pen drawing that closes, snaps and previews

## Problem

Drawing a closed shape with the pen is hard enough that users give up. The
causes are in `crates/fanta-tools/src/pen.rs` on 2026-10-29:

- **The close target scales with zoom.** A press closes the path when it lands
  within `CLOSE_DIST = 8.0` canvas units of the first anchor. At 25% zoom that
  is 2 screen pixels; at 800% it is 64.
- **Snapping works against closing.** The close test uses the snapped press
  position, so snapping to the grid or a nearby object can pull the press off
  the first anchor. The draft's own anchors are never snap targets.
- **Nothing says you can close.** Hovering the first anchor changes neither
  the cursor nor the anchor.
- **The preview is not what you get.** Curves preview as straight lines, and
  the rubber band follows the raw pointer while the anchor lands on the
  snapped point.
- **No modifiers.** No angle constraint, no breaking a handle while drawing,
  no moving the anchor being placed, no dragging on the closing press to
  shape the last curve.
- **No continuing.** The pen cannot extend an open path or join two; that
  takes separate Close and Join commands afterwards.
- **No pen controls.** The Draw toolbar row offers selection, crop and path
  commands, but no pen options and no hint of the gestures.

The founder reported the problem on 2026-10-29 and asked for the controls,
the panel and the drawing experience to improve, using CE.SDK or Graphite as
references.

References studied: Graphite's open-source pen tool (behavior only; its code
is not copied) and CE.SDK's public vector-edit documentation, under the
clean-room rule of [ADR 0022](../../docs/decisions/0022-offer-a-fanta-native-design-api.md).

## Goals

- Closing a path works on the first try at any zoom.
- What the preview shows is what a press will place.
- The pen supports the standard drawing modifiers.
- The pen continues, closes and joins open paths by drawing.
- The pen's options and gestures are visible while it is active.

## Non-goals

- The pencil and brush tools.
- The node-edit tool's gestures, which already work (anchor and handle drags,
  Alt to break a handle, double-click to toggle smooth and corner, inserting
  on a segment, Delete that keeps the shape). Only shared snapping changes
  reach it.
- Boolean operations, text on a path, and agent design ops for the pen.

## Requirements

- **R1.** While a draft has at least two anchors, a press within 10 screen
  pixels of its first anchor must close the path, at every zoom level.
- **R2.** The draft's first anchor must attract the cursor ahead of grid and
  object snapping when it is within that distance. The draft's other anchors
  must be snap targets for alignment without closing the path.
- **R3.** Hovering inside the close distance must show that a press will
  close: a ring on the first anchor, a close cursor, and a filled preview of
  the shape the close would produce.
- **R4.** Dragging after a closing press must shape the closing segment, and
  the path must commit on release.
- **R5.** The preview must draw the in-progress segment as the curve a press
  would create, show the handles of the last anchor, and end the rubber band
  at the snapped point a press would use.
- **R6.** Shift must constrain the new segment, and a handle being dragged, to
  the angle step chosen in the pen options.
- **R7.** While dragging a new anchor's handle, Alt must move only the
  outgoing handle, and Space must move the whole anchor with its handles.
- **R8.** A pen press on an endpoint of an open vector path must continue that
  path. A press on its other endpoint must close it, and a press on an
  endpoint of a different open path must join the two into one path.
- **R9.** While a draft is in progress, Undo must remove its last anchor
  instead of undoing document history. A finished path must undo as one step.
- **R10.** While the pen is active, the Draw toolbar row must show its
  options (snapping to the draft's anchors, the angle step, the handle mode
  for new smooth anchors) and a one-line hint of its gestures.

## Acceptance criteria

- **A1 (R1).** A `fanta-tools` test at zoom 0.25, 1 and 8 closes the path with
  a press 9 screen pixels from the first anchor and does not close it at 12.
- **A2 (R2).** With a grid snap target 3 pixels away and the first anchor 6
  pixels away, a press closes the path.
- **A3 (R3, R5).** Hover responses include a ring overlay on the first anchor,
  the close cursor and a filled path preview inside the close distance; away
  from it they include a curve preview whose end equals the snapped point.
- **A4 (R4).** A press on the first anchor followed by a drag commits a closed
  path whose closing segment is a cubic with the dragged tangent.
- **A5 (R6).** With Shift and a 45° step, a press at 30° from the last anchor
  places the anchor on the 45° ray at the same distance.
- **A6 (R7).** With Alt, a handle drag leaves the incoming handle unchanged;
  with Space, the anchor and both handles move by the drag delta.
- **A7 (R8).** A press on an open path's endpoint and two more presses leave
  one node with two more anchors. A press on its other endpoint closes it, and
  a press on another open path's endpoint leaves one node where there were two.
- **A8 (R9).** Undo during a draft removes one anchor and adds no history
  entry; after Enter, one Undo removes the whole path.
- **A9 (R10).** A mounted `fig_viewer` test shows the pen options and the hint
  in the Draw row while the pen is active, and turning off snapping to the
  draft's anchors changes where the tool snaps.
- **A10 (R1–R5).** Manually, in a release build, a five-point closed shape
  closes on the first press at 25%, 100% and 800% zoom.

## Open questions

- **Q1.** Angle step default: 45° (Figma's pen) or 15° (Graphite's)? The
  options offer both either way.
- **Q2.** Continue any hovered open path (Graphite) or only the selected one
  (Figma)? Hovering is faster; selection is harder to trigger by accident.
- **Q3.** Pen options in the Draw toolbar row, which needs a paired fanta-ui
  pull request, or in a floating chip on the canvas, which does not?
- **Q4.** A fixed 10-pixel close distance, or the user's snap tolerance
  setting?

## Changes after acceptance

- None.
