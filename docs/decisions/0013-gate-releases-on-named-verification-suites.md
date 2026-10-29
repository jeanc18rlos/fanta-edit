---
status: accepted
date: 2026-10-05
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [quality, release]
---

# Gate releases on named verification suites

## Context

Release reports in September mixed automated results, manual passes and
claims nobody had checked. A test could be renamed, skipped or silently pass:
`#[gpui::test]` discards a test's return value, so a test that returns `Err`
passes. "Verified" did not mean anything specific.

## Decision

`script/verify-fanta-release` defines the release suites as named stages
(engine, import, source-order, editor, workspace, ui, build, lint), runs them
reproducibly and writes logs and a `results.tsv`. CI runs the stages, then
checks that every regression on an explicit list of 51 editor and workspace
tests actually ran and passed. Renaming or deleting one of those tests means
updating the list in `check.yml` in the same change. Golden images are
regenerated only when the founder asks.

## Options considered

1. **Named stages plus a required-test list** (chosen).
2. **`cargo test` over the workspace.** Too slow on this workspace, and a
   missing test does not fail it.
3. **Manual release checklists only.** Kept for what automation cannot reach
   ([SMOKE.md](../alpha/SMOKE.md)), not as the gate.

## Consequences

- Good: a release claim names the stages it ran, and a required regression
  cannot quietly disappear.
- Bad: CI runs for a long time on macOS runners that compile Skia, and the
  required list is maintained by hand.

## Confirmation

The "Verify every required editor and workspace regression actually passed"
step in [`check.yml`](../../.github/workflows/check.yml), added on 2026-10-11
(`c46a1b41bc`), and the `fanta-verify` skill that maps changes to stages.

## References

- Commit `9a1bc934f4` "Document editor capabilities and enforce reproducible release verification" (2026-10-05, #55)
- #104 (hosted release lint), [RELEASE_VALIDATION.md](../alpha/RELEASE_VALIDATION.md)
