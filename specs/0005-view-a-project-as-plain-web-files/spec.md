---
status: draft
date: 2026-11-09
owner: Jean Rojas
decisions: [0027]
---

# View a project as plain web files

## Problem

A Fanta project can only be seen with Fanta. The format thesis
([ADR 0004](../../docs/decisions/0004-store-designs-as-project-folders-of-fnx-source.md))
makes a design reviewable text, but a collaborator who clones the project and
has no Mac sees FNX source, not the design. The one browser path we ship,
the website export ([website-export.md](../../docs/fanta/website-export.md)),
needs a runtime built from a second repository, loads 13.5 MB of WebAssembly
(3.4 MB brotli, measured 2026-10-21 in
[ADR 0018](../../docs/decisions/0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)),
and disables prototype interactions.

The founder's direction on 2026-11-09: people adopt tools whose files they
already know. A design should be viewable as SVG and CSS, a prototype as HTML
and JS, with several SVGs referencing each other — so that serving the folder
with any static file server shows the design, in any browser, with no app and
no runtime.

## Goals

- Cloning a saved project and serving its folder shows every page in a
  browser: correct layout, text, images, navigation between pages.
- The projected files are standard SVG, CSS, HTML and JS that other tools can
  open — a page SVG opens in Inkscape, Illustrator or an `<img>` tag.
- Every projected element names its FNX node, so agents and external tools
  can correlate what they see with the source they edit.
- The projection is deterministic: the same source produces the same bytes,
  so regenerating it never creates noise in Git.
- Design tokens survive projection: variables and modes become CSS custom
  properties, not baked-in constants.
- Simple prototype interactions (at least page navigation from
  `doc/flow_start.json` and click triggers) play with plain JS.

## Non-goals

- Replacing FNX as the authoritative format. The projection is derived
  output; the editor never reads it back
  ([ADR 0027](../../docs/decisions/0027-project-designs-onto-plain-web-files.md),
  [ADR 0009](../../docs/decisions/0009-make-project-files-authoritative.md)).
- Full-fidelity motion, video and audio playback. That remains the
  WebAssembly viewer's job ([ADR 0018](../../docs/decisions/0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)).
- Reconciling hand edits to the projected SVG back into FNX. If we ever want
  it, that is its own spec and decision.
- Importers for PSD, EPS and PDF. "Open anything, get a web-viewable
  project" is the product direction, but each importer is a separate spec
  behind the existing format registry.

## Requirements

- **R1.** Saving a project must write a web projection into a derived
  directory of the project folder: one SVG per page, one per component
  master, a shared stylesheet, an `index.html` with page navigation, and a
  small plain-JS viewer. No WebAssembly, no build step, no bundler.
- **R2.** The projection must render from any static file server
  (`python3 -m http.server` is the reference). Opened over `file://`, where
  cross-file `<use>` and `fetch` are blocked, `index.html` must say the
  folder needs serving instead of failing blank.
- **R3.** Variables and modes from `doc/variables.json` and
  `doc/active_modes.json` must project to CSS custom properties, with one
  class per mode, so switching a class switches the design's values.
- **R4.** Every projected element whose source node has an id must carry that
  id in its display form (`data-fanta-id="n_…"`), and each page SVG must name
  its source file.
- **R5.** Component instances must reference their projected master
  (`<use href>` across files) rather than inlining a copy, so the file graph
  mirrors the source's component structure.
- **R6.** Prototype start and click-to-navigate triggers must play in the
  served projection with plain JS. Unsupported triggers and actions must
  degrade to nothing, never to broken output.
- **R7.** The projection is derived only: regenerating it after deleting the
  directory restores identical bytes, the watcher ignores changes inside it,
  and nothing in the editor reads it as input.
- **R8.** Projection must be deterministic across platforms and runs: stable
  element order, stable numeric formatting, no timestamps.
- **R9.** Text must stay text (`<text>`/`<tspan>`, selectable and
  accessible), using the same font embedding and substitution policy as the
  website exporter; when no embeddable face can render a run, the export
  report says so rather than silently swapping metrics.
- **R10.** The headless engine must produce the same projection from the
  same source (`fanta-headless`), so the cloud and CI can generate it
  without the editor.
- **R11.** Unsupported visual features must be declared: the projection
  writes a report naming nodes whose rendering is approximate or missing,
  and the support matrix lives in `docs/fanta/`.

## Acceptance criteria

- **A1 (R1, R2).** Given the sample project, when its folder is served and
  opened in a browser, every page renders and the page list navigates
  between them; the directory contains no `.wasm` file.
- **A2 (R4).** For a page with nodes `n_a` and `n_b`, the projected SVG
  contains elements with `data-fanta-id="n_a"` and `data-fanta-id="n_b"`.
- **A3 (R7, R8).** Saving twice, and deleting the projection then saving
  again, each produce byte-identical files; a test proves the watcher raises
  no reconciliation for writes inside the projection directory.
- **A4 (R6).** The sample prototype's start page navigates to its target on
  click in a served browser session with JS enabled and wasm absent.
- **A5 (R3).** Toggling the projected mode class in dev tools visibly swaps
  a variable-bound fill.
- **A6 (R9).** A page using an unembeddable brand font lists the
  substitution in the export report.
- **A7 (R10).** The headless engine's projection of the sample project is
  byte-identical to the editor's.
- **A8 (R11).** A page using a feature outside the support matrix appears in
  the report with its node id.

## Open questions

- **Committed or generated?** Is the projection committed to Git (clone and
  serve works with no tooling — the adoption point) or gitignored and
  generated on demand (no derived churn in diffs)? Recommendation: committed;
  R8's determinism keeps the churn exactly proportional to real changes.
- **Where does it live?** `web/` beside `pages/`, or under the existing
  derived `previews/` root? The schema table in
  [project-schema.md](../../docs/fanta/project-schema.md) marks derived
  directories "no input to the design" — the projection must join that list.
- **How does this reshape the website export?** `fanta-site` could emit this
  projection for static pages and keep wasm only for motion — or stay
  untouched. Deciding changes ADR 0018's scope.
- **Fidelity bar for v1.** Which of blend modes, shadows, blurs, masks and
  strokes must be pixel-faithful before this ships, and what golden-image
  tolerance applies?
- **Does the first release also project `.fig` imports?** Import already
  materializes a project folder, so it would come for free — but it widens
  the fidelity surface.

## Changes after acceptance

- None.
