# Tasks: A web canvas for browsers and agent hosts

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [x] **T1.** ([#194](https://github.com/jeanc18rlos/fanta-edit/pull/194)) Engine to WebAssembly: move `fanta-format`'s filesystem code
  behind a default-on cargo feature, add the browser `getrandom` cfg, and
  build a wasm package over `fanta-doc` + `fanta-design-ops` + the
  read/emit core with a bytes-in/bytes-out API: load sources, apply a
  `batch_design` op list, return the rewritten files (R1, R2). Verify:
  A1's CI target check and A2's byte-identity test against the native
  writer.
- [ ] **T2.** `fanta-canvas-web`: the JS canvas component — load a
  web-format project, display `page.svg` with its scoped CSS, pan, zoom
  and click-select, apply ops through the T1 engine and swap the updated
  SVG in place (R3, R4, R5). Verify: A3.
- [ ] **T3.** Browser host: an edit mode on the emitted `index.html` /
  `viewer.js` that mounts the canvas component, with the persistence
  answer from the open questions (R3, R5). Verify: A3 against a served
  project.
- [ ] **T4.** Claude Code pane, terminal surface: the `fanta-canvas`
  plugin opens a pane showing the current page as PNG frames, re-rendered
  when project files change (R4). Verify: A4 on a terminal session.
- [ ] **T5.** Claude Code pane, desktop surface: host the T2 component as
  the pane's `Client` module with file read/write over the plugin host
  (R4). Verify: A4 on the desktop app.
- [ ] **T6.** Live-app attach: when the project is open in the desktop
  app, canvas hosts route edits through the live MCP server instead of
  writing files (R6). Verify: A5.
- [x] **T7.** ([#195](https://github.com/jeanc18rlos/fanta-edit/pull/195)) Headless MCP server (R7): serve the live app's tool
  vocabulary over MCP stdio against a project directory using
  `fanta-headless` (load, apply, save, `render_png` screenshots); the
  plugin bridge and the Codex config fall back to it when no live app is
  discovered. Verify: the plugin's edit loop end to end with no app
  installed, on Claude Code and Codex. Implemented after T1 in
  dependency order: T1, T7, T2, T3, T4, T5, T6.
