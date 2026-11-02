---
status: implemented
date: 2026-10-27
owner: Jean Rojas
decisions: [0001, 0023]
---

# Architecture map and boundaries

## Problem

Nothing in the repository says which crate may depend on which. The two
boundary rules that existed (no GPUI in `fanta-headless`, no Skia in the
model) were one-off `cargo tree` commands that CI never ran. There were no
architecture diagrams, so understanding the system meant reading code, and
there was nowhere to record why it is built the way it is. The engine split
(spec 0002) needs a boundary that is checked, not hoped for.

This spec was written together with its implementation, before the process
it introduces existed. Accepting the pull request accepts both.

## Goals

- Every workspace crate has a declared layer, in one file.
- The diagrams of the crates and layers regenerate from the code.
- A dependency that breaks the architecture fails CI.
- Decision records and specs are validated and indexed automatically.

## Non-goals

- Moving code between crates. That follows from ADR 0023, change by change.
- Diagrams of modules inside a crate.
- Checking that pull request descriptions link a spec. Review does that.
- Generating the context and container diagrams, which are drawn by hand in
  the architecture guide.

## Requirements

- **R1.** Every workspace crate must belong to exactly one layer, declared in
  `docs/architecture/layers.toml`.
- **R2.** A normal or build dependency that the layer rules do not allow must
  fail CI, naming the crate, the dependency and the way to fix it.
- **R3.** Forbidden package families (Skia in the core layers, `fanta-ui` in
  the engine) must be found through other workspace crates, with the path.
- **R4.** Dev-dependencies of engine crates must stay inside the engine.
- **R5.** Exceptions must be explicit seams with reasons, and a seam nothing
  uses must fail.
- **R6.** The crate map (layer diagram, crate diagram and tables) must be
  generated, identical on every run, and checked for staleness in CI.
- **R7.** Decision records and specs must be validated (status, dates,
  supersession on both sides, references, open tasks) and their indexes
  generated.
- **R8.** New records must be created from templates with the next number.
- **R9.** The checks must need no build, so CI finishes them in minutes on a
  Linux runner.

## Acceptance criteria

- **A1 (R1, R2).** `script/architecture check` passes on this branch, and a
  unit test shows an inner layer depending on an outer one failing.
- **A2 (R3).** A unit test shows a core crate reaching `skia-safe` through
  another crate, reported with the path.
- **A3 (R4).** Unit tests show an engine dev-dependency on an outer crate
  failing and one inside the engine passing.
- **A4 (R5).** A unit test shows an unused seam reported.
- **A5 (R6).** `script/architecture map --check` fails after `layers.toml`
  changes until the map is regenerated, and two renders are identical.
- **A6 (R7).** Unit tests cover supersession, open tasks and unknown
  references.
- **A7 (R8).** A unit test covers numbering and the generated slug.
- **A8 (R9).** The Architecture workflow finishes in under two minutes on
  `ubuntu-latest`.

## Open questions

- None.

## Changes after acceptance

- None.
