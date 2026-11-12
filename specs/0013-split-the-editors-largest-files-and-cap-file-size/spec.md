---
status: draft
date: 2026-11-12
owner: Jean Rojas
decisions: [0023]
---

# Split the editor's largest files and cap file size

## Problem

Anyone who changes the editor, a person or an agent, has to load files that
are too large to read. An agent that opens `view.rs` spends most of its
context on one file, and two pull requests that touch the same 24,000-line
file conflict even when their changes are unrelated.

Measured on `main` at 9716f162 (2026-11-12), counting the Rust files Fanta
owns (`fig_viewer`, `design_surface`, `fanta_ui`, `fanta_languages` and the
engine crates):

| File | Lines | Of which the inline `mod tests` |
| --- | ---: | ---: |
| `fig_viewer/src/view.rs` | 24,112 | 12,172 |
| `fig_viewer/src/gpui_adapters/design.rs` | 17,674 | 7,778 |
| `fig_viewer/src/document.rs` | 11,364 | 6,527 |
| `fig_viewer/src/canvas.rs` | 11,000 | 2,697 |
| `fig_viewer/src/generation_workspace.rs` | 10,383 | 3,951 |

These five hold 74,533 lines. Seventeen more files exceed 3,000 lines:

| File | Lines |
| --- | ---: |
| `fig_viewer/src/properties_panel.rs` | 6,866 |
| `fanta-present/src/tests.rs` | 6,502 |
| `fanta-format/src/project/session/tests.rs` | 6,323 |
| `fig_viewer/src/prototype_panel.rs` | 5,369 |
| `fig_viewer/src/code_workspace.rs` | 5,207 |
| `fanta-headless/src/apply.rs` | 4,980 |
| `fanta-render/src/raster/tests/split.rs` | 4,365 |
| `fig_viewer/src/properties_render.rs` | 4,099 |
| `fanta-present/src/fanta_present.rs` | 4,069 |
| `fig_viewer/src/design_panel.rs` | 3,881 |
| `fanta-format/src/project/session/workspace.rs` | 3,663 |
| `fig_viewer/src/text_edit.rs` | 3,516 |
| `fanta-format/src/project/write.rs` | 3,512 |
| `fanta-render/src/raster/split.rs` | 3,373 |
| `fanta-render/src/raster/camera_cache.rs` | 3,231 |
| `fig_viewer/src/comments_ui.rs` | 3,175 |
| `fig_viewer/src/timeline.rs` | 3,026 |

Nothing stops them growing. `view.rs` had 23,910 lines when
[ADR 0023](../../docs/decisions/0023-enforce-layered-crate-boundaries.md)
recorded the size of `fig_viewer` as debt on 2026-10-27, and has grown by
202 lines since.

The founder decided on 2026-11-12 to work on structure step by step: split
the five largest files and add a file-size ratchet, with no big-bang crate
split.

## Goals

- No Fanta-owned Rust file is too large to read in one sitting, starting
  with the five largest.
- A file that is already too large can only shrink.
- The splits change no behaviour and can be reviewed as moves.

## Non-goals

- Splitting `fig_viewer` into several crates, or extracting the inherited
  shell.
- Changing a signature, a visibility beyond what a move forces, or any
  behaviour. A refactor found along the way gets its own pull request.
- Splitting the seventeen files between 3,000 and 10,000 lines. The ratchet
  (R4) keeps them from growing; they are split when a change needs it.
- Moving GPUI-free logic out of `fig_viewer` into engine crates. ADR 0023
  already asks for that when a change touches such a module.
- Deleting inherited crates. After #176 the remaining candidates are in use.
- The sweep of unreferenced scripts and root files. It is one pull request
  below the spec threshold and is listed in `tasks.md` only so it is not
  forgotten.

## Requirements

- **R1.** Each of the five files in the first table must be split into the
  file plus submodules in a directory of the same name (`view.rs` and
  `view/*.rs`), never a `mod.rs`. After its split, the file and each new
  file must hold at most 2,000 lines, except a test module, which must hold
  at most 5,000.
- **R2.** A split must be a mechanical move: no change to a function body,
  a signature or behaviour. The only edits allowed besides moved lines are
  `mod` declarations, `use` lines and the visibility a moved item needs to
  stay reachable (`pub(super)` or `pub(crate)`, never `pub`).
- **R3.** A split must not change the name CI knows a test by. The required
  regressions in `.github/workflows/check.yml` are matched by the last path
  segment, and some steps filter by module path (for example
  `video_playback::tests`); both must keep matching the same tests, and the
  number of tests in the crate must not change.
- **R4.** A check must fail when a Fanta-owned `.rs` file that is not in the
  baseline exceeds 2,000 lines, or when a file in the baseline is longer
  than its recorded size. The baseline is checked in, and a pull request that
  shrinks a file lowers its entry.
- **R5.** The check must run without compiling, locally and in CI, and its
  failure message must name the file, its size and its limit.
- **R6.** Generated files and test fixtures must be excluded by path in the
  baseline file, each with a reason, not by a pattern in the script.

## Acceptance criteria

- **A1 (R1).** After the last split, `wc -l` shows no file among the five
  and their submodules above 2,000 lines, or 5,000 for a test module.
- **A2 (R2).** For each split, `git diff --color-moved=zebra` shows only
  moved lines apart from `mod`, `use` and visibility lines, and the pull
  request states the count of each.
- **A3 (R3).** For each split, the list from
  `cargo test --locked -p fig_viewer --lib -- --list` holds the same number
  of tests before and after, and the same set of last path segments.
  `cargo check --locked -p fanta -p cli` passes, stage `editor` of
  `script/verify-fanta-release` passes, and the required-test report names
  the same 51 tests.
- **A4 (R4, R5).** On a throwaway branch, adding a 2,001-line file fails the
  check, adding one line to a baseline file fails it, and removing a line
  from a baseline file without lowering the entry passes.
- **A5 (R4).** The baseline has no entry larger than the file it names.

## Open questions

- **Q1.** Is 2,000 lines the right cap for a new file, and 5,000 for a test
  module? The plan set 2,000; the test-module allowance is this spec's
  proposal, because the `view.rs` tests alone are 12,172 lines.
- **Q2.** Where does the ratchet run before `script/gate` exists? This spec
  proposes a subcommand of `script/architecture`, which CI already runs
  without a build, and moving the call into `script/gate` when
  spec 0012 (fast, gated CI and the agent
  harness, drafted alongside this spec) delivers it.
- **Q3.** Does the baseline cover inherited crates that Fanta edits heavily
  (`agent`, `agent_ui`, `workspace`)? This spec proposes no: only the crates
  listed under Problem.

## Changes after acceptance

- None.
