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
