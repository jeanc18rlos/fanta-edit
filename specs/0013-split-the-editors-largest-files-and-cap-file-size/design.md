# Design: Split the editor's largest files and cap file size

## Approach

All five files are in `fig_viewer` (desktop layer). No crate is added or
moved and `layers.toml` does not change.

**Tests first.** In each file the inline `mod tests` is the largest single
block, and moving it is the safest move there is: `#[cfg(test)] mod tests;`
replaces the block and the body goes to `<file>/tests.rs`. The module path
stays `<file>::tests`, so every test keeps its full name. Where the test
module is itself above 5,000 lines (`view.rs`: 12,172; `design.rs`: 7,778;
`document.rs`: 6,527) it is split again into `tests/<topic>.rs` submodules
that share the helpers through `use super::*`. That changes the middle of a
test's path, not its last segment, which is what the required-test check
matches; a CI step that filters by a longer path is checked by hand in the
same pull request (R3).

**Then the implementation.** Rust allows several `impl` blocks for one type
across modules of the same crate, so a cluster of methods moves to a
submodule as `impl FigView { … }` with `use super::*`. `view.rs` already
does this: it has five `impl FigView` blocks, four submodules under `view/`
(`agent_presence`, `properties_inspector`, `serialization`,
`timeline_adapter`) and six `#[path = "view_*.rs"]` siblings. New
submodules go under `view/`; the `#[path]` siblings stay where they are,
because moving them is churn with no gain.

The clusters are chosen per file in its pull request by reading the code,
not fixed here. For `view.rs` the starting point is the 6,540-line first
`impl FigView` (lines 754–7294, 202 methods), which holds construction and
mounting, tool and pointer handling, clipboard and paste, page and
selection actions, and save notices.

A private item that a moved method uses becomes `pub(super)`. A private
field of `FigView` is already visible to child modules, so most moves need
no visibility change.

`gpui_adapters/mod.rs` exists today. This spec does not rename it; the split
of `design.rs` creates `gpui_adapters/design/*.rs` beside it.

**Order.** One pull request per file, each opened when no open pull request
touches that file, because a move conflicts with every concurrent edit.
`view.rs` goes first as two pull requests (tests, then implementation), so
the review of a pure test move sets the pattern.

**Ratchet.** `script/architecture` gains a `sizes` subcommand, and `verify`
runs it. It reads `docs/architecture/file-sizes.toml`:

```toml
limit = 2000
roots = ["crates/fig_viewer", "crates/design_surface", "crates/fanta_ui", "crates/fanta_languages", "crates/fanta-*"]

[baseline]
"crates/fig_viewer/src/view.rs" = 24112

[exclude]
"crates/<crate>/src/<file>.rs" = "why this file is not counted, for example generated code"
```

The script counts lines of every `.rs` file under the roots. A file in
`[baseline]` fails when it is longer than its entry; any other file fails
above `limit`; an entry larger than its file fails too, so the baseline
only moves down. `sizes --write` rewrites the entries to the current sizes
and drops the ones at or under the limit; it refuses to raise one.
`script/test-architecture` covers the three failures of A4.

The example entries are illustrative. The first ratchet pull request
generates the real baseline, and it lands before the first split so that
each split shows its gain as a lowered entry.

## Interfaces and data

- `docs/architecture/file-sizes.toml`: `limit`, `roots`, `[baseline]`
  (path → lines) and `[exclude]` (path → reason).
- `script/architecture sizes [--write]`, and `verify` including it.
- No Rust interface changes: a split adds private modules only.

## Alternatives

- **Split `fig_viewer` into crates.** It would cut build times as well, but
  it needs new layers, moves public interfaces and conflicts with every
  open editor pull request. The founder ruled it out on 2026-11-12.
- **`clippy::too_many_lines` or a dylint.** Clippy's lint is per function,
  not per file, and a dylint needs a compile. A line count needs neither.
- **A fixed cap with no baseline.** It would fail on day one for 22 files
  or need a cap so high that it stops nothing.
- **A separate test crate for the editor tests.** The tests use private
  items of `fig_viewer`; a child module keeps that access.

## Risks

- **A move changes behaviour by accident,** for example through a `use`
  that resolves differently in the new module. Caught by `cargo check` and
  the unchanged test list (A3). Back out by reverting the one pull request.
- **Conflicts with open pull requests.** Each split waits until its file is
  free, and is rebased and merged the same day.
- **Macro-heavy test modules** may depend on declaration order
  (`macro_rules!` is textual). A macro used across the new files moves to
  the top of `tests.rs` before the `mod` lines.
- **The baseline becomes a place to hide growth** if `--write` could raise
  an entry. It cannot; raising one is a hand edit a reviewer sees.

## Verification

- **A1.** `wc -l` on the five files and their directories, in the last
  split's pull request.
- **A2.** `git diff --color-moved=zebra --stat` and the count of non-moved
  lines, pasted in each pull request.
- **A3.** `cargo test --locked -p fig_viewer --lib -- --list` before and
  after, compared as sorted last segments; `cargo check --locked -p fanta
  -p cli`; `./script/verify-fanta-release editor`. On Linux the 15
  macOS-only required tests are absent from the binary, so CI's macOS job
  is the witness for those.
- **A4, A5.** `script/test-architecture` cases for the ratchet.
