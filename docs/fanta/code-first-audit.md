# Code-first project audit and source-of-truth proposal

Status: source-of-truth proposal approved on 2026-09-23. This report records the pre-implementation audit; see [project-schema.md](project-schema.md) for the resulting file conventions.

## Current pipeline

| Step | Owner | Current behavior |
| --- | --- | --- |
| Canvas edit | `fig_viewer/src/document.rs:1680`, `fanta-doc/src/doc.rs`, `fanta-doc/src/op/apply.rs` | An operation mutates the live `Doc` and its scene graph. |
| Save scheduling | `fig_viewer/src/view.rs:3505`, `fig_viewer/src/document.rs:1896` | Autosave captures a whole-document snapshot; a background task writes it. |
| Serialize | `fanta-format/src/project/write.rs:323`, `fanta-fnx/src/print.rs:43` | Nodes are bucketed by page/component, printed as `.fnx`, with identity/order in `.ids.json`; singleton state is JSON. |
| Write files | `fig_viewer/src/document.rs:2375`, `fanta-format/src/project/write.rs:124` | The app scaffolds first, then the writer compares projected bytes, replaces individual files atomically, and prunes unprojected files. |
| Watch | `fig_viewer/src/document.rs:1235` | Zed worktree events are filtered, suppressed during and briefly after saves, then debounced for 300 ms. |
| Parse | `fanta-format/src/project/read.rs:42`, `fanta-fnx/src/parse.rs:23` | The entire tree is read, `.fnx` is parsed with its sidecar, and a new `Doc` is assembled. |
| Reconcile | `fig_viewer/src/document.rs:1327`, `fig_viewer/src/document.rs:1496` | Clean documents are replaced in full; dirty documents use a whole-document three-way merge. |

`fanta-format/src/project/session/` contains a finer artifact-scoped session and `fanta-fnx/src/source.rs` contains a source-preserving mirror. The live viewer does not use either; their production call sites are within the format crate.

## Reproduced failures and root causes

The format crate's direct no-op test passed:

```text
cargo test -p fanta-format --locked --offline project::tests::resave_without_changes_writes_and_removes_nothing -- --exact --nocapture
1 passed
```

A temporary Rust harness, using the current compiled `fanta-format` and `fanta-doc` libraries without repository edits, reproduced the following:

| Failure | Root cause and location |
| --- | --- |
| An unchanged **app save** rewrites `fanta.json` and changes its mtime, even though the final bytes match the original. A crash between the two writes can leave a different project ID. | `fig_viewer/src/document.rs:2381` calls `scaffold_project_tree` on every save. `fanta-format/src/project/layout.rs:510-525` directly writes a new manifest using a fresh ID and time, before `project/write.rs` restores the document's manifest. The direct writer's no-op test does not exercise this app path. |
| Regenerating a stale `Doc` replaced a hand-edited `pages/home/page.fnx`. | `fanta-format/src/project/write.rs:124-139,174-180,835-849` treats the in-memory document as authoritative. `fanta-format/tests/write_cache.rs:118-133` explicitly asserts this replacement. |
| Regeneration deleted a hand-written `pages/home/notes.md`. | `fanta-format/src/project/write.rs:192-200,1090-1167` prunes every file under a managed directory that is absent from the projection. Ownership boundaries are not recorded in the manifest. |
| Replacing an asset with different bytes of the same length was not repaired; a subsequent read returned the changed bytes. | `fanta-format/src/project/write.rs:852-883` assumes matching length means matching content. `fanta-format/src/project/read.rs:668-694` reads bytes without checking their identity. |

Code-path failures established by reading the live event and save paths:

| Failure or gap | Root cause and location |
| --- | --- |
| An external edit during a save or its one-second cooldown can be missed, then overwritten by a later canvas save. | `fig_viewer/src/document.rs:1280-1285` drops watcher events without a later hash rescan. `:1918-1995` can then save a stale document through the authoritative writer. This race needs a live event-order regression test. |
| A valid external edit reloads the whole project, even when one page changed. | `fig_viewer/src/document.rs:1496-1553,2628-2635` calls `read_project_tree` and replaces `FigDocument`; dirty state uses whole-Doc `merge_docs` at `:1327-1417`. |
| Source comments, formatting, and wrapper text disappear after a later canvas save. | `fanta-format/src/project/read.rs:505-534` keeps semantic nodes, while `project/write.rs:699-752` generates new source. The source-preserving `FnxSourceMirror` is confined to the unused artifact session. |
| A hand-inserted element can get a different ID in two checkouts of the same files. | `fanta-format/src/project/read.rs:615-640` includes the canonical absolute source path in its new-ID seed. |
| A design can fall back from one `.fnx` file to per-node JSON files without a visible save failure. | `fanta-format/src/project/write.rs:723-752` catches an FNX encode error, logs it, and projects a `nodes/` file pile. This violates the intended one-source-file-per-design review shape. |
| A new page or component in a branch switch is not discoverable through the artifact session as written. Removed files can survive its cross-tree copy. | `fanta-format/src/project/session/workspace.rs:510-559` routes only indexed artifacts. `session/sync.rs:31-64,94-102` iterates existing IDs and copies present files, without inventorying additions or deleting absent files. This session is not yet wired to the viewer. |
| A hand-added FNX element fails in the artifact session unless its sidecar is also changed. | `fanta-format/src/project/session/artifact.rs:1740-1751` decodes against the unchanged sidecar; the whole-tree reader instead reconciles it at `project/read.rs:521`. |
| New Design can leave a tagged but incomplete project after a failed write, and retry refuses the nonempty folder. | `fig_viewer/src/new_design.rs:105-139` writes directly to the destination. `project/layout.rs:510-525` creates `fanta.json` before design files; `:613-624` detects a project from the format tag alone. Save As has staging at `fig_viewer/src/document.rs:2308-2329`, but creation does not. |
| Autosave failures are only logged, and another edit is needed to rearm the consumed timer. | `fig_viewer/src/view.rs:3542-3559` detaches the save and logs its error. The user has no visible save failure or retry state. |

Workspace checks found shared live items for multiple views (`fig_viewer/src/document.rs:234`) and tab deduplication on normal worktree opens (`fig_viewer/src/workspace_hooks.rs:36-151`); no normal switching failure was established. The following lifecycle gaps are code-inferred: a project created inside an already-open folder does not trigger the hook, which listens only for `WorktreeAdded` (`workspace_hooks.rs:43`); moved/missing local folders are omitted from recents and session restore without a project-ID relocation path (`workspace/src/persistence.rs:2001,2044,2204`; the missing-path test at `:3996` asserts omission).

## File-type support

| Type | Import and display/edit path | Versioning and remaining gap |
| --- | --- | --- |
| Bitmap images | PNG, JPEG, WebP, GIF, BMP, TIFF paste creates bitmap layers (`fig_viewer/src/view.rs:5648`). | Stored under `assets/` and referenced by ID. Decode runs in the foreground document update (`document.rs:869-900`), which can stall the UI for a large input. |
| Fonts | Text styles keep family names and resolve installed/bundled/downloaded faces (`fig_viewer/src/view.rs:2055`). | Font blobs can be stored under `assets/fonts`, but project bytes are not registered with the renderer's font resolver. A project-contained font is not portable yet. |
| SVG | Export exists; the AI media path can convert a restricted SVG subset into editable vectors (`generation_workspace.rs:2141`, `generation_media.rs:64`). | Ordinary SVG file paste is absent; text, embedded images, masks, and filters fail editable conversion. Raw SVG can be versioned as an asset but is not an editable import path. |
| Video | Generated MP4 results make video nodes with inline macOS playback and trim controls (`generation_media.rs:719`, `fig_viewer/src/view.rs:4527`). | Ordinary video paste is absent (`view.rs:5648`). Project assets retain the bytes; playback has format/platform constraints. |
| Design tokens | Variables UI edits `doc/variables.json`; external changes use whole-project reload. | JSON is versioned, but the in-app JSON code pane is read-only (`fig_viewer/src/code_workspace.rs:387`). |
| Code | Page and component `.fnx` files are viewable, and their files are versioned. | The in-app Code pane is read-only (`fig_viewer/src/code_workspace.rs:337`); source authoring currently uses an external editor. |

The live project asset store is **not content-addressed**: image and video ingestion mint fresh IDs (`fig_viewer/src/document.rs:879`, `generation_media.rs:748`), and the writer names files by those IDs (`fanta-format/src/project/write.rs:817-830`). The separate `.fant` archive path does derive IDs from bytes (`fanta-format/src/container.rs:430`), but that is not the live project path. No latent-tensor asset convention is present.

There is no importer/exporter registry for these formats. The paste list is in `fig_viewer/src/view.rs:5648`, media detection is a central match in `fanta-format/src/project/media.rs:18`, and editable SVG conversion is a separate AI media path. Adding a type currently changes several core call sites.

## Source-of-truth decision

| Option | Benefit | Cost against this goal |
| --- | --- | --- |
| Files are authoritative; `Doc` is a materialized editing/render cache | One reviewable state for agents, developers, and canvas; external edits and branch switches enter the same path; source trivia can survive focused canvas patches. | Requires the artifact session to become the app path, explicit identity, a concrete-source model, and recovery/merge rules. Invalid source must leave the last-good canvas visible with an error. |
| `Doc` is authoritative; files are regenerated | Reuses most of the live viewer and deterministic writer. | A late save can overwrite code edits; a semantic reprint loses source text. Preserving hand edits requires a second shadow source model and careful merge on every save. |
| Generated and hand-written regions | Generated sections can stay canonical while separate files carry arbitrary developer code. | Boundaries are another schema to maintain; edits in the hand-written area cannot affect the canvas unless parsed. It does not by itself resolve the current watcher/save race. |

**Recommendation: make project files authoritative.** Keep the existing one-source-file-per-page/component `.fnx` layout, plus small JSON files for metadata and tokens. Version a deliberately restricted, static JSX-like FNX grammar with literal attributes; arbitrary React execution would make deterministic, safe round-trips unattainable. The accepted grammar and every representable property must round-trip exactly. Retain comments, whitespace, and user spelling in a concrete-source mirror, and make canvas operations patch only affected node spans. Unsupported syntax must produce a visible diagnostic and preserve the last-good canvas and original file.

Stable node identity should be explicit in the source, for example `id="n_..."` on each element, with the existing sidecar migrated to an ID-keyed order/index record or removed where source order suffices. This adds one attribute per node but avoids checkout-dependent identity and ambiguous matching after hand insertion. Existing projects need a one-time, versioned migration; ordinary edits afterward should change only the touched element line. Keep page/component files separate and sort all generated metadata and attributes with a canonical formatter. A no-op save must do no writes.

Keep binary data outside FNX and JSON. Store assets by full SHA-256 digest, verify bytes on read/write, deduplicate identical imports, and put only readable `asset="sha256:..."` references in source. Store latent tensors as versioned binary blobs with the same digest reference and media metadata; track referenced blobs in Git (or Git LFS for large blobs) and ignore derived previews. A registry of format modules should declare import, display, edit, and export capabilities, so a new file type does not require changes to core watcher/document code.

## Proposed implementation gates after approval

1. Make one project session the owner of manifest, artifact inventory, content hashes, dirty state, and file events. Adapt `WorkspaceSession`/`FnxSourceMirror` for the live viewer; repair its new/deleted-artifact and sidecar paths first. Keep operations as transient edits, committed through source patches with an on-disk hash check and three-way merge on mismatch.
2. Replace time-based event dropping with debounced, hash-gated reconciliation. Own-write events may be ignored only when their exact resulting hash is already indexed; queue or rescan every other change. Reconcile affected artifacts and update scene/render generations without replacing the whole document.
3. Stage and atomically replace changed files, record an ignored transaction journal for multi-file saves, and replay or roll back incomplete transactions on open. Scaffold only on creation. Prune only manifest-owned files; preserve unrelated files. Surface save/recovery errors and retry state in the UI.
4. Route external editors, agent commits, and branch switches through the same inventory/hash/reconcile operation. Document FNX/JSON schemas, ID rules, asset references, migration rules, and conflict behavior in the project `AGENTS.md`.
5. Gate with document → files → document equivalence, byte-identical repeated regeneration with zero writes and an empty Git diff, source-trivia preservation, external-edit races, created/deleted/renamed artifacts, branch switches, asset corruption, and interrupted-write recovery. Record unsupported formats and import limitations explicitly.

The files-authoritative proposal above was approved before implementation began.
