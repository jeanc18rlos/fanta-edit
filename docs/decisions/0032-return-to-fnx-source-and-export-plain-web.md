---
status: proposed
date: 2026-11-24
decision-makers: Jean Rojas
supersedes: [0027]
tags: [format, product, web]
---

# Return to FNX source and export plain web files

## Context

[0027](0027-project-designs-onto-plain-web-files.md) made SVG, CSS and JSON
the stored form of a design on 2026-11-10, and
[spec 0007](../../specs/0007-store-projects-as-plain-web-files/spec.md)
shipped it. Using it showed that the readable files are not the source. Each
design is written three times: `page.svg`, `page.css` and a complete node
record in `nodes.json`. On load the records are read first and a declared
subset of SVG and CSS values is laid over them (spec 0007 R7). `nodes.json`
is therefore the authority, and the project guide tells people and agents
never to write it by hand.

On 2026-11-24 the founder could not add or link an image by hand. Measured
that day with the project reader, `fanta-check` and both writers, on
`crates/fanta-format/tests/corpus/pristine` and `docs/samples/auth-wizard`:

- An `<image>` element typed into `page.svg` passes `fanta-check` with no
  finding, loads, and is gone after the next save. With an `id` it is dropped
  with `id.not_a_node` or `record.missing`. Nothing added by hand survives.
- An image fill is not read back from the web files. It lives only in
  `nodes.json`; in the fixture the node has no element in `page.svg` at all.
- Opening and saving the 122-node sample changes 11 text nodes: a text bound
  to a component property, with empty `content`, comes back holding the text
  the SVG showed.
- The same page is 166 KB in three files as web storage, and 54 KB as one
  `page.fnx` plus a 14 KB id sidecar.
- With the FNX writer, which still works (`write_legacy_project_tree`), an
  element added without an id loads and gets one.
- Converting a web-storage project to FNX and back gives the same bytes as
  saving it as web storage directly, on both fixtures.

Outside evidence points the same way. Penpot built an SVG-plus-JSON file
format and never released it. Benchmarks of models editing vector files find
the failures concentrated in coordinate-level SVG edits, while JSX scores
about level with plain HTML for strong models.

This record does not settle images and fonts. Both formats require a
hash-named file listed in `assets/index.json`, and neither loads fonts from
the project. [0033](0033-link-images-and-fonts-by-project-path.md) decides
that.

The founder decided on 2026-11-24 to return to FNX as the source and keep the
web files as an export.

## Decision

Store a design as FNX source again, as
[0004](0004-store-designs-as-project-folders-of-fnx-source.md) decided: one
`page.fnx` per page and one `master.fnx` per component master, with their id
sidecars and the JSON files under `doc/`. The editor, the headless engine,
the cloud worker and `.fig` import write this into a project and nothing
else.

Generate the web files (SVG, CSS, HTML and the viewer) on demand as an
export. They go to an output directory or stay in memory, never into the
project tree, and nothing reads them back as source. `fanta-site`, the
browser canvas and the agent-host pane display that export.

A project saved as web storage keeps opening and converts to FNX on its next
save. The web-storage reader stays until a deprecation record removes it.

This is the last change of storage format before the first users. A later
change needs a new record that cites evidence from people using the product.

## Options considered

1. **FNX source, web files as an on-demand export** (chosen). One place per
   fact, so hand edits that add structure work. A repository holds one
   format, which answers the founder's 2026-11-10 objection to a projection
   ("two formats in every repository").
2. **Keep web storage and teach the reader to adopt hand-written SVG.** The
   cheapest step this week. It loses because every feature then needs a
   projection writer, a reader and a rule for which of two copies wins;
   image fills, instances, booleans and paints beyond the SVG subset have no
   editable spelling in SVG; and three files per design triple every diff.
3. **One JSON document per design.** This is what `nodes.json` already is:
   64 lines for one rectangle. 0004 rejected it as unreadable to review.
4. **HTML and CSS as the source.** Familiar to every person and model. It
   needs a browser-grade layout and paint engine, and Fanta's engine is a
   scene graph on Skia
   ([0015](0015-render-the-metal-canvas-with-skia-graphite.md)).

## Consequences

- Good: a person or an agent can add, remove and reorder nodes in the file.
  The source is typed by `fnx.d.ts`, and a review reads one file per design.
- Good: source edits from the Code workspace have a save path on every
  project again (`apply_source_edit` writes FNX; SVG buffers never got one).
- Good: the export keeps what 0027 wanted for viewing: export or serve the
  project and see the design in a browser.
- Bad: a cloned project cannot be viewed in a browser without running the
  exporter. The adoption story changes from "you already know these files"
  to "readable source that exports to web files".
- Bad: this is the third decision on the storage format in five months. The
  work spent on the web reader's round-trip guarantee now serves migration
  and the export only.
- Bad: `fanta-check`, the project guide, the schema page and the tests that
  assert "no `.fnx`" must be reworked, and
  [spec 0008](../../specs/0008-a-web-canvas-for-browsers-and-agent-hosts/spec.md)'s
  rule that the browser canvas saves the same bytes as the desktop app now
  means FNX bytes.
- Not changed: FSS. The project writer has never written `.fss` files. Shared
  styles persist as `doc/styles.json` (schema 4, #132) and a node's own style
  is inline in its element. `.fss` text is read only by the portable source
  compiler (`crates/fanta-format/src/runtime.rs`).
- Not carried over: 0027's profile family for print-origin content (PDF or
  SVG plus JSON). It gets its own record when an importer is specced.
  [0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md)
  stays narrowed to full-fidelity embedding, and
  [0009](0009-make-project-files-authoritative.md) is unchanged.
- Follow-up:
  [spec 0015](../../specs/0015-return-to-fnx-source-with-linked-images-and/spec.md),
  and notes on specs 0005, 0007 and 0008. When this record is accepted, 0027
  becomes `superseded`.

## Confirmation

Spec 0015's acceptance criteria in CI: a saved project and a `.fig` import
contain no web-storage file; a web-storage project converts on save to a
property-equal document; an element added by hand survives load and save; the
export writes nothing into the project tree; `fanta-check` reports no finding
on the FNX fixtures.

## References

- [Spec 0015](../../specs/0015-return-to-fnx-source-with-linked-images-and/spec.md),
  [spec 0007](../../specs/0007-store-projects-as-plain-web-files/spec.md),
  [spec 0008](../../specs/0008-a-web-canvas-for-browsers-and-agent-hosts/spec.md)
- [0004](0004-store-designs-as-project-folders-of-fnx-source.md),
  [0009](0009-make-project-files-authoritative.md),
  [0027](0027-project-designs-onto-plain-web-files.md),
  [0033](0033-link-images-and-fonts-by-project-path.md)
- Founder decision recorded in session notes, 2026-11-24
- [Penpot file format history](https://help.penpot.app/technical-guide/developer/data-model/penpot-file-format/),
  [SVGEditBench V2](https://arxiv.org/html/2502.19453v1),
  [DesignBench](https://arxiv.org/html/2506.06251v3). Research agents read
  these on 2026-11-23; they were not re-read line by line for this record.
