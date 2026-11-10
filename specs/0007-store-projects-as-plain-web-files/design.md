# Design: Store projects as plain web files

Draft sketch for Gate 1; detail lands after the spec is accepted.

## What maps where

| Today | Web format | Notes |
| --- | --- | --- |
| `pages/<slug>/page.fnx` tree + geometry | `pages/<slug>/page.svg` | element order, `id`, `data-name`, transforms, path data, text content |
| `.fss` rules (`fill`, `corner-radius`, `layout`, `gap`, `padding`) | `design.css` (+ `--fanta-*` for engine concepts) | near 1:1 — FSS is already a CSS dialect |
| variables (`doc/variables.json`) | `tokens.css` as source of values; registry structure stays JSON | spec 0005's tokens emitter, inverted to read back |
| reactions/bindings embedded as JSON in FNX attributes | JSON sidecars (`pages/<slug>.interactions.json` or `doc/`) | content unchanged — it is JSON today |
| `components/<set>/<variant>/master.fnx` | `components/<set>/<variant>/master.svg` | `def.json`/`set.json` stay |
| `doc/*.json`, `fanta.json`, `assets/` | unchanged | `fanta.json` version bump marks the format |

## Already built

- The writer's core: spec 0005 T1's CSS-first emitter (`fanta-site`) —
  id-addressed SVG, per-node CSS rules, `var()` for bindings, deterministic
  output. It moves into `fanta-format` as the canonical writer (R10).
- The vocabulary: the `.fss` property set becomes the `--fanta-*` set.
- The viewer: spec 0005's `index.html`/`viewer.js` ship inside the project.

## New work

- An SVG+CSS reader in `fanta-format`: parses the emitted dialect (not
  arbitrary SVG) plus the R7 hand-edit subset into the scene graph; shares
  property parsing with the existing `.fss` reader where the names overlap.
- Canonicalization: load-then-save normalizes hand-edited formatting, like
  a formatter; property values survive, whitespace does not.
- The flip: editor save, headless engine and `.fig` import write through
  the new writer; legacy conversion on save (R9).

## Risks

- Dual truth for geometry: solved coordinates in SVG versus layout intent in
  CSS. Rule R8 resolves it (auto-layout children's geometry is derived);
  the same duality exists today between FNX `x/y` and FSS `layout`.
- Text: SVG `<text>` has no wrapping; the box and content are source, the
  engine shapes, browsers approximate. Declared in the support matrix.
- Diff churn: an intent edit must not rewrite sibling coordinates noisily —
  R8 plus canonical write keep the diff at the intent line; a golden diff
  test guards it.
- Outside editors rewriting files wholesale (Illustrator re-serializes
  everything): the loader reports what it cannot keep (R7) instead of
  guessing; the support matrix tells users which tools are round-trip-safe.

## Verification

Per acceptance criteria A1–A7: file-set assertions on create and import,
byte and property round-trips over every node kind, hand-edit fixtures,
legacy golden conversions, the auto-layout re-solve test, and the existing
CI regression suites against the new save path.

## T1 decisions (recorded after implementation, PR #186)

- **`nodes.json` beside each SVG**: the complete serialized node records of
  the design, keyed by the persisted (bare ULID) id form in sorted order.
  This makes the web format lossless from day one — the SVG+CSS pair is the
  readable projection, and T2's reader lets parsed SVG/CSS fields win over
  the record. T4 shrinks the sidecar as native SVG/CSS coverage grows; it is
  not a second source of truth for anything the SVG expresses.
- **Two-phase raster API**: `fanta-format` sits in the persistence layer and
  cannot render. `write_web_project_tree` returns `NeedsRasters(nodes)` for
  node kinds outside the SVG subset; the caller renders PNGs
  (`fanta-headless::export_node`) and calls again. Bounds come in the same
  way (`page_visual_bounds`), with the scene's world bounds as fallback.
- **Shared transactional sync**: the web writer reuses the FNX writer's
  sync (extracted as `sync_project_files`), and the web file set joins
  `is_generated_artifact` — so stale pruning converts a project between
  storage formats by deleting the other format's files (A5) with no extra
  machinery.
- **`fanta.json` marks storage** with an optional `"storage": "web"` field
  instead of a version bump: older binaries that drop the field simply fail
  to find `.fnx` sources and refuse, which is the intended behavior.
