# Design: View a project as plain web files

Founder decisions from the 2026-11-10 session: the web files are a projection
beside the FNX source (ADR 0027 as proposed), committed to the project folder,
under `web/`.

## Approach

> **Amended during T1 (PR #183).** The first implementation used the Skia
> SVG backend as written below, but its output bakes every style into
> attributes (`fill="#111C33"` on each element), so nothing referenced
> `tokens.css` and the founder rejected it ("but I see no CSS"). T1 shipped
> the hand-written, CSS-first emitter that this document originally deferred
> to T6: elements carry only `id`/`class`/`data-name` and geometry; all
> styling lives in `design.css`, one rule per node id, with
> variable-bound properties emitted as `var(--token, literal)`. Nodes the
> emitter does not cover yet (gradients, masks, instances, booleans) fall
> back per node to a Skia-rendered PNG placed in the tree, so pages stay
> correct while the native subset grows. T6 therefore becomes coverage
> expansion, not a new emitter.

The original plan: reuse the engine's existing SVG renderer instead of a new
emitter. `fanta-headless::export` already renders any node or page to SVG
through the real Skia pipeline (`render_svg`, `export_node` with
`ExportEncoding::Svg`), so page fidelity is the renderer's fidelity from day
one.

A new `web` module in `fanta-site` (layer: export shell; it already depends
on `fanta-doc`, `fanta-format`, `fanta-text`; it gains `fanta-headless` for
the renderer) assembles the projection:

- `web/pages/<slug>.svg` — one Skia-rendered SVG per page.
- `web/tokens.css` — variables and modes from `doc/variables.json` as CSS
  custom properties, one class per mode.
- `web/index.html` — page list and viewer shell; plain hand-written HTML.
- `web/viewer.js` — navigation, pan/zoom, and prototype hotspots; plain JS,
  no dependencies, no build step.
- `web/report.json` — substitutions and unsupported features (R11).

Prototype hotspots do not require touching the SVG internals: triggers in
`doc/motion.json`/`doc/flow_start.json` name node ids, and the solved scene
gives each node's world bounds, so the shell overlays transparent anchor
rectangles on top of the page SVG. Click-to-navigate works in v1 while the
SVG stays a plain image of the page.

Generation runs in three places sharing the one implementation:
`fanta-site --plain` (CLI, T1), the editor on save (`fig_viewer`, T3), and
the headless engine (already the implementation's host, so cloud parity is
free).

## Interfaces and data

- `fanta_site::web::export_web(project: &Path, out: &Path, options) ->
  Result<WebReport>`; `out` defaults to `<project>/web`.
- `web/` joins the derived directories in
  [project-schema.md](../../docs/fanta/project-schema.md): no input to the
  design, ignored by the watcher, regenerated whole.
- Determinism (R8): stable page order from the manifest, fixed float
  formatting, no timestamps; the directory is replaced atomically like other
  derived outputs.
- Node-id metadata (R4): *as built*, every emitted element carries
  `id` (display-form node id), `class` (name slug) and `data-name`;
  `web/pages/<slug>.map.json` (node id → world bounds) still ships for
  tools that want bounds without parsing SVG.
- Text (R9): *as built*, text is emitted as `<text>`/`<tspan>` with font
  properties in CSS — selectable and restylable — but laid out by the
  browser, not the engine shaper; every text node is recorded in the
  report as that approximation until engine-shaped positions land (T6).

## Alternatives

- A hand-rolled SVG emitter walking the scene: full control over ids and
  `<text>` from day one, but reimplements fills, strokes, gradients, masks
  and blend modes the renderer already gets right. **Chosen during T1
  after all**: the Skia backend's baked attributes defeated R5 (styling in
  CSS), and the per-node raster fallback keeps the renderer as the safety
  net instead of reimplementing everything at once.
- Serving through the wasm runtime (status quo, ADR 0018): perfect fidelity
  and motion, but 13.5 MB of runtime and no plain files; stays the
  full-fidelity path.
- Decision with lasting consequences: [ADR 0027](../../docs/decisions/0027-project-designs-onto-plain-web-files.md)
  (proposed) — projection beside the source, not a replacement format.

## Risks

- Committed derived churn: an engine rendering change rewrites every page
  SVG. Noticed by diff size in review; backed out by regenerating from the
  older commit. Determinism tests keep noise at zero for a no-op save.
- Fidelity drift between canvas and SVG backend: golden comparisons (A1)
  catch it; the report (R11) names what is approximate.
- `fanta-site` grows a renderer dependency: checked against
  `docs/architecture/layers.toml` by `script/architecture verify`.

## Verification

- A1, A4: a fixture project exported in a test; assertions on files present,
  no `.wasm`, hotspot anchors resolving to existing pages; a served smoke
  check in the release `build` stage.
- A2 (as amended by the id-map interface): the map file lists every node id
  on the fixture page.
- A3, A7: byte-identical double export in tests, editor-vs-headless
  comparison test.
- A5: tokens.css asserted to contain each variable under each mode class.
- A6, A8: report content asserted for a brand-font fixture and an
  unsupported-feature fixture.
