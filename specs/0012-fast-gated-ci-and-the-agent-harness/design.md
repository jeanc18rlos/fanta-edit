# Design: Fast, gated CI and the agent harness

The founder decided the shape on 2026-11-12: one required `gate` check with
a founder bypass, Windows advisory until its cache is fixed, the Mac App
Store job nightly and before a release, and coding-agent guardrails now.
[ADR 0030](../../docs/decisions/0030-gate-merges-on-one-fast-required-check-and.md)
records the decision and supersedes ADR 0013.

No crate is added or moved and `layers.toml` does not change. Everything
here is workflows, scripts, configuration and agent rules.

## Approach

### `check.yml`

| Job | Runner | Runs |
| --- | --- | --- |
| `changes` | ubuntu | Classifies the diff into `docs`, `rust` and `workflows`, so jobs skip instead of the workflow not starting. A workflow that does not start cannot satisfy a required check; a skipped job can. |
| `hygiene` | ubuntu, no compile | `script/gate`. Runs on every pull request, docs included. |
| `linux-test` | ubuntu | Stages `engine` (without `fanta-render`), `import`, `source-order`, `workspace`, `ui`, `shell` and `release-blockers`; the wasm check; `script/check-licenses`; pull request clippy. |
| `macos` | macos-26, one job | `cargo check --locked -p fanta -p cli`, stages `editor` and `render`, `script/test-macos-release`. |
| `gate` | ubuntu, `if: always()` | Passes when every job in `needs` is `success` or `skipped`. The only required check. |

Triggers: `push` on `main` only; `pull_request` with `ready_for_review`
added to its types; the compiling jobs skip on draft pull requests. This
part is below the spec threshold and lands first, on its own.

Caches: every `Swatinem/rust-cache` use gets
`save-if: ${{ github.ref == 'refs/heads/main' }}` and one `shared-key` per
job. The Linux shards share one key. Budget: about 2.7 GiB for macOS and 2
to 3 GiB each for Linux and Windows, under the 10 GB limit. The release
workflows stop saving caches from pull request runs.

`architecture.yml` folds into `hygiene`. `linux.yml`, `windows.yml` and
`fanta-runtime.yml` keep their own triggers and are not in `gate`'s `needs`
(Windows is advisory by decision).

### Stages

`script/verify-fanta-release` gains three stages so that YAML names stages
instead of commands:

- `render`: `fanta-render`, split out of `engine`. It holds the golden
  image test, which only macOS passes today.
- `shell`: the per-crate test steps that trail the `check` job today (13
  at `9716f162`: bundled Git, languages, AI provider authentication,
  browser sign-in, media server account, Save As prompts, update
  verification, agent panel toggle, design prompt preparation, design
  source validation, the hostile-edit corpus, the checker's dependency
  boundary, sidebar).
- `release-blockers`: the eight test commands of the
  `release-blocker-regressions` job and its default-feature clippy.

`engine` keeps its name and loses `fanta-render`, so a local `--stage
engine` passes on Linux.

### Required tests

`ci/required-tests.json` replaces the two inline Python blocks:

```json
{
  "tests": [
    {"name": "property_draft_explicit_save_commits_focused_input", "stage": "editor", "platform": "macos"},
    {"name": "test_close_window_reports_save_failure_and_retries", "stage": "workspace", "platform": "any"}
  ]
}
```

`script/verify-required-tests` has two modes. `--names` greps the source
for `fn <name>` and fails unless each name exists exactly once; it runs in
`hygiene`. `--results <dir> --platform <linux|macos>` reads the stage logs
and `results.tsv` and fails unless each test for that platform ran once and
passed; it runs after the stages in `linux-test` and `macos`. Names match
on the last path segment, as today, so moving a test between modules does
not change the list.

The per-step source-hash evidence of the release-blocker job (file hashes,
copied configuration) is dropped. Logs and `results.tsv` stay as the
uploaded artifact.

### Nightly

`nightly.yml` runs on a schedule and on manual dispatch: the Mac App Store
suite (today's `mac-app-store-runtime` job), the macOS all-features lint
(today's `fanta-lint.yml`), Skia source compatibility, the fuzz targets
and the custom lints. A final job opens or updates one issue labelled
`nightly-failure` with `gh issue`. `skia-source-compatibility.yml` keeps a
pull request trigger for the paths that affect Skia. The `fanta-release`
skill gains one line: the release commit's nightly run must be green.

### `script/gate`

A Python script, like `script/architecture`. Default mode, no compile:

| Check | Tool |
| --- | --- |
| Formatting | `cargo fmt --all -- --check` |
| Spelling | `typos` with the existing `typos.toml` |
| Architecture and records | `script/architecture verify` |
| Workflows | `zizmor .github/workflows` |
| Dependencies | `cargo deny check advisories sources bans` with a new `deny.toml` |
| File size | The ratchet of spec 0013, once it lands |
| Required tests exist | `script/verify-required-tests --names` |

A tool that is not installed is reported as skipped locally and is an error
when `CI` is set, so the local command works on a fresh machine and CI
cannot skip silently.

`--full` runs the default checks, then maps `git diff --name-only
origin/main...` to crates, runs `./script/clippy -p` for them and the
`verify-fanta-release` stages that cover them. The `fanta-verify` skill
shrinks to "run `script/gate --full`" plus the macOS-only caveats.

Licences stay with `script/check-licenses`; `cargo deny` does not check
them.

### Hooks

`.claude/settings.json` gains two hooks. `PostToolUse` on `Edit` and
`Write` runs `rustfmt` on the edited `.rs` file. `Stop` runs `script/gate`
and returns its failure text once; a second stop goes through, so a gate
that cannot pass does not trap a session.

`guard_bash.py` tokenizes the command with `shlex` before it splits on
shell operators, so an operator inside quotes stays inside its word. It
gains a `--self-test` mode that `script/gate` runs.

### Fuzzing

One `fuzz/` directory per parser crate, made with `cargo fuzz init`, kept
out of the workspace so the nightly toolchain it needs does not reach
normal builds. Corpora are seeded from each crate's test fixtures.

## Interfaces and data

- `ci/required-tests.json`: the schema above. `platform` is `any`, `linux`
  or `macos`.
- `deny.toml`: `advisories` (vulnerabilities deny, unmaintained warn, each
  `ignore` with a `reason`), `sources` (crates.io plus the pinned git
  sources in `Cargo.toml`), `bans`.
- The `Protect main` ruleset (id 25141618) gains a required status check
  `gate` and the founder as bypass actor. The founder applies it with the
  `gh api` command in T4's pull request; an agent does not change rulesets.

## Alternatives

- **Require every job.** Path-filtered workflows that do not start leave a
  required check pending forever. One aggregating job avoids that.
- **Keep everything on macOS and buy faster runners.** It pays for runner
  time that 66 of the 81 required tests do not need. Faster runners remain
  the escalation if A1 fails.
- **GitHub's merge queue.** Not available to this repository until spec
  0003's transfer.
- **A pre-commit framework instead of `script/gate`.** A second tool and a
  second configuration; agents in worktrees would each need it installed.
- **A compiling `Stop` hook.** It would hold a session for the length of a
  build. `--full` is a rule (R20), not a hook.

These are one decision, recorded in ADR 0030.

## Risks

- **Editor tests on headless Ubuntu** may differ from the Linux desktop
  (Q1). The `editor` stage stays on macOS, and the Linux jobs are advisory
  until five consecutive green `main` runs.
- **The cache budget is tight.** If eviction returns, drop the Windows
  cache first; it is advisory.
- **The formatting commit conflicts with open pull requests.** It lands
  when the backlog is merged or closed.
- **Store code can rot between nightlies.** A break shows within a day
  instead of per push; the release rule in R6 keeps it out of a release.
- **`cargo deny` reports inherited advisories** on its first run (Q3). The
  enabling pull request triages them.
- **Backing out:** each task is one pull request and reverts on its own.
  Removing the required check from the ruleset is one `gh api` call.

## Verification

- A1, A2: medians from `gh run list --workflow Check --json` before T1 and
  after T4 (time to green, queue time, macOS job-minutes).
- A3, A6: throwaway pull requests, one per violation, closed afterwards.
- A4: `gh api repos/{owner}/{repo}/actions/caches`.
- A5: the required-test report artifact of a `main` run.
- A7: `gh workflow run nightly.yml` with a seeded failure, twice.
- A8: the nightly log, and a local revert of one #176 fix.
- A9, A10: `script/gate`, the guard's `--self-test`, and one agent session.
- A11: by reading.
