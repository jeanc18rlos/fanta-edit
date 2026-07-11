---
name: fanta-design
description: >-
  Design real UI on the Fanta canvas with the native design tools — build
  screens, cards, and layouts in the open .fig/Fanta project. Use when the
  user wants to lay out a UI, design a screen or component, restyle nodes, or
  match a reference mockup on the canvas. Covers the state → edit → screenshot
  loop, canvas op mechanics, FNX source editing for advanced styling, depth &
  material, and typography.
---

# Designing on the Fanta canvas

The design canvas renders the open `.fig` file or Fanta project. Three native
tools drive it — no MCP setup required:

- **`design_state`** — read the document: pages, selection, viewport, node
  trees (`page`), or full node detail (`nodes`). Ids and geometry come from
  here; never guess them.
- **`design_edit`** — apply a batch of ops as ONE undoable transaction. If any
  op fails the whole batch rolls back and the result names the failing op —
  fix it and resend.
- **`design_screenshot`** — render a PNG of a page or a node's region into the
  thread. This is your eyes; use it.

The user must have a `.fig` file or Fanta project open; the tools target the
most recently focused canvas. If they error with "no design canvas", ask the
user to open one.

## The loop

1. **Inspect** — `design_state` (no args) for the overview, then
   `design_state {page: N}` for the node tree of the page you'll work on.
2. **Build** — `design_edit` with an ops batch. Give each batch a `label`
   (it becomes the undo step's name).
3. **Verify** — `design_screenshot {node: <frame-id>}` after each substantive
   batch and READ the image. Coordinates lie; pixels don't.
4. **Refine** — target nodes by id from the returned `created` list or a
   fresh `design_state`.

## Canvas ops (`design_edit`)

- `create_node` — `node_type`: `frame` (clipping container with background),
  `rectangle`, `ellipse`, `text`. `x`/`y` are world coordinates of the
  top-left corner; `width`/`height` required; `fill` is `#RRGGBB` or
  `#RRGGBBAA` (for text it's the glyph color); text nodes take `text` and
  `font_size`. Pass `parent` (a frame/group id) to nest — omit it and the
  node lands on the active page root.
- `set_props` — change only the provided fields: `name`, `x`, `y`, `width`,
  `height`, `opacity`, `fill`, `corner_radius` (rectangles/frames), `text`
  (text nodes), `hidden`, `locked`.
- `reparent` — move a node under a new parent, preserving its world position.
  `index` 0 is the bottom of the stack; omit to place on top.
- `delete` — removes the whole subtree. `select` — sets the editor selection
  (useful to show the user what you changed). `set_viewport` — persisted
  document viewport.

Mechanics that matter:

- **Nest as you create.** Create the outer frame first, then pass its id as
  `parent` for children (its id is in the same batch's result only AFTER the
  batch runs — so create structure in one batch, read `created`, then fill it
  in the next batch, or compute absolute coordinates and `reparent`).
- **Z-order is creation order** within a parent: later nodes render on top.
  Backgrounds first, content after.
- **Coordinates are world-space** even for nested children — the tools rebase
  them under the parent for you.
- **Size text boxes for their content.** A text node keeps its created box;
  text wraps to the box width. Default type is Inter 16px black.

## When ops aren't enough: edit the FNX source

Auto-layout stacks, gradients, shadows, blurs, strokes, per-corner radii,
component instances, and variable bindings are not covered by canvas ops yet.
For those, edit the project's `.fnx` sources directly with the normal
`read_file`/`edit_file` tools:

- Sources live in the project directory: `pages/<page-id>/page.fnx` and
  `components/<cid>/master.fnx` (JSX-like: element = node, attributes = the
  node's serde fields). `design_state` (no args) reports `project_root`.
- The canvas hot-reloads the file ~300ms after a save; then
  `design_screenshot` to verify.
- Do NOT mix lanes mid-flight: while an FNX buffer has unsaved edits the
  canvas is locked (`design_edit` will refuse with `source_edit_locked`).
- A freshly opened `.fig` has no source tree until the user saves it as a
  Fanta project; canvas ops still work in memory.

## Depth & material — don't ship flat

Solid gray rectangles read as "placeholder". Even in phase-1 ops you have
opacity, layered fills, and rounded corners; the FNX lane opens shadows,
gradients, and blurs:

- Lift cards and floating panels with a soft drop shadow (FNX `effects`):
  low-alpha black (`#00000026`), cards ~`dy 6, blur 18`, popovers
  `dy 14, blur 40`.
- Two close gradient stops beat one solid on heroes and primary buttons.
- Layer translucent white/black rectangles over a base fill for cheap
  elevation when staying in canvas ops.
- One consistent corner-radius scale (e.g. 8 for cards, 6 for buttons).

## Typography

The house font is **Inter** (bundled, the default — don't set a family for
UI). Hierarchy comes from size + weight, not from more fonts: titles
600/16–20, body 400/14, captions 400/12 muted. Weight and family changes are
FNX-lane edits for now; `font_size` and color are available at creation.

## Scale to the ask

A quick layout → one or two `design_edit` batches + one screenshot. A full
screen → structure batch (frames), content batches (text/shapes per section),
a screenshot per section, and FNX passes for auto-layout and depth. Keep
frames ≤ 1920×1080 and screenshot a FRAME or node, not a sprawling page.
