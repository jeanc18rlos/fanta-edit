---
status: accepted
date: 2026-09-27
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [ui, repository, dependencies]
---

# Consume GPUI and the shared UI from fanta-ui

## Context

After vendoring ([0008](0008-vendor-the-engine-into-this-repository.md)),
GPUI, Zed's UI baseline crates and Fanta's UI kit all lived in this
repository. Fanta wanted a reusable design system with its own repository and
CI. #9 resynchronized the vendored kit with `fanta-ui` first.

## Decision

Publish GPUI and the reusable UI crates from `squidred-dev/fanta-ui` as the
`fanta-gpui-*` packages (0.5.0 on crates.io since 2026-09-21). The workspace
depends on exact `=0.5.0` versions and patches all 41 packages to a single
`fanta-ui` git revision in `[patch.crates-io]`, so the editor can use
`fanta-ui` commits before they are released. Remove the in-tree copies.

## Options considered

1. **Registry versions patched to one git revision** (chosen).
2. **Registry versions only.** Every UI change the editor needs would wait for
   a release.
3. **Keep vendoring.** No separate design-system repository.

## Consequences

- Good: UI work has its own repository and CI, and the editor builds from a
  clean clone.
- Bad: a UI change the editor needs takes a pair of pull requests and a pin
  bump across all 41 entries. A deleted branch can leave a pinned commit
  unreachable, so pins point at merge commits. When both repositories move, a
  pin conflict is resolved with a `fanta-ui` commit that contains both sides,
  never by picking one revision. Neither repository can merge on green by
  itself today.

## Confirmation

`Cargo.lock` carries one `fanta-ui` source revision for every `fanta-gpui-*`
package. The layer map treats `fanta-ui` as an external family that the
engine group must never reach.

## References

- #9 (2026-09-21), #15 (2026-09-27), `squidred-dev/fanta-ui#1`
- [crates/VENDORED.md](../../crates/VENDORED.md)
