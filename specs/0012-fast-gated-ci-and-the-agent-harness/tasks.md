# Tasks: Fast, gated CI and the agent harness

One task per pull request. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

Order: T1 to T4 are the critical path. T5 comes early because T4's
`hygiene` job calls it. The rest may go in any order once T4 has merged,
except where a task names another.

The trigger and cache change (R1, R2) is below the spec threshold and lands
before T1 in its own pull request.

Fast CI:

- [ ] **T1.** (plan A1) Add stages `render`, `shell` and `release-blockers` to
  `script/verify-fanta-release` and take `fanta-render` out of `engine`.
  Add the `changes` and `linux-test` jobs to `check.yml` as advisory
  (R3, R4, R5). Verify: `--list` shows the stages; the Linux job is green
  on the pull request; Q1 answered in the description.
- [ ] **T2.** (plan A2) After five consecutive green `main` runs of
  `linux-test`: reduce macOS to the one `macos` job; delete the
  `release-blocker-regressions` and `mac-app-store-runtime` jobs (the
  second moves in T3, so T3 merges first or together). Move the 51 and 30
  required names into `ci/required-tests.json` with
  `script/verify-required-tests`. Update the rule in `CLAUDE.md` that
  points at `check.yml` (R4, R9). Verify: A2, A5.
- [ ] **T3.** (plan A3) `nightly.yml` with the Mac App Store suite, the macOS
  all-features lint, Skia source compatibility and the tracking issue;
  narrow `skia-source-compatibility.yml`'s pull request paths; delete
  `fanta-lint.yml`; add the nightly rule to the `fanta-release` skill
  (R6). Verify: A7.
- [ ] **T4.** (plan A4) The `gate` and `hygiene` jobs; fold `architecture.yml`
  into `hygiene`. The description carries the `gh api` command with which
  the founder adds the required check and the bypass to the ruleset
  (R7, R8). Verify: A1, A3, A4.

Agent harness:

- [ ] **T5.** (plan H1) `script/gate`, default and `--full`; shrink the
  `fanta-verify` skill (R17). Verify: A9.
- [ ] **T6.** (plan H2) The `PostToolUse` and `Stop` hooks (R18). Verify: A10.
- [ ] **T7.** (plan H3) Fix `guard_bash.py` and add its self-tests (R18).
  Below the spec threshold on its own; it may land before the spec is
  accepted. Verify: A10.
- [ ] **T8.** (plan H4) Agent pull request rules in `CLAUDE.md` and `AGENTS.md`
  (R20). Verify: A11.
- [ ] **T9.** (plan H5) Run the six lints nightly, advisory; evaluate the id
  lint with the `lint-creator` skill and record the result here (R19).
  Needs T3. Verify: the nightly log.

Product-agent evals (the plan's H6) are
spec 0014, not a task here.

Gates:

- [ ] **T10.** (plan G1) One `cargo fmt --all` commit, listed in
  `.git-blame-ignore-revs`, then the formatting check in `script/gate`
  (R11). Lands when no open pull request would conflict. Verify: A6.
- [ ] **T11.** (plan G2) `typos` and `zizmor` in `script/gate`, with the
  findings fixed (R8). Verify: A6.
- [ ] **T12.** (plan G3) `deny.toml` and `cargo deny check advisories sources
  bans` in `script/gate`, with the first run triaged (R10, Q3).
  Verify: A6.
- [ ] **T13.** (plan G4) `script/verify-required-tests --names` in
  `script/gate` (R9). Needs T2. Verify: A6.
- [ ] **T14.** (plan G5) Fuzz targets for the four parsers and their nightly
  job (R13, R14). Needs T3. Verify: A8.
- [ ] **T15.** (plan G6) Measure `clippy::unwrap_used` and
  `let_underscore_must_use` in the four parser crates; enable per crate
  where small; record the counts here (R15). Verify: `./script/clippy -p`
  for each crate.
- [ ] **T16.** (plan G7) Security review of the November agent surfaces;
  findings filed as issues and linked here (R16). Verify: A11.
- [ ] **T17.** (plan G8) `SECURITY.md` (R12). Verify: A11.
