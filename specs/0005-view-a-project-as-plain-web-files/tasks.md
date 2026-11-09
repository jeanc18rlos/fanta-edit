# Tasks: View a project as plain web files

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [ ] **T1.** `fanta_site::web::export_web` and `fanta-site --plain`: one
  Skia-rendered SVG per page (via `fanta-headless` export), `tokens.css`
  from variables and modes, `index.html` + dependency-free `viewer.js`,
  `pages/<slug>.map.json` id map, `report.json` (R1, R3, R4, R8, R11; R2
  except the `file://` notice). Verify: fixture export test asserting the
  file set and no `.wasm`, byte-identical double export, tokens.css mode
  assertions, `script/architecture verify` for the new dependency.
- [ ] **T2.** Prototype hotspots: overlay anchors from `flow_start.json`
  and click triggers, plus the `file://` notice (R2, R6). Verify: anchor
  targets resolve to exported pages in tests; a served click-through smoke
  in the release `build` stage (A4).
- [ ] **T3.** Editor save wiring: regenerate `web/` on save in
  `fig_viewer`, watcher ignores the directory, schema doc gains the `web/`
  row (R1, R7). Verify: watcher test raises no reconciliation for writes
  inside `web/`; delete-then-save restores identical bytes (A3).
- [ ] **T4.** Headless command and cloud parity: `fanta-headless` exposes
  the same export; editor and headless outputs compared byte-for-byte in a
  test (R10, A7).
- [ ] **T5.** Fonts and the report: reuse the website exporter's embedding
  and substitution policy; substitutions and unembeddable runs land in
  `report.json`; support matrix page in `docs/fanta/` (R9 partial, R11, A6,
  A8).
- [ ] **T6.** Id-aware emitter: inline `data-fanta-id` attributes and
  text-as-`<text>`, replacing the map file where it can (R4 full, R9 full,
  A2). Revisits the design's emitter alternative.
