---
status: accepted
date: 2026-11-11
owner: Jean Rojas
decisions: []
---

# A web canvas for browsers and agent hosts

## Problem

Since spec 0007 a project *is* web files: every page is `page.svg` + scoped
CSS a browser renders natively, and every project ships `index.html`,
`viewer.js` and `pages.js`. Viewing is solved. Editing is not: the only
canvas is the desktop app. The founder wants two more canvases (2026-11-11):

- a browser editor — open a project on the web and edit it like Figma;
- the canvas embedded in agent hosts — the Claude Code plugin should show
  and update the design beside the chat, as seamlessly as the canvas sits
  inside the Zed-fork app, instead of only bridging tools to the desktop
  app (PR #185).

Feasibility, probed on this checkout (2026-11-11):

- `fanta-doc` and `fanta-design-ops` compile unchanged to
  `wasm32-wasip2`; on `wasm32-unknown-unknown` the only failure is
  `getrandom`'s browser backend cfg. The document model and the whole
  design-op vocabulary are WebAssembly-ready today.
- `fanta-format` does not compile to wasm: its project I/O (atomic writes,
  `tempfile`, directory scans) assumes a filesystem. Its emitters and
  readers are string transforms and are separable.
- `fanta-runtime` already builds a WebAssembly playback binary for
  compiled prototypes; it depends only on `fanta-bundle` and stays
  playback-only.
- Claude Code plugin surfaces: the desktop app draws `Svg` and plugin
  `Client` modules (interactive web content) in panes; the terminal draws
  `Image` (PNG by file path, cheap frame swap) and `Client`; VS Code and
  mobile draw `Svg`.

## Goals

- One canvas implementation, `fanta-canvas-web`: a JS component plus a
  WebAssembly engine that loads a web-format project, renders it through
  the browser's own SVG+CSS pipeline, and applies the same design-op JSON
  the MCP tools accept.
- A browser host: serve or open a project folder, edit on the canvas, and
  save files byte-identical to what the desktop app would write.
- A Claude Code host: the `fanta-canvas` plugin opens a pane; on the
  desktop surface it hosts the interactive canvas as a `Client` module, on
  the terminal surface it shows a live PNG of the current page, refreshed
  when project files change.

## Non-goals

- Desktop-app parity: panels, variables workspace, templates, `.fig`
  import and text shaping stay in the app. The web canvas edits what the
  design-op vocabulary can express.
- Prototype playback (`fanta-runtime` covers it) and multiplayer editing.
- A hosted service: this spec ends at static files and local hosts.

## Requirements

- **R1.** The engine crates needed by the web canvas (`fanta-doc`,
  `fanta-design-ops`, and the read/emit core of `fanta-format`) must
  compile to browser WebAssembly. `fanta-format`'s filesystem code moves
  behind a default-on cargo feature; the wasm build takes sources as
  bytes in and returns written files as bytes out.
- **R2.** The wasm engine's edit interface is the design-op JSON the MCP
  `batch_design` tool accepts — one vocabulary everywhere, no parallel op
  set.
- **R3.** Rendering is the project's own files: the canvas displays
  `page.svg` with its scoped CSS unmodified. After an edit the engine
  re-emits through the one writer (spec 0007 R9/R10), so a file saved
  from the browser is byte-identical to the desktop app saving the same
  document.
- **R4.** The canvas component is host-neutral: browser page and Claude
  desktop `Client` pane embed the same component; hosts provide only file
  read/write and change notification. The terminal pane fallback renders
  the current page to PNG and swaps frames on project-file change.
- **R5.** The web canvas must not require the desktop app or any network
  beyond the host serving the project's own files.
- **R6.** When the desktop app has the same project open, canvas hosts
  attach through the live MCP server instead of writing files directly,
  so the app's session stays the owner of unsaved state.
- **R7.** A headless MCP server serves the live app's tool vocabulary
  (`get_editor_state`, `batch_get`, `batch_design`, `get_screenshot`, …)
  against a project directory with no app installed, over MCP stdio, so
  Claude Code and Codex work standalone. The agent bridges prefer the
  running app (R6) and fall back to this server.

## Acceptance criteria

- **A1 (R1).** `cargo check --target wasm32-unknown-unknown` passes for
  the wasm engine package in CI.
- **A2 (R2, R3).** A test feeds the wasm engine a project, applies a
  `batch_design` op list, and the returned files equal the bytes the
  native writer produces for the same document.
- **A3 (R3).** Opening a project in the browser host shows the same page
  pixels as `index.html` does today; an edit updates the canvas without a
  reload.
- **A4 (R4).** The Claude Code plugin opens its pane: interactive canvas
  on the desktop surface, PNG frames on the terminal surface, both
  updating when the project changes on disk.
- **A5 (R6).** With the app running on the project, an edit made on a
  canvas host round-trips through the live MCP server (visible in the
  app) rather than through direct file writes.

## Open questions

- Browser persistence: File System Access API (Chromium-only today)
  versus a small local dev-server with PUT, versus download-on-save.
- Where the wasm bundle lives for plain static hosting: copied into each
  project's `exports/` (self-contained, but stale copies) or fetched from
  a pinned URL (fresh, but a network dependency against R5).
- Text editing scope in v1: geometry and style ops only, or full text
  runs (shaping differences between browser text and `fanta-text` may
  break byte-identity for text-heavy saves).

## Changes after acceptance

- 2026-11-11, founder: accepted as "Go with all Ts", with the
  requirement sharpened to "the app can work independently inside
  claude and codex, without needing the fanta app" — recorded as R7 and
  Codex added as a host. `fanta-headless` already renders PNG/JPEG/SVG
  and `fanta-engine` already applies design steps without the app, so
  R7 wraps existing machinery.
