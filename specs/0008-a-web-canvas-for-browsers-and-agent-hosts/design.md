# Design: A web canvas for browsers and agent hosts

## Approach

Three layers, outermost first:

- **Hosts** own files and chrome: the browser page (served project), the
  Claude Code plugin pane (`Client` module on the desktop surface, PNG
  `Image` frames on the terminal surface), and later other agent hosts. A
  host gives the component three callbacks: read file, write file, watch
  for changes.
- **`fanta-canvas-web`** (new, JS): the canvas component. It injects the
  project's own `page.svg` + scoped CSS into the DOM — the browser is the
  renderer, exactly as spec 0007 intends — and draws selection and
  handles above it. No drawing code of our own.
- **Wasm engine** (new crate, engine layer): `fanta-doc` +
  `fanta-design-ops` + `fanta-format`'s read/emit core compiled to
  `wasm32-unknown-unknown`. API: `load(files) -> doc`,
  `apply(doc, batch_design_json) -> changed files`. The one writer keeps
  browser saves byte-identical to desktop saves (spec 0007 R9/R10).

Probed 2026-11-11 on this checkout: `fanta-doc` and `fanta-design-ops`
already compile to `wasm32-wasip2` unchanged; browser wasm needs only the
`getrandom` backend cfg; `fanta-format` needs its filesystem half
feature-gated (`tempfile`, atomic writes, directory scans).

## Interfaces and data

- Edit interface: the `batch_design` op JSON, unchanged. One vocabulary
  for MCP tools, the wasm engine and any host.
- File interface: the spec 0007 web file set, unchanged. The engine takes
  `{path: bytes}` in and returns `{path: bytes}` out; hosts move bytes.
- Attach interface: `<data-dir>/fanta_live_mcp.json` discovery plus the
  live MCP socket, as in the agent plugin, when the desktop app owns the
  project (R6).

## Alternatives

- Compile the whole app's renderer (Skia/GPUI) to wasm: enormous, and
  needless — the files are already a browser-native format.
- A JS-only editor that edits the SVG DOM directly: loses byte-determinism
  and the single-writer contract; drifts from the document model.
- Keep the plugin tools-only and rely on screenshots in chat: that is
  today's state; the founder asked for an embedded canvas.

## Risks

- Byte-identity for text-heavy pages (browser text metrics versus
  `fanta-text`); noticed by A2/A3, backed out by scoping v1 ops away from
  text runs.
- Browser persistence is platform-uneven (File System Access API);
  isolated in the host layer, so a weaker fallback (download-on-save)
  cannot leak into the component or engine.
- Wasm binary size from `fanta-doc`'s dependency tree; measured in T1 CI;
  backed out by trimming features before the component ships.

## Verification

- A1: a CI job checks the wasm package on `wasm32-unknown-unknown`.
- A2: native test comparing engine output bytes with the native writer.
- A3: served-project smoke test driving the canvas and asserting the DOM
  updated without reload.
- A4: plugin pane exercised on a terminal session and on the desktop app.
- A5: live-app attach exercised against a running app over the MCP
  socket, as `script/smoke-mcp` does.
