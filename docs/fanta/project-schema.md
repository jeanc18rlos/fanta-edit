# Fanta project files

A Fanta project is a directory with a `fanta.json` manifest. Its files are the
persisted design. The canvas holds a parsed editing and rendering cache. Saving
a canvas change patches the affected source and commits changed files; an
external editor or branch switch enters through the same file reconciliation
path.

## Layout and ownership

| Path | Meaning | Edit by hand? |
| --- | --- | --- |
| `fanta.json` | Project ID and schema/layout versions | No |
| `pages/<slug>/page.json` | Page ID, name, and order | Change name/order with care; keep the ID |
| `pages/<slug>/page.fnx` | One page's node tree | Yes |
| `pages/<slug>/page.ids.json` | Legacy identity and sibling order record | Keep with its FNX file |
| `components/<slug>/def.json` | Component identity and definition | Keep IDs and root consistent |
| `components/<slug>/master.fnx` | One component master's tree | Yes |
| `components/<slug>/master.ids.json` | Legacy identity and sibling order record | Keep with its FNX file |
| `doc/variables.json`, `doc/active_modes.json` | Tokens and current modes | Yes, as valid JSON |
| `doc/metadata.json`, `doc/motion.json`, `doc/flow_start.json` | Project metadata, animation, prototype start | Yes, as valid JSON |
| `assets/<family>/<AssetId>.<ext>` | Binary source assets | Add through an importer when possible |
| `assets/index.json` | Full SHA-256 digest and size for each asset | Update together with any binary file |
| `previews/`, `exports/` | Derived output | No input to the design |

Page and component slugs are readable directory names, not identity. The IDs
in their headers identify designs through a rename or branch switch. Files in
managed directories whose names are not part of the generated schema belong
to the user and survive regeneration. Removing a page or component removes
its generated files; an unrelated note alongside them remains.

## FNX source

FNX is a restricted, static JSX-like language. An element is a scene node; its
attributes are literal properties, and nested elements are its children. The
`id` attribute is the stable node identity and should stay with the same node
when changing its properties or position. A copied element needs a new ID.
The `.ids.json` sidecar remains for older projects and sibling order. A legacy
element without an explicit ID receives a deterministic ID on import; the next
save writes it into FNX, so two checkouts do not invent different identities.

The printer uses a canonical order for generated attributes and metadata.
Canvas edits keep unaffected source spans, including comments and whitespace.
Unsupported syntax or a parse error leaves the file unchanged and keeps the
last valid canvas visible with a diagnostic. Source references to binary data
use asset IDs; raw binary and latent tensor data never live inside FNX or JSON.

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

Keep `fanta.json`, an FNX file, its sidecar, and any referenced assets in the
same commit. Save valid JSON and FNX before switching branches. The running
editor debounces file events, checks content hashes, and reconciles changed
artifacts. A concurrent canvas and file edit is merged when the changes do not
overlap; an overlap is shown as a conflict and is never silently overwritten.

Changed project files are written through temporary files and atomic renames.
A multi-file change uses the ignored `.fanta-transaction/` journal. Opening a
project replays an interrupted transaction only when its expected old/new
hashes still match; otherwise it reports a recovery error instead of choosing
one version. A no-op save writes nothing, and a changed page does not require
printing unrelated pages. Do not edit or commit `.fanta-transaction/`.

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
being silently replaced by a delayed autosave. Artifact sessions retain the
parsed FNX source and its comments; a common shape edit only projects the
affected page or component. Shared JSON and the asset index are checked
separately. The writer compares bytes before scheduling a rename, and only
removes generated files that belong to a deleted or regenerated artifact.

The watcher queues events even during a save, then hashes the changed path.
An event is an echo only when its bytes exactly match the writer's committed
digest. Clean external changes use the artifact index to parse changed pages
and components; changes to project-wide structures or asset bytes use the full
reader. Unsaved canvas edits use a three-way merge. Invalid source leaves the
last valid canvas on screen and raises a conflict for review.

The short asset ID preserves compatibility with existing `AssetId` references.
The full SHA-256 digest in `assets/index.json` validates contents and catches
an ID collision; the same bytes are stored once. The multi-file journal lets a
restart complete a save that stopped between atomic file renames.

## Current limits

- The Code pane displays FNX and tokens but does not yet edit them in place;
  use an external editor or agent. Both enter through the same watcher path.
- Project-contained font registration, ordinary SVG/video paste, and editable
  SVG features outside the current vector subset still need format handlers.
  Latent-tensor blobs have the content-addressed storage convention but no
  import or rendering handler yet.
- A Git branch switch that changes the project ID requires opening that
  project as a different project. A moved project can be relinked from Recents
  only when its manifest carries the same stored project ID.
- File-system hash checks are performed immediately before commits, but
  operating systems do not provide a portable compare-and-rename operation.
  Another process writing the exact same path in the interval between the
  check and rename can still race the save. The journal and next watcher
  event recover or report the resulting divergence.
- Saves and external-change checks still scan and hash the project inventory
  for integrity, even when only one design needs parsing or rewriting.
  Path-scoped inventory updates and cached binary digests remain performance
  work for very large projects.
