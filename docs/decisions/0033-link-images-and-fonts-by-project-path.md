---
status: proposed
date: 2026-11-24
decision-makers: Jean Rojas
tags: [format, assets, fonts]
---

# Link images and fonts by project path

## Context

A person or an agent cannot put `photo.png` or a font file into a project and
use it by writing its path. Measured on 2026-11-24 with the project reader:

- `<Image asset="assets/photo.png" … />` in FNX makes the whole project fail
  to load (`invalid length`), because `asset` accepts only an asset id.
- The documented way needs the file renamed to its content hash under
  `assets/images/`, an entry with its size and SHA-256 in
  `assets/index.json`, and a hand-typed `natural_size`. Leave out the index
  entry and the project fails to load ("asset … is missing from
  assets/index.json").
- No code loads fonts from a project. The resolver knows system fonts, four
  bundled families, Google Fonts downloads and the directories named by the
  `FANTA_FONT_DIRS` environment variable
  (`crates/fanta-text/src/font_resolver/`).

The content-addressed store is right for what the editor and the importers
add: it deduplicates, detects damage and keeps ids stable across `.fig`
imports. It is wrong as the only door for a file someone adds by hand. Other
tools that keep designs in a repository reference media by relative path.

## Decision

Let source files reference media by project-relative path, and load fonts
from a `fonts/` folder in the project.

- An `asset` value that is not an asset id is a path relative to the project
  root. The loader reads the file, derives the asset id from its content and
  remembers the path. The writer writes the path back and leaves the file
  where it is. The managed store under `assets/` and its index keep serving
  everything the editor and the importers add.
- Every `.ttf`, `.otf`, `.ttc` and `.otc` file under `fonts/` is registered
  when the project opens, in the editor, the headless engine, the cloud
  worker and the web export, ahead of system fonts of the same family.
- A reference that cannot be resolved (a missing file, a path outside the
  project, an unreadable image, a managed file the index does not list) is a
  diagnostic on the node and a placeholder on the canvas. It never fails the
  project load, and a save never rewrites or deletes what the load could not
  read.

## Options considered

1. **Path links kept as written** (chosen). The source says what the person
   typed, and the file stays theirs.
2. **Adopt on load.** Copy the file into the managed store and rewrite the
   reference to its id on the next save. No model change, but the link turns
   into a hash, the file is duplicated and later edits to it are ignored.
3. **Ids only, with better importers** (`import_image`, Place image). Does
   not help someone editing files without the app, or an agent without the
   live MCP server.
4. **Fonts in the managed store** (`assets/fonts/<id>.ttf`; the family
   already exists in `media.rs`). Hash names again, while a font file already
   carries its family name.

## Consequences

- Good: add a file, write its path, done. The cloud worker renders with the
  project's own fonts, so output stops depending on the machine.
- Bad: there are two ways to reference media. A linked file can change or
  vanish behind the document's back, so its asset id can differ between
  loads and a missing file is a normal state the canvas must show.
- Bad: paths are a trust boundary. A project from someone else must not make
  Fanta read or export a file outside its folder. Absolute paths, `..`
  segments, URLs and symlinks that leave the project are refused.
- Bad: a project that uses path links does not open in a build older than
  this change (it fails with `invalid length`).
- Open: font registration is per process today, so two open projects that
  ship different files for one family need a rule.
  [Spec 0015](../../specs/0015-return-to-fnx-source-with-linked-images-and/spec.md)
  lists it as a question.

## Confirmation

Spec 0015's tests: a project with `<Image asset="images/photo.png" … />` and
a font in `fonts/` loads, renders and saves with the path intact; a missing
file gives `ref.missing_asset` and unchanged source after a save; a path
outside the project is refused with a diagnostic and never read.

## References

- [Spec 0015](../../specs/0015-return-to-fnx-source-with-linked-images-and/spec.md)
- [0032](0032-return-to-fnx-source-and-export-plain-web.md),
  [0009](0009-make-project-files-authoritative.md)
- `crates/fanta-format/src/project/media.rs` (asset index),
  `crates/fanta-text/src/font_resolver/external.rs` (font directories)
- [Pencil's file format](https://docs.pencil.dev/for-developers/the-pen-format),
  which references images by relative path. Read by a research agent on
  2026-11-23.
- Founder report recorded in session notes, 2026-11-24
