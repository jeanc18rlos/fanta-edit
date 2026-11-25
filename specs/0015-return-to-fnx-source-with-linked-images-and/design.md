# Design: Return to FNX source with linked images and project fonts

## Approach

The order matters: media first because it is needed in either storage, then
everything that reads FNX, then the export, and the save flips last so no
tool is left without files to show.

**Linked media (R1 to R6)** in `fanta-doc` and `fanta-format`:

- `fanta_doc::ProjectAsset` (`crates/fanta-doc/src/doc.rs`) gains
  `path: Option<String>`. An entry with a path is a linked asset.
- The FNX reader (`crates/fanta-format/src/project/read.rs`) resolves `asset`
  values in a pass over the decoded element before it is deserialized, which
  is where a path fails today with `invalid length`. A value that is not an
  `AssetId` is validated (R3), read, hashed to an id the way `media.rs` mints
  one, added to the asset map and recorded in `doc.asset_library` with its
  path. A file that is missing or refused keeps a placeholder id derived from
  the path, so the link survives a save.
- The FNX writer writes the path for a linked asset and skips it when writing
  `assets/<family>/` and the index. The export's `asset_hrefs`
  (`crates/fanta-format/src/project/web.rs`) uses the path as the href.
- `natural_size` becomes optional in the FNX decode and falls back to the
  node's size. `fanta-format` has no image decoder, so the real size is
  filled in by `fanta-headless`, which both hosts open projects through and
  which already reads image headers under a pixel limit
  (`crates/fanta-headless/src/assets.rs`).
- `AssetIndex::verify_entry` (`media.rs`) reports instead of failing when the
  reader has a diagnostics sink: an unlisted file is a warning, a digest
  mismatch is an error and the bytes are not loaded. The writer's stale-file
  pruning keeps every file under `assets/` that the load reported.

**Fonts (R7 to R9)** in `fanta-text`:

- `register_external_font_dirs`
  (`crates/fanta-text/src/font_resolver/external.rs`) already registers every
  font file in a list of directories. The resolver exposes the same step for
  one directory, and the two places that open a project (the editor's
  document load and `fanta-headless`) call it with `<project>/fonts`. The
  text engine builds its resolver in `crates/fanta-text/src/layout/engine.rs`.
- The export copies `fonts/` and writes one `@font-face` rule per file into
  `tokens.css`.

**FNX loader contract (R12, R13).** `reconcile_fnx_sidecar` (`read.rs`) mints
ids for elements without one when a sidecar exists. The older layout, with
inline ids and no sidecar, gets the same treatment instead of the "missing
identity sidecar" error. A design that does not parse takes spec 0009's
`design.unavailable` path.

**Checker (R15).** `crates/fanta-format/src/project/check.rs` dispatches on
`storage` as the reader does. The FNX branch resolves components and
manifest files from the FNX layout.

**Export (R14).** `write_web_project_tree` stays the one emitter and loses
its in-place conversion duty (it prunes `.fnx` today when pointed at a
project). `fanta-site` keeps writing it to its own output directory.
`script/serve-project`, the browser host whose PUT channel persists edits,
serves the emitted files instead of the project folder's own, and so does the
plugin pane. `fanta-wasm-engine` already emits in memory
(`emit_web_project_tree`), and its `read_project_files` is the same reader as
on disk, so it already accepts FNX.

**Save (R10, R11, R17).** `write_project_tree`
(`crates/fanta-format/src/project/write.rs`) calls the FNX writer again, and
`write_legacy_project_tree` and its `_checked` twin lose "legacy" from their
names. `crates/fanta-headless/src/save.rs` switches with it. The FNX writer's
stale-file pruning removes the web-storage files on the converting save.

## Interfaces and data

- FNX: `asset` takes an id or a project-relative path (the type stays
  `string` in `fnx.d.ts`), and `natural_size` becomes optional.
- `doc/asset_library.json`: an entry may carry `"path"`.
- Project layout: `fonts/` is user-managed and never pruned.
- New diagnostic codes: `asset.unindexed` (warning), `asset.damaged`
  (error), `ref.asset_outside_project` (error), `font.substituted`
  (warning). `ref.missing_asset` and `design.unavailable` exist.
- `fanta.json`: `storage` is no longer written. `"storage": "web"` is still
  read.

## Alternatives

The format choices are ADRs
[0032](../../docs/decisions/0032-return-to-fnx-source-and-export-plain-web.md)
and [0033](../../docs/decisions/0033-link-images-and-fonts-by-project-path.md),
both proposed. Within this design:

- **Keep web storage as a second save target behind a setting.** Two
  storages to keep lossless forever. Rejected.
- **Restore the incremental save that #190 deleted** (1,328 lines in
  `crates/fig_viewer/src/document.rs`). Not needed: the web writer has done a
  full projection on every save since the flip. Revisit with save-time
  profiles of large projects.

## Risks

- **Conversion loses data.** Snapshots already wrap every save (spec 0009),
  so a converting save is one Restore Snapshot away from undone. A7 runs the
  conversion over the corpus.
- **A tool still expects web files in the project.** The export task lands
  before the save flips, and A10 checks that nothing writes into the tree.
- **The web reader's text defect is frozen into FNX.** The 11 bound text
  nodes change on the first load of a web-storage project. T6 fixes the
  reader rule (a text bound to a property keeps its own content) before
  converting.
- **Font tests differ by platform.** The committed golden was rendered on
  macOS. A4 compares the resolved family and file, not pixels.
- **A third change of format.** ADR 0032 closes the question until there is
  evidence from users.

## Verification

- A1 to A3, A8, A9: `fanta-format` tests on fixtures under
  `crates/fanta-format/tests/`; stages `engine`, `source-order` and
  `shell-design-source`.
- A4, A5: `fanta-text` and `fanta-headless` tests; stages `engine` and
  `render`.
- A6, A7, A13: stages `engine`, `import` and `editor`, plus the regressions
  named in `.github/workflows/check.yml`.
- A10: `fanta-site` and `fanta-wasm-engine` tests, stage `lint-wasm` and the
  plugin pane smoke test.
- A11: stage `shell-hostile-edit-corpus`.
- A12: review, and `script/gate`.
