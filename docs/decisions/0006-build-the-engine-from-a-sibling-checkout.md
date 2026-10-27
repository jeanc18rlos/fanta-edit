---
status: superseded
date: 2026-07-10
recorded: 2026-10-27
decision-makers: Jean Rojas
superseded-by: 0008
tags: [engine, repository]
---

# Build the engine from a sibling checkout

## Context

The engine crates were ported into this repository on 2026-07-07. The same
crates were also being developed in a separate engine repository,
`squidred-dev/fantaisa-engine`, checked out at `../fanta-engine-migration`.
Keeping two copies in step meant copying every engine fix by hand.

## Decision

Delete the in-tree copies and depend on the sibling checkout through `path`
dependencies (`../fanta-engine-migration/crates/fanta-doc` and so on), "so
engine fixes land here without re-vendoring". The UI kit followed the same
pattern through `../fanta-gpui`.

## Options considered

1. **Sibling path dependencies** (chosen at the time). Zero ceremony on one
   machine.
2. **Git dependencies pinned by revision.** Not considered in the commit.

## Consequences

- Good: engine and editor changes could be made together on the founder's
  machine.
- Bad: the build depended on folders that existed only on that machine, so CI
  and anyone cloning the public repository could not build it. The public GPL
  repository no longer carried the source of the binary it described. The UI
  checkout pointed back at this repository, a circular path reference.
- Superseded on 2026-09-08 ([0008](0008-vendor-the-engine-into-this-repository.md)).
  The engine repository's working branch kept moving until 2026-08-16 and was
  never pushed (see 0008).

## Confirmation

None; see 0008.

## References

- Commit `e929627ef1` "Switch fanta engine crates to fanta-engine-migration path dependencies" (2026-07-10)
