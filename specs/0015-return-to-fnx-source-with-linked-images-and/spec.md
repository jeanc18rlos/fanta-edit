---
status: draft
date: 2026-11-24
owner: Jean Rojas
decisions: [0032, 0033]
---

# Return to FNX source with linked images and project fonts

## Problem

On 2026-11-24 the founder could not add an image or a font to a design by
hand. Reproducing it with the project reader, `fanta-check` and both writers
(on `crates/fanta-format/tests/corpus/pristine` and
`docs/samples/auth-wizard`) showed two separate problems.

| Hand edit | Web storage (today) | FNX |
| --- | --- | --- |
| Add a new element | Deleted on the next save, no finding | Works, gets an id |
| Add an image by its path | Deleted on the next save, no finding | Whole project fails to load |
| Add an image the documented way | Hash-named file, index entry, 59-line JSON record | Hash-named file, index entry, hand-typed size |
| Use a font file from the project | Not loaded | Not loaded |
| The same 122-node page on disk | 166 KB in three files | 54 KB in one file, plus 14 KB of ids |

**Media.** Both formats accept an image only as a content-hash id backed by
`assets/index.json`, and no code loads fonts from a project
([ADR 0033](../../docs/decisions/0033-link-images-and-fonts-by-project-path.md)).

**Storage.** In web storage the readable files are a view: `nodes.json` is
the authority, so nothing added by hand to `page.svg` survives, and one fact
lives in up to three files. FNX keeps one place per fact and accepts added
elements ([ADR 0032](../../docs/decisions/0032-return-to-fnx-source-and-export-plain-web.md)).

One smaller defect surfaced on the way: opening and saving the 122-node
sample as web storage changes 11 text nodes (a text bound to a component
property, with empty `content`, comes back holding the text the SVG showed).

## Goals

- A person or an agent adds an image by writing its path, and uses a font by
  dropping its file into the project.
- A problem with one asset, font or design is reported and shown. It never
  stops the project from opening, and a save never destroys what the load
  could not read.
- A project holds FNX source and JSON. The web files are an export.
- Projects in either storage keep opening, and a web-storage project
  converts on save without loss.
- The storage format then stays put.

## Non-goals

- Changing the FNX grammar or element set, or writing shared styles as
  `.fss` text (they stay `doc/styles.json`; see Q4).
- Removing the web-storage reader. A deprecation record does that later.
- Linking by path from the editor's own importers: Place image, paste,
  `import_image` and `.fig` import keep using the managed store.
- Remote URLs as asset references.
- Font subsetting, licence checks, and WOFF or WOFF2 (see Q2).
- Per-design incremental saves. The save stays a full-tree write, as the web
  writer is today.
- Model training, evals and retrieval (specs 0010 and 0014).

## Requirements

Media:

- **R1.** An `asset` value in FNX (`Image`, `Video`, `Audio` and image fills)
  that is not an asset id must be read as a path relative to the project
  root. The loader reads the file and the canvas shows it.
- **R2.** A path link must survive a save: the writer writes the same path
  back, leaves the linked file untouched and adds no copy under `assets/`.
- **R3.** A linked path must stay inside the project. Absolute paths, `..`
  segments, URLs and symlinks that resolve outside the project root must be
  refused with a diagnostic and never read.
- **R4.** `natural_size` must be optional on read. When it is absent the size
  comes from the image file, and when the file cannot be decoded, from the
  node's own size.
- **R5.** A file in the managed store that `assets/index.json` does not list
  must load with a warning, and the next save must list it.
- **R6.** An asset problem (a missing file, a digest mismatch, an image that
  does not decode, a refused path) must be a diagnostic that names the file
  and the node, with a placeholder on the canvas. The project still opens,
  and a save must not delete, rename or overwrite an asset file the load
  could not verify.

Fonts:

- **R7.** Every `.ttf`, `.otf`, `.ttc` and `.otc` file under `fonts/` at the
  project root must be registered when the project opens, in the editor, the
  headless engine and the cloud worker, and must win over a system font of
  the same family.
- **R8.** A text node whose family is provided by neither the project, the
  bundled fonts nor the system should get a warning that names the family
  and the substitute used.
- **R9.** The web export must copy `fonts/` and the linked media, and
  declare each project font with `@font-face`.

Storage:

- **R10.** Saving a project, creating one and importing a `.fig` must write
  FNX source (`page.fnx`, `master.fnx`, id sidecars) and JSON, and no
  web-storage file (`page.svg`, `page.css`, `master.svg`, `master.css`,
  `nodes.json`, viewer files). `fanta.json` carries no `storage` field.
- **R11.** A web-storage project must open as it does today and convert on
  its next save. The document after conversion is property-equal to the one
  loaded, and no web-storage file remains.
- **R12.** The hand-edit contract for FNX: changed values, added elements
  (an id is minted), removed elements and reordered elements survive load
  and the next save, in the current layout and in the older one without id
  sidecars. Anything the loader cannot keep is a diagnostic; nothing is
  silently dropped.
- **R13.** A design whose FNX does not parse or decode must be left out with
  `design.unavailable` and never touched by a save, as spec 0009 does for
  web storage. The rest of the project opens.
- **R14.** One function produces the web export from a loaded document, into
  an output directory or into memory. `fanta-site`, `serve-project`, the
  browser canvas, the wasm engine and the plugin pane must use it and must
  not write into the project tree. Nothing reads the export back as source.
- **R15.** `fanta-check` and `check_project` must check FNX projects: no
  finding on a valid FNX project, and the hostile-edit corpus gains FNX
  cases.
- **R16.** The project guide seeded into new projects, the plugin skill,
  `fnx.d.ts`, the schema page, the README and the alpha page must describe
  FNX as the source and the web files as an export, and must give one
  consistent instruction for how an agent adds a node.
- **R17.** Saving from the browser canvas (spec 0008 R3) must write the same
  FNX bytes as the desktop app, or be switched off until it does (see Q5).

## Acceptance criteria

- **A1 (R1, R2, R4).** Given a project and a new file `images/photo.png`,
  when `<Image asset="images/photo.png" x={0} y={0} width={64} height={48} />`
  is typed into `page.fnx`, then the project loads with one more node,
  `fanta-check` reports nothing, the render shows the image, and after a
  save the element still says `asset="images/photo.png"`, the file is
  unchanged and `assets/` has no new file.
- **A2 (R3).** Given `asset="../outside.png"`, `asset="/etc/hostname"`, a URL
  and a symlink that leaves the project, then each gives a diagnostic, none
  is read and the project opens.
- **A3 (R5, R6).** Given a hash-named file with no index entry, then the
  project loads with a warning and the next save lists the file. Given a
  listed file whose bytes changed, a deleted file and a truncated PNG, then
  each gives a diagnostic naming the file and the node, the project opens,
  and a save leaves those files on disk as they were.
- **A4 (R7, R9).** Given a font file in `fonts/` whose family the machine
  does not have, and a text node that uses it, then the headless engine
  resolves the node to that file, and the export contains the file and an
  `@font-face` rule for it.
- **A5 (R8).** Given a text node with a family nothing provides, then the
  load reports a warning naming the family and the substitute.
- **A6 (R10).** A new project in the editor, a headless create and a `.fig`
  import: a listing shows no web-storage file and `fanta.json` has no
  `storage` field.
- **A7 (R11).** `crates/fanta-format/tests/corpus/pristine` and a
  web-storage copy of `docs/samples/auth-wizard` convert on save: the
  documents are property-equal before and after, no web-storage file
  remains, and exporting the converted project gives the same bytes as
  exporting the original.
- **A8 (R12).** An element added without an id to `page.fnx` survives load
  and save with a minted id, in a current-layout project and in
  `docs/samples/auth-wizard` (the older layout, which fails today with
  "missing identity sidecar"). Removing and reordering elements survives too.
- **A9 (R13).** Given a page with a syntax error, then the other pages open,
  the checker reports `design.unavailable`, and a save leaves the broken
  file's bytes unchanged.
- **A10 (R14).** Running the export, `serve-project` and the plugin pane
  against a project leaves `git status` clean in it.
- **A11 (R15).** `fanta-check docs/samples/auth-wizard` reports nothing
  (today: 15 false `ref.missing_component` errors and 28
  `manifest.file_missing` warnings).
- **A12 (R16).** A reviewer reads the seeded guide and the plugin skill and
  finds the same instruction for adding a node; no page under `docs/` or the
  README says the web files are the source.
- **A13 (R10, R17).** The editor and workspace regressions that `check.yml`
  requires pass on the FNX save path, with the list updated for any renamed
  test.

## Open questions

- **Q1.** Where do project fonts live: `fonts/` at the project root (draft)
  or under `assets/`?
- **Q2.** WOFF and WOFF2: accept them when the Skia build decodes them, or
  TrueType and OpenType only (draft)?
- **Q3.** Two open projects ship different files for one family name:
  per-project registration, or the first one wins with a warning (draft)?
- **Q4.** Should shared styles be written as `.fss` text, which the portable
  source compiler already reads, instead of `doc/styles.json`? Draft: no
  change.
- **Q5.** Saving from the browser canvas: port it to FNX in this spec, or
  switch it off here and port it later (draft)?
- **Q6.** When does the web-storage reader go? Draft: a deprecation record
  after the first release that writes FNX.
- **Q7.** The seeded project guide is 32 KB. Rewrite it for FNX here and
  shrink it with spec 0014's eval (draft), or shrink it here?

## Changes after acceptance

- None.
