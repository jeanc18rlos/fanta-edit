---
status: draft
date: 2026-11-12
owner: Jean Rojas
decisions: [0013, 0030]
---

# Fast, gated CI and the agent harness

## Problem

Every pull request waits on CI that rarely finishes, nothing stops a red
change from merging, and a coding agent gets no feedback until that CI runs.
The founder asked on 2026-11-12 to stop waiting on CI and to make agent
edits safer. That day 24 pull requests were merged on a local Linux run
because the hosted queue was hours deep.

Measured on 2026-11-12. The CI timings were taken at `992c3c77`; the rest
was checked again at `9716f162`.

| Problem | Evidence |
| --- | --- |
| CI does not finish | The last green `Check` run took 476 minutes of wall-clock time for about 72 minutes of work per job: its three macOS jobs waited 305 to 405 minutes in the queue. None of the 60 runs before the measurement completed. |
| The queue is deep by construction | `check.yml` runs on `push` to every branch and on `pull_request`, so one push to a pull request starts two runs of three macOS jobs each (`check`, `mac-app-store-runtime`, `release-blocker-regressions`) against a limit of five concurrent macOS jobs. A pull request that changes only a spec runs all of it. |
| Builds start cold | The macOS jobs share one cache key (`macos-check`) and any branch may save it. A cache saved by a feature branch cannot be read by other branches, and the repository's 10 GB limit evicts the Linux and Windows caches. `main` had no macOS cache. |
| Work runs on the most expensive runner | 15 of the 51 required editor and workspace regressions are `#[cfg(target_os = "macos")]`. Those, the golden render test and macOS packaging need a Mac; the other suites pass on Linux (clippy too, since #204). A 60-minute job checks the Mac App Store channel on every push, although [ADR 0012](../../docs/decisions/0012-distribute-a-notarized-dmg-with-web-billing.md) moved distribution away from the Store. |
| Nothing is enforced | The `Protect main` ruleset blocks deletion and force pushes only; no check is required. CI does not run `cargo fmt --check` (355 unformatted hunks at `9716f162`), `typos` (the config exists), the six lints in `tooling/lints`, a dependency advisory check or a workflow linter. |
| Required tests are a hand-kept list in YAML | 51 names in one inline Python block of `check.yml` and 30 in another ([ADR 0013](../../docs/decisions/0013-gate-releases-on-named-verification-suites.md)). A renamed test is found after the 40-minute build that precedes the check. |
| Agents get no feedback | One Claude Code hook exists, `.claude/hooks/guard_bash.py`. It blocks unscoped clippy, has no tests, and wrongly denies a `grep` whose quoted pattern contains `\|script/clippy `. There is no single command an agent or a person runs before pushing, and nothing checks an agent's tree before it stops. |
| Untrusted input is not fuzzed | Four parsers read files from outside (`.fig`, FNX, the SVG and CSS web reader, bundles). #176 fixed crashes in them found by reading; no fuzz target exists. |

## Goals

- A pull request learns whether it may merge within half an hour; a
  docs-only pull request within three minutes.
- One required check decides whether a pull request may merge.
- Each suite runs on the cheapest runner that can witness what it tests.
- Formatting, spelling, workflow safety, dependency advisories and the
  required-test list are checked before anything compiles.
- One command runs the same checks locally and in CI, and agent hooks call
  it.

## Non-goals

- Extracting the Zed shell, or splitting `fig_viewer` into crates.
- Deleting the `mac_app_store` feature. It moves to the nightly tier (R6).
- The repository transfers of [spec 0003](../0003-github-organization/spec.md).
  They would unlock GitHub's merge queue; revisit then.
- Faster or self-hosted runners. That is the escalation if A1 is not met,
  not part of this spec.
- A coverage-percentage gate, CodeQL, or publishing to crates.io.
- Review by an AI running in CI. It needs a secret that pull request
  workflows could read today; it waits for the agents' own GitHub App
  identity ([ADR 0025](../../docs/decisions/0025-organize-the-repositories-under-one-github-organization.md)).
- Evaluating the product's design agent. That is
  spec 0014.
- Changing the release workflows, apart from their cache settings (R2).

## Requirements

Fast CI:

- **R1.** One push to a pull request must start one `Check` run, not two.
  Draft pull requests must skip the compiling jobs.
- **R2.** Only runs on `main` may save a build cache. Every job that
  compiles restores `main`'s cache, and the caches together fit the
  repository's limit.
- **R3.** `Check` must start for every pull request and classify the diff,
  so that jobs skip when nothing they test changed. A docs-only or
  spec-only pull request runs no compiling job.
- **R4.** Every suite that does not need macOS must run on Linux. macOS
  runs one job, holding only what macOS alone can witness: the app and
  command line build check, the `editor` stage, the render stage with the
  golden image, and the macOS packaging tests.
- **R5.** Workflow files must call named stages of
  `script/verify-fanta-release`; test commands are not written inline in
  YAML.
- **R6.** The Mac App Store suite, the macOS all-features lint, Skia source
  compatibility, fuzzing (R14) and the custom lints (R19) must run nightly
  and on manual dispatch, off the pull request path. A nightly failure
  opens or updates one tracking issue. A release is built only from a
  commit whose nightly run passed.

Gates:

- **R7.** One job named `gate` must pass exactly when every job the diff
  needed succeeded or was skipped. It is the only check the `main` ruleset
  requires. The founder can bypass it. Windows stays advisory until its
  cache is fixed.
- **R8.** A `hygiene` job must run on every pull request, without
  compiling, in about two minutes: formatting, spelling, the architecture
  and record checks, a workflow linter, dependency advisories, sources and
  bans, and R9.
- **R9.** The required regressions must live in one checked-in data file
  with a platform for each test. `hygiene` fails when a listed name does
  not exist exactly once in the source. After the suites run, a script
  fails when a listed test did not run and pass on its platform. Every
  name required today stays required.
- **R10.** A dependency with a known vulnerability must fail `hygiene`; an
  unmaintained one warns. Each ignored advisory carries a written reason.
- **R11.** The tree must be formatted in one commit listed in
  `.git-blame-ignore-revs` before the formatting check becomes binding.
- **R12.** The repository must have a `SECURITY.md` that says how to report
  a vulnerability and which versions are supported.

Untrusted input:

- **R13.** The four parsers of outside files must each have a fuzz target,
  seeded from the existing fixtures and the crash inputs of #176:
  `fanta-fig-interop` (kiwi and zip), `fanta-fnx`, the web reader in
  `fanta-format`, and `fanta-bundle`.
- **R14.** Each fuzz target must run nightly on Linux for five minutes.
- **R15.** `clippy::unwrap_used` and `let_underscore_must_use` must be
  measured in those four crates and enabled per crate where the count is
  small enough to fix in the same pull request.
- **R16.** The agent surfaces added in November must get one security
  review, with each finding filed as an issue: the live MCP server
  (`fig_viewer/src/live_mcp.rs`), `fanta-mcp`, the browser edit host and
  `script/serve-project`, the plugin pane, and the cloud worker's HTTP API
  (#178).

Agent harness:

- **R17.** `script/gate` must run the `hygiene` checks locally in seconds,
  without compiling, and exit non-zero on any failure. `script/gate --full`
  must also map the files changed against `origin/main` to scoped clippy and
  to `script/verify-fanta-release` stages. CI's `hygiene` job calls the same
  script.
- **R18.** Claude Code hooks must format an edited Rust file after each
  edit, and run `script/gate` when the agent stops, blocking once with the
  failure text. No hook compiles. The Bash guard must not treat text inside
  quotes as a command, and must have self-tests.
- **R19.** The six lints in `tooling/lints` must run nightly on Fanta's
  GPUI crates, advisory until the tree is clean. A lint for the id
  serde-versus-`Display` trap must be evaluated, and written if it is
  feasible.
- **R20.** The agent rules must say: open pull requests as drafts; mark one
  ready after `script/gate --full`; before that, run a code review (and a
  security review when the diff touches parsers, IPC, update or signing
  code) and summarize it in the description. `AGENTS.md` names
  `script/gate` for Codex, which has no hooks.

## Acceptance criteria

- **A1 (R1 to R5).** Over the 20 pull request runs after T4 merges, the
  median time from push to a green `gate` is at most 30 minutes for a pull
  request that changes Rust and at most 3 minutes for one that changes only
  docs or specs. Measured with `gh run list`, before and after.
- **A2 (R4, R6).** One push to a Rust pull request uses at most 45 macOS
  job-minutes (about 415 at `992c3c77`).
- **A3 (R3).** A docs-only pull request runs `changes`, `hygiene` and
  `gate` and nothing else.
- **A4 (R2).** `gh api repos/{owner}/{repo}/actions/caches` lists build
  caches for `refs/heads/main` only, and under 10 GB in total.
- **A5 (R9).** The required-test report of a `main` run lists all 81 names
  as run and passed, each on its platform.
- **A6 (R7 to R11).** Each gate fails when it is violated once on a
  throwaway branch: an unformatted file, a renamed required test, a
  workflow with an unpinned action, a dependency with a known advisory, a
  misspelled word. `gate` is red in each case and the ruleset refuses the
  merge.
- **A7 (R6).** A nightly run executes the five suites of R6, and a seeded
  failure opens one issue that the next failure updates rather than
  duplicates.
- **A8 (R13, R14).** Each fuzz target runs five minutes in the nightly job
  without a crash, and reproduces a seeded crash input from #176 when its
  fix is reverted locally.
- **A9 (R17).** `script/gate` exits 0 on a clean tree and non-zero on each
  seeded violation of A6, in under 30 seconds.
- **A10 (R18).** The guard's self-tests pass, including the quoted `grep`
  case. A session that leaves an unformatted file is stopped once by the
  hook with the failure text.
- **A11 (R12, R16, R20).** `SECURITY.md` exists; the review's findings are
  open issues linked from T16; `CLAUDE.md` holds the pull request rules.

## Open questions

- **Q1.** Do `editor`-stage tests that are not macOS-only pass on a
  headless Ubuntu runner (fonts, display)? They pass on the Linux desktop.
  T1 answers it; until then the whole `editor` stage stays on macOS, as R4
  says.
- **Q2.** Does the Linux golden image get its own baseline? Today
  `tier1_golden_scene_matches` fails on Linux by text glyph edges (FreeType
  against CoreText), so the render stage stays on macOS. A per-platform
  baseline is the founder's decision and is not needed for this spec.
- **Q3.** ~~How many advisories does the first `cargo deny` run report in
  the inherited dependencies?~~ **Answered by T12, 2026-11-13:** 43
  advisories on 39 crate versions with cargo-deny 0.20.2: 24
  vulnerabilities, 4 unsound and 15 unmaintained. One pull request was
  enough: the 28 that fail are ignored in `deny.toml` with a reason each
  and no dependency was bumped; the 15 unmaintained warn. One is reachable
  from Fanta's own code, RUSTSEC-2026-0194 in `quick-xml`, a slow load on
  a crafted project SVG. The bumps are follow-up work.

## Changes after acceptance

- None.
