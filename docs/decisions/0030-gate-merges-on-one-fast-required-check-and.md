---
status: accepted
date: 2026-11-12
decision-makers: Jean Rojas
supersedes: [0013]
tags: [quality, release, ci]
---

# Gate merges on one fast required check and run each suite on the cheapest platform that can witness it

## Context

[ADR 0013](0013-gate-releases-on-named-verification-suites.md) made
"verified" mean something: named stages in `script/verify-fanta-release`
and a list of required regressions that must actually run. It accepted one
cost, "CI runs for a long time on macOS runners", and that cost grew until
CI stopped working as a gate.

On 2026-11-12 the last green `Check` run had taken 476 minutes, of which
305 to 405 were its three macOS jobs waiting in the queue, and no later run
had completed. `check.yml` runs on every branch push and on every pull
request, on macOS, although only 15 of the 51 required regressions, the
golden image and packaging need a Mac. No check is required to merge: the
`main` ruleset blocks deletion and force pushes only. That day the founder
had 24 pull requests merged on a local Linux run instead.
[Spec 0012](../../specs/0012-fast-gated-ci-and-the-agent-harness/spec.md)
holds the measurements.

The founder decided the following on 2026-11-12, in the planning session
that produced spec 0012. This record is `proposed` until the founder merges
its pull request; that merge is the acceptance, and the same change then
marks ADR 0013 superseded.

## Decision

Require one check, `gate`, to merge into `main`, with the founder as bypass
actor. `gate` passes when every job the change needed succeeded or was
skipped.

Run each suite on the cheapest platform that can witness what it tests:
checks that need no compiler in a `hygiene` job, platform-independent
suites on Linux, and on macOS one job holding only what macOS alone can
witness. Move what does not need to run per push (the Mac App Store suite,
the all-features lint, Skia source compatibility, fuzzing, the custom
lints) to a nightly tier that must be green on a commit before it is
released. Windows is advisory until its cache is fixed.

Keep ADR 0013's named stages and its rule that a required regression must
actually run and pass. Move the list of required tests out of `check.yml`
into a checked-in data file with a platform per test. Keep its rule on
golden images.

## Options considered

1. **One required aggregating check, suites placed by platform, a nightly
   tier** (chosen). A pull request gets an answer in minutes, the answer
   binds, and macOS minutes go only to what needs a Mac.
2. **Keep ADR 0013's shape and add faster or self-hosted macOS runners.**
   It pays to run on a Mac what Linux can witness, and still leaves merging
   ungated. It remains the escalation if the target of spec 0012 (A1) is
   missed.
3. **Require every job individually.** A path-filtered workflow that does
   not start leaves its required check pending forever, so every pull
   request that skips a job would need a bypass.
4. **GitHub's merge queue.** Not available to this repository until the
   transfer in [spec 0003](../../specs/0003-github-organization/spec.md).
5. **Gate on local runs.** What happened on 2026-11-12. It depends on one
   machine and cannot witness the macOS-only tests.

## Consequences

- Good: a red change cannot merge without the founder choosing to bypass;
  a docs-only pull request is checked in minutes; a renamed required test
  fails before anything compiles.
- Good: `script/gate` gives people and agents the same checks locally that
  CI runs.
- Bad: Mac App Store code, the all-features lint and Skia source
  compatibility can break on `main` and be noticed up to a day later.
- Bad: a Linux pass no longer implies a macOS pass for the suites that
  moved; platform-specific behaviour in them is witnessed on Linux only.
- Bad: one more data file and script to maintain in place of the inline
  list.
- Follow-up: spec 0012's tasks; the founder adds the required check and
  the bypass to the ruleset; `CLAUDE.md`'s rule about the 51 names in
  `check.yml` changes with the task that moves them.

## Confirmation

The `Protect main` ruleset lists `gate` as a required status check.
`script/verify-required-tests` fails when a required test is missing from
the source or did not run and pass on its platform. The nightly workflow
opens an issue when a nightly suite fails, and the `fanta-release` skill
refuses a release commit without a green nightly run.

## References

- [Spec 0012](../../specs/0012-fast-gated-ci-and-the-agent-harness/spec.md)
- [ADR 0013](0013-gate-releases-on-named-verification-suites.md), which this
  record supersedes
- [ADR 0012](0012-distribute-a-notarized-dmg-with-web-billing.md),
  which moved distribution away from the Mac App Store
- #204 (per-platform lint features, which made Linux clippy possible)
