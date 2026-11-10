---
status: proposed
date: 2026-11-10
decision-makers: Jean Rojas
tags: [format, product, web]
---

# Store projects as plain web files

## Context

Adoption pressure: people prefer files they already know over a new format,
and today a cloned Fanta project cannot be seen without Fanta. The founder's
direction on 2026-11-09 was that a design should be viewable as SVG+CSS and a
prototype as HTML+JS by serving the project folder. A first draft of this
record chose a projection beside the FNX source; on 2026-11-10, after seeing
the CSS-first projection working, the founder went further: creating a design
or importing a `.fig` must produce **no `.fnx` or `.fss` at all** — only
HTML, SVG, CSS, JS and JSON. The question with lasting consequences: are the
web files the design, or a view of it?

Two facts make the stronger direction cheaper than it first looks. FNX
already stores everything non-visual (reactions, bindings, text styles) as
JSON embedded in its markup, and `.fss` is already a CSS dialect
(`.s_… { fill: #AABBCC; corner-radius: 12; layout: vertical; gap: 4; }`).
The only genuinely custom syntax in the format is the tree-plus-geometry
markup — which is exactly what SVG expresses natively. The existing browser
path needs a 13.5 MB WebAssembly runtime from a second repository
([0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)).

## Decision

Make plain web files the authoritative project format, in stages. A project
is `fanta.json`, one SVG per page and per component variant (structure,
geometry, ids), `design.css` + `tokens.css` (styling and variables, with
engine concepts as `--fanta-*` custom properties, the role `.fss` plays
today), JSON sidecars (reactions, flows, motion, component definitions,
variables — all JSON already), an HTML+JS viewer, and `assets/`. The editor,
the headless engine and `.fig` import write this format and nothing else;
`.fnx`/`.fss` are no longer written once
[spec 0007](../../specs/0007-store-projects-as-plain-web-files/spec.md)
lands. Legacy projects keep opening and convert on save.

The staging: [spec 0005](../../specs/0005-view-a-project-as-plain-web-files/spec.md)'s
CSS-first emitter (shipped as `fanta-site --plain`) becomes the writer;
spec 0007 adds the reader, the round-trip guarantees and the editor flip.

When accepted, this record supersedes the format choice of
[0004](0004-store-designs-as-project-folders-of-fnx-source.md) — the thesis
"a design is source code" stands; the source language changes from FNX/FSS
to SVG/CSS/JSON — and narrows
[0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)
to full-fidelity embedding. [0009](0009-make-project-files-authoritative.md)
(files are authoritative) carries over unchanged.

## Options considered

1. **A projection beside the FNX source** (first draft's choice). Anyone can
   view the design with a static server while FNX keeps the semantics.
   Rejected by the founder on 2026-11-10: it leaves two formats in every
   repository, the custom one still authoritative, so the adoption story
   ("you already know these files") stays half true.
2. **SVG+CSS+JSON as the authoritative format** (chosen). The strongest
   version of "normal files". Its real costs, and how they are carried:
   auto-layout, overrides and the prototype graph do not fit SVG — they stay
   JSON and `--fanta-*` properties, which is where FNX kept them anyway
   (embedded JSON); outside editors may destroy that metadata — the loader
   parses only the dialect we emit, honors a declared subset of hand edits
   and reports, never silently drops, the rest; an auto-layout change
   rewrites solved sibling geometry — solved geometry of auto-layout
   children is derived (regenerated on load), so review reads the intent
   change in CSS, not the coordinate churn.
3. **Keep WebAssembly-only viewing** (status quo). One renderer, perfect
   fidelity — but viewing needs a runtime built from a second repository and
   nothing about the project is legible to the web. This is the adoption
   problem, not a solution to it.

## Consequences

- Good: clone + serve = see the design; create or import = web files from
  the first commit. One source format, standard by construction; agents and
  humans edit SVG/CSS directly, which is the product thesis.
- Bad: fanta-format gains an SVG+CSS reader and a canonicalizing writer, and
  must hold a round-trip guarantee FNX never needed; text keeps engine
  shaping semantics (an SVG `<text>` has no wrap), so browsers approximate
  what the canvas renders and the support matrix must say so.
- Migration: the FNX/FSS readers stay until a deprecation record; writers go
  as soon as spec 0007's flip lands.
- Follow-up: spec 0005's T3+ tasks fold into spec 0007; revisit the
  `example/` runtime export's place in the layout.

## Confirmation

Spec 0007's acceptance criteria in CI: a new project and a `.fig` import
contain no `.fnx`/`.fss`; byte-deterministic writes; property-level
round-trip equality over every node kind; the hand-edit contract tests;
legacy-project golden comparisons before and after conversion.

## References

- [Spec 0007](../../specs/0007-store-projects-as-plain-web-files/spec.md),
  [spec 0005](../../specs/0005-view-a-project-as-plain-web-files/spec.md)
- [0004](0004-store-designs-as-project-folders-of-fnx-source.md),
  [0009](0009-make-project-files-authoritative.md),
  [0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)
- Founder direction recorded in session notes, 2026-11-09 and 2026-11-10
