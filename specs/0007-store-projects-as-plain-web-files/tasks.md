# Tasks: Store projects as plain web files

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [x] **T1.** ([#186](https://github.com/jeanc18rlos/fanta-edit/pull/186))
  Writer in `fanta-format`: move spec 0005's emitter out of
  `fanta-site` and extend it to the full project (components, interaction
  sidecars, `fanta.json` version bump); `fanta-site --plain` becomes a
  caller (R1 file set, R2–R5, R10). Verify: A3's byte round-trip on write,
  file-set test, `script/architecture verify` and map.
- [x] **T2.** ([#187](https://github.com/jeanc18rlos/fanta-edit/pull/187))
  Reader: load a web-storage tree back into the document from its sidecars
  (`nodes.json` records plus the shared headers); property round-trip over
  every node kind (R6). Verified: A3 in full. The SVG+CSS field overlay
  moved into T3, where the tolerant parser lives.
- [ ] **T3.** Hand-edit contract: parse the emitted SVG+CSS dialect
  tolerantly (the R7 subset), let parsed fields win over the `nodes.json`
  records, canonicalizing save, per-node report of what could not be kept
  (R7, R8). Verify: A4, A6, the diff-noise golden.
- [ ] **T4.** Source parity for gradients, masks, booleans and instances —
  no raster stand-ins in storage (R11; shared with spec 0005 T6). Verify:
  round-trip fixtures per feature; canvas golden against the written form.
- [ ] **T5.** The flip: editor save, headless engine and `.fig` import
  write the web format; legacy FNX/FSS projects convert on save; FNX/FSS
  writers removed, readers kept (R1, R9, R10). Verify: A1, A2, A5, A7.
- [ ] **T6.** Docs and schema: project-schema doc rewritten for the web
  layout, support matrix for hand-editing tools, deprecation note for the
  FNX/FSS readers' future.
