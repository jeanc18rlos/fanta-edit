# Fanta project files

A Fanta project is a directory with a `fanta.json` manifest. Its files are the
persisted design, stored as plain web files: SVG, CSS, HTML, JS and JSON
(spec 0007, ADR 0027). Opening `index.html` in a browser views the project
with no build step. The canvas holds a parsed editing and rendering cache.
Saving a canvas change rewrites the affected design's projection and commits
changed files; an external editor or branch switch enters through the same
file reconciliation path.

## Layout and ownership

| Path | Meaning | Edit by hand? |
| --- | --- | --- |
| `fanta.json` | Project ID, schema/layout versions, `"storage": "web"` | No |
| `index.html`, `viewer.js`, `canvas.js`, `editor.js`, `pages.js` | Dependency-free project viewer, the browser editor it upgrades into under `script/serve-project`, and the page manifest | No — regenerated each save |
| `design.css` | Imports each design's stylesheet; the home for project-wide rules | Yes — add theme-wide rules; keep the generated `@import` lines |
| `tokens.css` | Variable collections and modes as CSS custom properties | No — generated from `doc/variables.json` |
| `report.json` | The last save's findings and anything a load could not keep, as [diagnostic records](#diagnostic-codes) | No — the loader's report channel |
| `pages/<slug>/page.json` | Page ID, name, and order | Change name/order with care; keep the ID |
| `pages/<slug>/page.svg` | The page's tree: element order, ids, `data-name`, geometry, path data, text | Yes |
| `pages/<slug>/page.css` | The page's styling: one rule per node, selected by node id; `var(--token, fallback)` where a property is bound | Yes — declaration values, `var()` swaps, `--fanta-*` layout intent |
| `pages/<slug>/nodes.json` | The complete node records, keyed by bare node id | Yes, as valid JSON; keep the id keys |
| `pages/<slug>/map.json` | Node id → name and world bounds, for outside tools | No — derived |
| `components/<slug>/def.json` | Component identity and definition | Keep IDs and root consistent |
| `components/<slug>/master.svg` + `master.css` + `nodes.json` | A component master's tree, styling and records | Yes, like a page |
| `components/<set>/set.json` | A variant set's axes, values and default | No — use the editor's variant ops |
| `doc/variables.json`, `doc/active_modes.json` | Tokens (`color`, `float`, `string`, `boolean`, `typography`, `enum` with its `options`, `image` asset ids) and current modes | Yes, as valid JSON |
| `doc/metadata.json`, `doc/motion.json`, `doc/flow_start.json` | Project metadata, animation, prototype start | Yes, as valid JSON |
| `assets/<family>/<AssetId>.<ext>` | Binary source assets | Add through an importer when possible |
| `assets/index.json` | Full SHA-256 digest and size for each asset | Update together with any binary file |
| `previews/`, `exports/` | Derived output | No input to the design |

Page and component slugs are readable directory names, not identity. The IDs
in their headers identify designs through a rename or branch switch. Files in
managed directories whose names are not part of the generated schema belong
to the user and survive regeneration. Removing a page or component removes
its generated files; an unrelated note alongside them remains.

## The web projection and the hand-edit contract

The SVG is the authoritative scene tree (element order, `id` in the display
`n_…` form, `data-name`, geometry, path data, text content) and the design's
own stylesheet (`page.css` or `master.css` beside the SVG) its authoritative
styling; `nodes.json` carries each node's complete record, so properties
without an SVG/CSS spelling are never lost. The root `design.css` imports
every design stylesheet and is where project-wide rules live; where a root
rule and a design's rule set the same property for a node, the design's
declaration wins. Each SVG references `tokens.css` and its own stylesheet
with `<?xml-stylesheet?>`, so opening it standalone renders styled, and its
root element carries `data-fanta="page"` or `data-fanta="master"` so a tool
handed the bare file knows it is Fanta source and which kind. On load the
records are read first, then parsed SVG and CSS fields win over them. The
writer is canonical and deterministic: load-then-save normalizes hand-edited
formatting like a formatter — values survive, whitespace does not.

Honored hand edits (R7): geometry attributes, element order, `data-name`,
text content, CSS declaration values, swapping a literal for
`var(--token, literal)`, and auto-layout intent (`--fanta-layout`,
`--fanta-gap`, `--fanta-padding` on the frame's rule — editing intent
re-solves the children on load, so solved child coordinates in the SVG are
derived, not source). Derived output is ignored on load rather than applied:
`-grad`/`-mask` defs and the `url(#…)` paints referencing them, stacked-fill
clone geometry (`<id>-f1`, …; only the clone's `fill` is read), instance
content marked `data-virtual`, and boolean outlines (operands live in
`nodes.json`). Anything a load cannot keep is reported per node in
`report.json` on the next save; nothing is silently dropped. An SVG that
fails to parse degrades to the records with a report, and the next save
rewrites it.

### Hand-editing support matrix

| Tool | Round-trip safe? |
| --- | --- |
| Text editors, diff tools, agents editing values in place | Yes — the contract above |
| JSON edits to `nodes.json`, `doc/*.json` | Yes, as valid JSON with ids preserved |
| Browsers (open `index.html`) | View only; text wrapping is approximated — the engine shapes text |
| CSS formatters on `design.css`, `page.css`, `master.css` | Values survive; formatting is canonicalized on the next save |
| Illustrator, Inkscape and other SVG apps that re-serialize whole files | No — structure outside the emitted dialect is reported and regenerated, not guessed at |
| Anything editing `tokens.css`, `map.json`, `pages.js`, `report.json`, the viewer files | No — regenerated every save |

## Diagnostic codes

A finding from reading a project is a record: `severity`, `code`, `message`,
and where known `file` (project-relative), `line`, `column` and `node_id`.
`report.json` lists them under `warnings`, and `fanta-check <project> [--json]`
reports them for a whole project without writing anything: exit status 0
with no errors, 1 with errors, 2 when the folder is not a project. The cases
under `crates/fanta-format/tests/corpus` show what each kind of outside edit
produces. A code is permanent: new codes are
added, and an existing code never changes meaning.

| Code | Meaning |
| --- | --- |
| `edit.not_kept` | A hand edit to a node's SVG or CSS could not be applied, or sits where the loader does not read (an inline `style`); the next save regenerates it |
| `edit.selector_outside_dialect` | A CSS rule's selector is not a single node id; the rule is regenerated on save |
| `id.duplicate` | The same `id` is on two elements of one design SVG |
| `id.not_a_node` | An `id` in the SVG or CSS is not a node id |
| `load.failed` | The project does not load, for a reason no other finding explains |
| `load.omitted_geometry` | Auto-layout geometry left out of the source could not be solved on load |
| `manifest.file_missing` | A design's `nodes.json` is gone (error), or an SVG or stylesheet the writer would emit is gone (warning) |
| `parse.conflict_marker` | Unresolved merge-conflict markers in a project file |
| `parse.css` | A stylesheet has unbalanced braces |
| `parse.json` | A project JSON file is not valid JSON |
| `parse.svg` | A design SVG is not well-formed; edits after that point are ignored and a save regenerates the file |
| `record.missing` | An element or rule names a node that has no record in `nodes.json` |
| `record.no_element` | A node the writer draws has no element in its design's SVG; removing an element does not delete the node |
| `record.unreadable` | A `nodes.json` record does not decode as a node |
| `ref.ambiguous_token` | Two variables produce the same CSS token; `var()` edits resolve to the first |
| `ref.missing_asset` | The asset index lists a file that is gone, or a node paints an image the project does not hold |
| `ref.missing_component` | An instance of a component the project does not define |
| `ref.missing_token` | A `var()` names a token no variable produces; the fallback is kept unbound |
| `source.unknown_attribute` | An attribute on a legacy FNX node is not in the vocabulary; it is preserved and ignored by the renderer |

## Asset identity

New asset IDs are the first 128 bits of SHA-256 of the original bytes. The
index stores the full 256-bit digest and byte length, keyed by ID:

```json
{
  "version": 1,
  "assets": {
    "<asset-id>": { "size": 123, "sha256": "<64 lowercase hex digits>" }
  }
}
```

Identical bytes deduplicate. Fanta verifies the full digest on read, including
when a damaged file has the original length. Older projects without an index
remain readable; their existing IDs are preserved and indexed on the next
save. Media type detection uses a bounded signature probe to choose a readable
family and extension. A latent tensor can use the same store with a versioned
format handler; its source reference remains a small asset ID.

## Editing and recovery

Keep `fanta.json`, a design's SVG, its `nodes.json`, its stylesheet and any
referenced assets in the same commit. Save valid files before
switching branches. The running editor debounces file events, checks content
hashes, and reconciles changed artifacts. A concurrent canvas and file edit is
merged when the changes do not overlap; an overlap is shown as a conflict and
is never silently overwritten.

Changed project files are written through temporary files and atomic renames.
A multi-file change uses the ignored `.fanta-transaction/` journal. Opening a
project replays an interrupted transaction only when its expected old/new
hashes still match; otherwise it reports a recovery error instead of choosing
one version. Unchanged generated design files are not rewritten. Save can seed
missing editor support files or upgrade exact legacy seeds; customized support
files remain preserved. Do not edit or commit `.fanta-transaction/`.

## Legacy FNX/FSS projects

Projects written before spec 0007 store each design as `page.fnx` /
`master.fnx` plus `.ids.json` sidecars and `.fss` styling. They open exactly
as before and convert to the web layout on their first save: the writer emits
the web file set and prunes the FNX sources it replaces. There is no FNX/FSS
writer anymore (`write_legacy_project_tree` survives only for conversion test
fixtures). **Deprecation note:** the FNX/FSS readers remain so legacy projects
keep opening; they will be removed by a future decision record once conversion
is no longer needed, after which such projects must be converted by an older
build first. Do not build new tooling against `.fnx`.

## Format modules

The document format registry routes native project folders and `.fant`
snapshots and accepts additional import/export handlers. The binary media
registry contains ordered, bounded byte probes and declares each format's
import, export, display, edit, and version capabilities. An application adds
a new handler at its boundary; project persistence does not need a new
file-extension switch. The editor registers `.fig` import alongside the native
handlers and materializes imported `.fig` and `.fant` files into editable
projects. Imported source files remain untouched.

## Why the writer and watcher work this way

The project files are authoritative. The in-memory document is a fast editing
projection, so a save checks each source file's last observed digest before
committing a canvas patch. That prevents an edit made by an external tool from
being silently replaced by a delayed autosave. Shared JSON and the asset index
are checked separately. The writer compares bytes before scheduling a rename,
and only removes generated files that belong to a deleted or regenerated
artifact.

The watcher queues events even during a save, then hashes the changed path.
An event is an echo only when its bytes exactly match the writer's committed
digest. Clean external changes to a design's web files reload through the full
reader, which applies the hand-edit contract; unsaved canvas edits use a
three-way merge. Invalid source leaves the last valid canvas on screen and
raises a conflict for review.

The short asset ID preserves compatibility with existing `AssetId` references.
The full SHA-256 digest in `assets/index.json` validates contents and catches
an ID collision; the same bytes are stored once. The multi-file journal lets a
restart complete a save that stopped between atomic file renames.

## Current limits

- The Code workspace edits JSON sources in place and shows SVG and CSS; SVG
  buffers have no direct save path yet, so hand edits to them are applied at
  the next load through the contract above. External editors and agents use
  the same reconciliation boundary. See
  [Projects, pages and source](capabilities.md#projects-pages-and-source).
- A save re-emits the full web projection; incremental per-artifact writes
  and cached digests remain performance work for very large projects, and an
  externally changed web design reloads through the full reader rather than
  an incremental splice.
- A Git branch switch that changes the project ID requires opening that
  project as a different project. A moved project can be relinked from Recents
  only when its manifest carries the same stored project ID.
- File-system hash checks are performed immediately before commits, but
  operating systems do not provide a portable compare-and-rename operation.
  Another process writing the exact same path in the interval between the
  check and rename can still race the save. The journal and next watcher
  event recover or report the resulting divergence.
