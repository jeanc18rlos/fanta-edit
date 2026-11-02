# Tasks: Pen drawing that closes, snaps and previews

One task per pull request, in order. T1 alone fixes the reported closing
problem; each later task adds one capability.

- [ ] **T1.** `fanta-canvas` and `fanta-tools`: a 10-pixel screen-space close
  distance tested before snapping, the first-anchor magnet, and the draft's
  anchors as alignment candidates (R1, R2). Verify: A1, A2.
- [ ] **T2.** `fanta-tools` and `fig_viewer` canvas: the curve and filled
  previews, the close ring and cursor, and a rubber band that ends at the
  snapped point (R3, R5). Verify: A3.
- [ ] **T3.** `fanta-tools` and `fig_viewer` keys: the closing drag, Shift
  angle steps, Alt to break a handle and Space to move the anchor during a
  drag (R4, R6, R7). Verify: A4–A6, and Space still pans outside a drag.
- [ ] **T4.** `fanta-tools`: continue, close and join open paths with the pen
  (R8). Verify: A7.
- [ ] **T5.** `fanta-tools` and `fig_viewer`: Undo removes the last anchor
  while drafting (R9). Verify: A8.
- [ ] **T6.** fanta-ui and `fig_viewer`: pen options and the gesture hint in
  the Draw toolbar row, with the pin bump (R10). Verify: A9.
- [ ] **T7.** Founder's manual check at three zoom levels, and the pen
  section of `docs/fanta/capabilities.md` (A10).
