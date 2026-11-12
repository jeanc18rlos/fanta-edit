# fanta-canvas-web

The host-neutral canvas component (spec 0008 T2). It renders a web-format
Fanta project with the browser's own SVG+CSS pipeline — `page.svg` plus the
stylesheets that SVG names in its `<?xml-stylesheet?>` instructions — and
draws selection above it. With a T1 wasm engine (`fanta-wasm-engine`) it also
applies `batch_design` ops, writes the re-emitted files back through the
host, and swaps the updated SVG in place without a reload.

One dependency-free ES module: [`src/fanta-canvas.js`](src/fanta-canvas.js).
Hosts supply file access; nothing here touches the network or the desktop
app.

```js
import { createFantaCanvas } from "fanta-canvas-web/src/fanta-canvas.js";

const canvas = await createFantaCanvas({
  container,                  // a block element the canvas fills
  files: {
    read,                     // async (path) -> Uint8Array | string
    write,                    // async (path, Uint8Array) — engine mode
    list,                     // async () -> [path]       — engine mode
    watch,                    // optional (onChange) -> dispose
  },
  engine,                     // optional FantaEngine from fanta-wasm-engine
  editable,                   // optional: drag-move writes through the engine
  onSelect,                   // optional ([nodeId]) => void
});
await canvas.showPage(canvas.pages[0].slug);
await canvas.applyOps([{ op: "set_props", id, fill: "#FF5522" }], "Tint");
```

- **View-only without an engine**: pan (drag), zoom (wheel, cursor-anchored,
  0.05–8×), click-select with shift multi-select, `zoomToFit`, `refresh`.
- **Editing** goes through the engine — the same document model and writer as
  the desktop app — so component instances update with their master and the
  written files are byte-identical to a desktop save (spec 0008 R3).
- Selected node ids are the display form (`n_…`), the form every Fanta agent
  tool accepts.
- `files.watch` (or a host calling `refresh()`) re-reads the current page, so
  external edits — an agent writing through `fanta-mcp`, a git checkout —
  appear live (R4).

## Demo and tests

[`demo/index.html`](demo/index.html) hosts the component over plain HTTP
with in-memory writes; its header comment has the three build-and-serve
steps. `node --test web/fanta-canvas-web/test/helpers.test.mjs` covers the
pure parsing helpers; the DOM half is exercised against a project served by
`script/serve-project`, the T3 browser host whose PUT channel persists edits
(spec 0008 A3).
