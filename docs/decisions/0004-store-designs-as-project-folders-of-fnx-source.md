---
status: accepted
date: 2026-07-07
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [format, product]
---

# Store designs as project folders of FNX source

## Context

Fanta's thesis is that a design is source code: every canvas edit should be a
change a person or an agent can read and review with Git. A `.fig` file is an
opaque binary, and a single JSON document would produce diffs nobody can
review.

## Decision

A design lives in a project folder with a `fanta.json` manifest. Each page and
each component master is one `.fnx` file, a restricted JSX-like text format
whose elements are scene nodes and whose attributes are node fields. Metadata,
variables and motion are small JSON files. Assets are binary files under
`assets/`. Opening a `.fig` file creates a project folder beside it and leaves
the `.fig` untouched; editing happens in the folder.

The layout grew in steps that kept this decision:

- 2026-07-07: the first save creates the project folder, replacing a "Create
  Fanta Project" button.
- 2026-07-18: opening a `.fig` creates the folder in the background, and page
  and component directories use readable slugs while identity stays in their
  JSON files.
- 2026-10-19: schema 4 adds persisted FSS styles (#132).

## Options considered

1. **A folder of FNX and JSON** (chosen). Each design is reviewable text, and
   one page per file keeps diffs local.
2. **One JSON document.** Simple to write; unreadable to review.
3. **Edit `.fig` files in place.** An undocumented binary format that Fanta
   does not own.

## Consequences

- Good: Git, code review and text-editing agents work on designs directly. A
  pull request shows a canvas change as a few changed lines.
- Bad: the format is a public contract with migrations, slugs, identity rules
  and an id sidecar to maintain. Keeping hand edits and canvas saves
  consistent became its own decision
  ([0009](0009-make-project-files-authoritative.md)).

## Confirmation

`fanta-format` round-trip and migration tests, the `source-order` release
stage, and the schema documented in [project-schema.md](../fanta/project-schema.md).

## References

- [README](../../README.md), [project-schema.md](../fanta/project-schema.md)
- Founder decisions recorded in session notes on 2026-07-07 and 2026-07-18
- #132 (schema 4)
