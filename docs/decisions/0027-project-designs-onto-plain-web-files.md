---
status: proposed
date: 2026-11-09
decision-makers: Jean Rojas
tags: [format, product, web]
---

# Project designs onto plain web files

## Context

Adoption pressure: people prefer files they already know over a new format,
and today a cloned Fanta project cannot be seen without Fanta. The founder's
direction on 2026-11-09 is that a design should be viewable as SVG+CSS and a
prototype as HTML+JS by serving the project folder — and, by extension, that
Fanta could turn `.fig`, PSD, EPS and PDF files into such projects. The
existing browser path needs a 13.5 MB WebAssembly runtime from a second
repository ([0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)).
The question with lasting consequences: are the web files the design, or a
view of it?

## Decision

Generate plain web files — SVG pages, a CSS file of tokens, an HTML+JS
viewer — as a deterministic projection beside the FNX source, on every save.
FNX remains the single authoritative format
([0004](0004-store-designs-as-project-folders-of-fnx-source.md),
[0009](0009-make-project-files-authoritative.md)); the editor never reads the
projection back. Scope and requirements live in
[spec 0005](../../specs/0005-view-a-project-as-plain-web-files/spec.md).

## Options considered

1. **A projection beside the source** (chosen). Anyone can view the design
   with a static server; agents get standard files annotated with node ids;
   FNX keeps the semantics SVG cannot carry and the reviewable diffs ADR
   0004 was chosen for.
2. **SVG+CSS as the authoritative format.** Strongest version of "normal
   files", but SVG cannot express auto-layout, component overrides, variable
   bindings or the prototype graph; all of it becomes metadata that
   Illustrator or Inkscape silently destroys on the first edit, and a single
   auto-layout change rewrites every sibling's coordinates, burying review.
   It trades the format thesis for viewability.
3. **Keep WebAssembly-only viewing** (status quo). One renderer, perfect
   fidelity — but viewing needs a runtime built from a second repository,
   prototypes are disabled, and nothing about the project is legible to the
   web. This is the adoption problem, not a solution to it.

## Consequences

- Good: clone + serve = see the design, in any browser, with zero runtime.
  The projection doubles as a preview surface for agent plugins and a
  low-cost replacement for most static website exports.
- Bad: a second rendering of every page to keep faithful — SVG/CSS fidelity
  (blend modes, filters, text metrics) will trail the Skia canvas and needs
  a declared support matrix; committed derived files add churn that only
  strict determinism keeps proportional.
- Follow-up: spec 0005's tasks; revisit ADR 0018's website-export scope once
  the projection covers static pages.

## Confirmation

Spec 0005's acceptance criteria in CI: byte-determinism tests, the
no-read-back watcher test, and golden comparisons between the canvas and the
projected pages.

## References

- [Spec 0005](../../specs/0005-view-a-project-as-plain-web-files/spec.md)
- [0004](0004-store-designs-as-project-folders-of-fnx-source.md),
  [0009](0009-make-project-files-authoritative.md),
  [0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)
- Founder direction recorded in session notes, 2026-11-09
