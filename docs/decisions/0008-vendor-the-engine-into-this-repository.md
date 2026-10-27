---
status: accepted
date: 2026-09-08
recorded: 2026-10-27
decision-makers: Jean Rojas
supersedes: [0006]
tags: [engine, repository, licensing]
---

# Vendor the engine into this repository

## Context

Building against sibling checkouts
([0006](0006-build-the-engine-from-a-sibling-checkout.md)) worked on one
machine only. CI and anyone cloning the public repository could not build it,
the public GPL repository did not carry the source of the binary, and the UI
checkout referred back to this repository.

## Decision

Copy the nine engine crates from `squidred-dev/fantaisa-engine` at `9370fa2`
and `fanta-gpui` from `fanta-ui` at `a5edcd6` into `crates/`, repoint the ten
workspace dependencies at them, and record the provenance in
[`crates/VENDORED.md`](../../crates/VENDORED.md). Package versions in
`Cargo.lock` stay identical. From here on the engine is developed in this
repository.

## Options considered

1. **Vendor the crates** (chosen).
2. **Depend on the engine repository by git revision.** The commit does not
   discuss it. Three weeks later the UI kit moved to that pattern
   ([0011](0011-consume-gpui-and-shared-ui-from-fanta-ui.md)).

## Consequences

- Good: the editor builds from one clone, one pull request can change the
  engine and the editor together, and the public repository is complete.
- Bad: the engine can only be reached by cloning the editor. Fanta-JS pins a
  fanta-edit commit (`fanta-edit.rev`) and checks out the whole editor to
  build the WebAssembly runtime. Engine crates inherit the editor's workspace
  settings and licence fields. The engine repository's working branch
  stopped at the vendored commit, and its importers for `.psd` and Illustrator
  files, its harness and its facade crate were left behind. That branch,
  `feature/artifact-ir-workspace-session` with 109 commits from July and
  August, was never pushed: on 2026-10-27 it existed only in the founder's
  local checkout, and `9370fa2` is not on GitHub.
- Follow-up: [0024](0024-move-the-engine-into-its-own-repository.md) proposes
  moving the engine out again, without the problems of 0006.

## Confirmation

`crates/VENDORED.md` lists what was copied and every later divergence.
`script/check-licenses` requires each crate's licence file.

## References

- Commit `5c15d4bdb0` "Vendor the fanta engine and UI crates into the tree" (2026-09-08)
