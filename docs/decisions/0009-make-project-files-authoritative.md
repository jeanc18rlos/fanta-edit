---
status: accepted
date: 2026-09-23
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [format, editor]
---

# Make project files authoritative

## Context

The code-first audit ([code-first-audit.md](../fanta/code-first-audit.md))
reproduced data-loss bugs that all came from treating the in-memory document
as the truth: an unchanged save rewrote `fanta.json`; regenerating from a
stale document replaced a hand-edited `page.fnx` and deleted a hand-written
`notes.md`; a same-length asset replacement went unnoticed; external edits
during a save could be dropped and later overwritten.

## Decision

Project files are the source of truth, and the document is a cache for
editing and rendering. Keep one source file per page and per component, with
a restricted, static FNX grammar of literal attributes. Preserve comments,
whitespace and spelling, and make canvas edits patch only the affected source
spans. Give nodes explicit, stable ids. Store assets by SHA-256 digest and
verify them. A save without changes writes nothing. Prune only files the
manifest owns. Invalid source keeps the last good canvas and shows a
diagnostic.

## Options considered

1. **Files authoritative, document as cache** (chosen).
2. **Document authoritative, files regenerated.** Reuses the existing writer,
   but a late save can overwrite code edits and reprinting loses source text.
3. **Generated and hand-written regions.** Another schema to maintain, and
   edits in hand-written regions cannot reach the canvas.

## Consequences

- Good: agents, developers and the canvas share one reviewable state. External
  editors, agent commits and branch switches enter through the same
  reconciliation path.
- Bad: a large rework of saving and loading, which landed over October
  (#30, #53, #56, #87, #110, #111, #123), and a more complex save path to keep
  correct.

## Confirmation

The `source-order` release stage, the `fanta-format` round-trip tests and the
save and recovery regressions that `check.yml` requires by name.

## References

- [code-first-audit.md](../fanta/code-first-audit.md), approved 2026-09-23
- [project-schema.md](../fanta/project-schema.md)
