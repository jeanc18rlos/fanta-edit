---
status: draft
date: 2026-11-10
owner: Jean Rojas
decisions: [0027]
---

# Store projects as plain web files

## Problem

A Fanta project is `.fnx` markup plus `.fss` stylesheets plus JSON — text,
as ADR 0004 wanted, but text in a dialect nobody else reads or writes.
[Spec 0005](../0005-view-a-project-as-plain-web-files/spec.md) made projects
*viewable* as SVG+CSS+HTML, and seeing it working, the founder set the
stronger direction (2026-11-10): opening the editor, creating a design, or
importing a `.fig` must produce **no `.fnx` or `.fss` at all** — only HTML,
SVG, CSS, JS and JSON. The web files are the design, not a view of it
([ADR 0027](../../docs/decisions/0027-project-designs-onto-plain-web-files.md)).

The gap is smaller than it reads: FNX already embeds reactions, bindings and
text styles as JSON inside its markup, and `.fss` is already a CSS dialect
carrying `fill`, `corner-radius`, `layout`, `gap`, `padding`. The custom
part — the tree and geometry markup — is what SVG does natively, and spec
0005's emitter already writes id-addressed SVG styled entirely from CSS.

## Goals

- A project folder holds only web-standard files: `fanta.json`, SVG per page
  and per component variant, `tokens.css` + `design.css`, `index.html` +
  viewer JS, JSON sidecars, `assets/`.
- The editor, the headless engine and `.fig` import write that format and
  nothing else, through one shared writer.
- The write is canonical and deterministic; the loader round-trips it.
- A declared subset of hand edits to the SVG and CSS is honored on reload —
  the adoption promise is that people and agents edit these files.
- Legacy FNX/FSS projects open unchanged and convert on save.

## Non-goals

- Keeping FNX/FSS as a parallel write format (readers stay for migration).
- Parsing arbitrary SVG/CSS from other tools; the loader accepts the dialect
  Fanta emits, plus the hand-edit subset (R7).
- Importing PSD/AI/EPS/InDesign/PDF: separate work, and those importers
  write their own print-origin profile (R12), not this one.
- Changing prototype playback; the viewer remains spec 0005's.

## Requirements

- **R1.** Creating a project and importing a `.fig` write only: `fanta.json`,
  `index.html`, viewer JS, `tokens.css`, `design.css`, `pages/<slug>.svg`
  (+ `page.json`), `components/<set>/<variant>.svg` (+ `def.json`,
  `set.json`), `doc/*.json`, `assets/`. No `.fnx`, no `.fss`, anywhere.
- **R2.** The SVG is the authoritative scene tree: element order, node ids
  (`id`), names (`data-name`), geometry, path data, text content.
- **R3.** CSS is the authoritative styling: one `design.css` rule per node;
  variable-bound properties as `var(--token, literal)`; `tokens.css` carries
  collections and modes. Engine concepts without a CSS property use
  `--fanta-*` custom properties (layout direction, gap, padding, per-corner
  radii, …) — the vocabulary `.fss` carries today.
- **R4.** Semantics SVG/CSS cannot express stay JSON sidecars: reactions,
  flows, motion, component definitions and overrides, variable registries,
  bindings beyond CSS `var()`. (All of these are JSON already — today
  embedded in FNX attributes or under `doc/`.)
- **R5.** The write is canonical: the same document produces the same bytes;
  stable ordering; no timestamps; shortest-round-trip numbers.
- **R6.** Round-trip: loading a written project yields a document
  property-equal to the one written, for every `NodeData` kind.
- **R7.** Hand-edit contract: edits inside the emitted dialect — geometry
  attributes, CSS declaration values, text content, swapping a literal for a
  `var()` — survive load and the next save. Anything the loader cannot keep
  is reported per node in `report.json`; nothing is silently dropped.
- **R8.** Auto-layout: intent lives in CSS (`--fanta-layout`, `--fanta-gap`,
  …); the solved geometry written into the SVG is derived for auto-layout
  children and regenerated on load, so a hand edit to intent wins over stale
  coordinates. Absolutely positioned nodes treat their geometry as source.
- **R9.** Legacy: FNX/FSS projects open exactly as before and convert to the
  web format on first save; the FNX/FSS readers remain until a deprecation
  record. The FNX/FSS writers are removed when the flip lands (R1).
- **R10.** One writer: the editor save path, the headless engine, cloud
  workers and `fanta-site` share the implementation; `--plain` on a
  web-format project reduces to refreshing the viewer files and report.
- **R11.** Before the default flips, every feature the canvas renders has a
  non-raster source representation: gradients, masks, booleans and instances
  as SVG/CSS/JSON, never a baked PNG standing in for the source. (Raster
  fallback remains acceptable in the *viewer*, not in storage.)
- **R12.** `fanta.json` declares the project's content profile. This spec
  implements the **design** profile (SVG+CSS+JSON) and the prototype layer
  over it (HTML+JS+JSON referencing the designs). Print-origin profiles —
  PDF or SVG + JSON for PSD, AI, EPS, InDesign and text documents — are
  reserved in the manifest and specced with their importers; nothing in
  this layout may assume every project has pages of screens.

## Acceptance criteria

- **A1.** New project in the editor: a directory listing shows only the R1
  file set; `find -name '*.fnx' -o -name '*.fss'` is empty.
- **A2.** `.fig` import: same check on a real fixture file.
- **A3.** Write → load → write is byte-identical, and load(write(doc))
  property-equals doc, on a fixture covering every node kind (R5, R6).
- **A4.** Hand-edit: change a fill in `design.css` and a width in a page
  SVG; the canvas shows both after reload; the next save keeps both (R7).
- **A5.** A legacy FNX project renders golden-identical before and after
  conversion, and no `.fnx`/`.fss` remains after the converting save (R9).
- **A6.** An auto-layout intent edit in CSS re-solves children on load; the
  stale solved coordinates do not win (R8).
- **A7.** The 51 editor/workspace CI regressions and the import suite pass
  against the new save path.

## Open questions

- **Q1.** ~~Is "pdf" in the founder's message CSS or a stored file?~~
  **Resolved by the founder, 2026-11-10:** PDF is a *source* representation,
  but only for print-origin content — InDesign files and text documents —
  never for screen designs. Recorded as R12's profile family; the design
  profile stores no PDF.
- **Q2.** Instances in SVG: expanded subtree per use-site annotated with the
  component link (bigger files, every page self-contained) versus SVG
  `<use>` into the component's file (smallest source, cross-file `<use>` is
  uneven across browsers). Draft assumes expansion; decide in design review.
- **Q3.** Does the existing `example/` runtime export fold into this layout
  or stay a separate derived directory?
- **Q4.** `index.html` per project versus per page (`pages/<slug>.html`
  wrappers) for direct deep links without the viewer.
