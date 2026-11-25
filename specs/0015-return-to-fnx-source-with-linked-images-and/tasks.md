# Tasks: Return to FNX source with linked images and project fonts

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [ ] **T1.** Link media by path and load assets tolerantly: path values in
  `asset`, optional `natural_size`, an unlisted or damaged managed file as a
  diagnostic, no asset file removed by a save that could not verify it
  (R1 to R6). Verify: A1, A2 and A3 as `fanta-format` and `fanta-headless`
  tests; stage `engine`.
- [ ] **T2.** Load project fonts from `fonts/` in the editor, the headless
  engine and the worker, and warn on a substituted family (R7, R8). Verify:
  A4 (resolution half) and A5; stages `engine` and `render`.
- [ ] **T3.** Harden the FNX loader: mint ids for added elements in the
  older layout, and leave out a design that does not parse (R12, R13).
  Verify: A8 and A9; stages `source-order` and `shell-design-source`.
- [ ] **T4.** Teach `fanta-check` and `check_project` the FNX layout, with
  FNX cases in the hostile-edit corpus (R15). Verify: A11; stage
  `shell-hostile-edit-corpus`.
- [ ] **T5.** Move the web export out of the project tree for `fanta-site`,
  `serve-project`, the browser canvas, the wasm engine and the plugin pane,
  and have it carry fonts and linked media (R9, R14). Verify: A4 (export
  half) and A10; stage `lint-wasm`.
- [ ] **T6.** Save FNX again: the writer switch in `fanta-format` and
  `fanta-headless`, conversion of web-storage projects on save, the web
  reader's bound-text fix, and browser-side saving per Q5 (R10, R11, R17).
  Update the test names in `.github/workflows/check.yml` in the same pull
  request. Verify: A6, A7 and A13; stages `engine`, `import` and `editor`.
- [ ] **T7.** Rewrite the seeded project guide, the plugin skill, the schema
  page, the README and the alpha page for FNX as the source, and note the
  change in specs 0005, 0007 and 0008 (R16). Verify: A12; `script/gate`.
