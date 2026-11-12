---
name: fanta-canvas
description: Drive the running Fanta design editor over its live MCP tools — read the open document, edit the canvas with batch_design, screenshot the result. Use when asked to inspect or change a Fanta design, canvas or page.
---

# Driving the Fanta canvas

The `fanta` MCP server is the running Fanta app. Edits land on the user's
open document and autosave into the project's plain files (SVG, CSS, JSON),
so every change shows up as a git diff in the project folder.

1. Start with `get_editor_state`: the open project's name, `project_root`
   path, pages and selection. Never assume what document is open.
2. Read nodes with `batch_get` before changing them.
3. Edit with `batch_design` operations, for example
   `{"op":"create_node","node_type":"rectangle","name":"hello","x":0,"y":0,"width":100,"height":80,"fill":"#ff0000"}`
   or `{"op":"delete","id":"n_..."}`. Node ids in tool input use the
   prefixed form (`n_...`).
4. Verify visually with `get_screenshot`.
5. The project under `project_root` is reviewable with git; the project's
   own `AGENTS.md` documents its file layout.
6. The user can watch your edits live by running `/fanta-canvas`, a pane
   that re-renders the page on every save.

## The web canvas in a browser

Every saved project is also a web app: its emitted `index.html` upgrades
into a pan/zoom/drag-edit canvas when served. To give the user that view,
run the `serve-project` script in the plugin folder (two directories above
this file) in the background and tell them the URL it prints:

```sh
python3 -u <plugin-root>/serve-project <project_root> --port 0
```

The first stdout line is `serving <project> at http://127.0.0.1:<port>/`
(`-u` matters: buffered, the line only appears on exit).
No app is needed to view. Browser-side editing needs a built wasm engine
(`--engine <pkg-dir>`); when the app is running on the same project, the
browser routes edits through it automatically and needs no engine.
