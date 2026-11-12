# Tasks: Split the editor's largest files and cap file size

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges. T2 to T7 each wait until no open pull request
touches their file.

- [ ] **T1.** File-size ratchet: `docs/architecture/file-sizes.toml` with
  the generated baseline, `script/architecture sizes`, `verify` running it
  (R4, R5, R6). Verify: A4, A5.
- [ ] **T2.** `view.rs`: move `mod tests` to `view/tests.rs` and split it
  into `view/tests/*.rs` (R1, R2, R3). Verify: A2, A3.
- [ ] **T3.** `view.rs`: split the `impl FigView` blocks by method cluster
  into `view/*.rs` (R1, R2, R3). Verify: A2, A3.
- [ ] **T4.** `gpui_adapters/design.rs` (17,674 lines): tests, then
  implementation, into `gpui_adapters/design/*.rs` (R1, R2, R3). Verify:
  A2, A3.
- [ ] **T5.** `document.rs` (11,364) into `document/*.rs` (R1, R2, R3).
  Verify: A2, A3.
- [ ] **T6.** `canvas.rs` (11,000) into the existing `canvas/` directory
  (R1, R2, R3). Verify: A2, A3.
- [ ] **T7.** `generation_workspace.rs` (10,383) into
  `generation_workspace/*.rs` (R1, R2, R3). Verify: A1, A2, A3.

Outside this spec, recorded here so it is not lost:

- **Dead files sweep,** one pull request below the spec threshold: scripts
  and root files that no workflow, doc, skill or script references. Check
  each against non-Rust consumers first (the asset-deletion trap in
  `CLAUDE.md`).
- **Editor-group layers:**
  [ADR 0031](../../docs/decisions/README.md), proposed with this spec's
  pull request stack. It moves no code.
