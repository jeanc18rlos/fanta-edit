---
status: accepted
date: 2026-11-12
owner: Jean Rojas
decisions: [0026, 0027]
---

# Check, repair and snapshot project files

## Problem

[Spec 0007](../0007-store-projects-as-plain-web-files/spec.md) made a project
a folder of SVG, CSS, HTML and JSON, and promised that people and agents may
edit those files directly (R7). The founder's stated fear (2026-11-12) is
that an edit made outside Fanta breaks the project. Three gaps make that
fear reasonable today:

- **Nobody is told.** The tolerant loader collects what it could not keep as
  free-text strings (`warnings: &mut Vec<String>` in
  `fanta-format/src/project/web_read.rs`) and the *next save* writes them to
  `report.json`. A person editing in another tool, or an agent that never
  saves through Fanta, sees nothing. The strings carry no code, file or
  line, so a tool cannot act on them. A structured `SourceDiagnostic` exists
  (`project/session/types.rs`) but only for the legacy FNX path.
- **The save that reports the loss also performs it.** The save after a
  rejected hand edit canonicalizes the file. The edit is listed in
  `report.json` and gone from disk, with no earlier state to return to.
- **There is no fix path.** A broken reference or a duplicated id is found
  at best as a sentence in a report; repairing it is manual.

This also blocks the larger goal the founder set the same day: the format
should be adoptable by other tools. A format other tools can adopt needs a
validator that runs without the editor.

## Goals

- One command says whether a project is valid, and where it is not, in a
  form a tool can act on.
- The same check runs automatically after any change, whoever made it.
- No state of a project that Fanta has seen is ever unrecoverable.
- An agent can turn check output into a proposed fix that the user accepts
  or rejects as a diff.
- The checker is usable, and licensed for use, without the Fanta editor.

## Non-goals

- Changing what the format stores. Storing each fact once (the SVG versus
  `nodes.json` overlap) changes spec 0007's R2 to R4 and gets its own spec;
  this spec's diagnostics are the evidence for it.
- Accepting arbitrary SVG or CSS from other tools. The dialect and the
  hand-edit subset stay as spec 0007 defines them; this spec reports what
  falls outside, it does not widen what is kept.
- An automatic `--fix` mode in the checker. Repairs go through the skill
  (R8) so every change is reviewed.
- A user-facing version history, branching or sync. Snapshots are a safety
  net with a restore action, not a collaboration feature.
- A database. Files stay authoritative
  ([ADR 0009](../../docs/decisions/0009-make-project-files-authoritative.md)).

## Requirements

- **R1.** A `check` command takes a project directory, loads it through the
  same reader the editor and headless engine use, and writes nothing. It
  exits 0 when there are no errors and non-zero otherwise. `--json` prints
  the diagnostics as JSON.
- **R2.** Every finding is a structured diagnostic: a stable code, a
  severity (`error` or `warning`), a project-relative file, a line and
  column where the reader knows them, the node id in its prefixed form where
  one applies, and a message. The reader's free-text warnings are replaced
  by these, and `report.json` carries the same records.
- **R3.** The checker must report at least: a file that does not parse; a
  duplicate id; a reference to a node, component, token or asset that does
  not exist; a sidecar record with no element and an element with no record;
  an edit outside the hand-edit subset that the next save would not keep; an
  unknown attribute or property (warning; it is preserved where the format
  already preserves it); a manifest that disagrees with the files present.
- **R4.** Diagnostic codes are documented and permanent: a code is never
  reused for a different meaning.
- **R5.** The check runs without being asked: in the editor after its file
  watcher sees an external change, with the result shown before any save;
  and through an MCP tool, in both the live and the headless server, that
  returns the same JSON as the command.
- **R6.** Fanta records a snapshot of the project files before any save
  that would rewrite a file changed outside Fanta, and after every
  successful save. Recording a snapshot must not change the user's branch,
  index, working tree or any ref under `refs/heads`, and must work whether
  or not the project is inside a git repository. Snapshots live in a private
  git directory inside the project that the user's git ignores; Fanta never
  writes to a repository it does not own. Every snapshot is kept for 30
  days, then one per day.
- **R7.** The user can list snapshots and restore one. A restore first
  snapshots the current state, so a restore can itself be undone.
- **R8.** A repair skill ships with the agent plugins and the in-app agent.
  It takes the `--json` output, proposes file changes, and re-runs the check.
  It may read the snapshots, and the user's own git history where there is
  one, to reconcile a broken state against the last valid one. In the app the proposal is shown as a diff and applies only when accepted.
- **R9.** The checker builds as a binary whose normal dependency tree
  contains no GPUI, Skia or editor crate. The founder's direction
  (2026-11-12) is that the format specification, the checker and the
  conformance fixtures are public from the first release under a permissive
  licence; that takes effect through
  [ADR 0026](../../docs/decisions/0026-choose-the-engine-and-sdk-licence.md),
  whose preconditions (SPDX fix, provenance check, counsel) still apply.
- **R10.** A corpus of hostile edits lives in the repository, each case with
  the diagnostics it must produce. It is the checker's regression suite and
  the conformance set for other tools.
- **R11.** A project with errors still opens, with the diagnostics shown,
  and can be saved, because R6 makes that save reversible. Only a file that
  does not parse is refused: it is left untouched on disk and its design is
  shown as unavailable.

## Acceptance criteria

- **A1 (R1, R2).** On a freshly written fixture covering every node kind,
  `check --json` exits 0 with an empty list.
- **A2 (R2, R3, R10).** For each corpus case the command produces exactly
  the expected codes, with file and line, and never panics. The corpus
  holds at least: a deleted page SVG; a deleted sidecar; an id renamed in
  the SVG only; an element copied and pasted (duplicate id); every file
  reformatted with Prettier (expected: no errors); an SVG re-saved by
  Inkscape; unresolved merge-conflict markers; `var()` naming a missing
  token; an instance of a deleted component; a missing asset.
- **A3 (R6, R7).** For each corpus case: open in the editor, save, restore
  the snapshot taken before the save. The project directory is
  byte-identical to the hostile state.
- **A4 (R6).** After ten saves of a project inside a git repository,
  `git status`, `HEAD`, the index and `git branch -a` are unchanged, and ten
  snapshots are listed. The same ten are listed for a project outside any
  repository.
- **A5 (R5).** With a project open, an external edit that breaks a reference
  shows the diagnostic in the editor without a save.
- **A6 (R5).** The MCP tool's result equals the command's `--json` output
  on the same project, in the live and the headless server.
- **A7 (R8).** On the corpus cases marked repairable, the check is clean
  after the skill's accepted proposal, and the proposal touches only files
  named in the diagnostics.
- **A8 (R9).** A workflow fails if GPUI, Skia or an editor crate appears in
  the checker binary's dependency tree, as the `fanta-runtime` workflow does
  for its own boundary.
- **A9 (R4).** The documented code list and the codes the corpus exercises
  match; a test fails when a code is added without documentation.
- **A10 (R11).** A project with a broken reference opens and saves; a
  project whose page SVG holds merge-conflict markers opens with that page
  unavailable and the file unchanged on disk after a save.

## Open questions

- **Q1.** ~~Where snapshots live.~~ **Resolved by the founder, 2026-11-12:**
  a private git directory inside the project, in every case (R6).
- **Q2.** ~~Retention.~~ **Resolved by the founder, 2026-11-12:** everything
  for 30 days, then one per day (R6).
- **Q3.** ~~What the editor does with a project that has errors.~~
  **Resolved by the founder, 2026-11-12:** open with diagnostics and allow
  saving; refuse only files that do not parse (R11). The founder added that
  the repair agent should see git to reconcile changes (R8).
- **Q4.** ~~Where the binary lives.~~ **Answered in `design.md`:** a binary
  target in `fanta-format`, no new crate. Original question: a new crate, or a subcommand of an
  existing engine binary (`fanta-site`, `fanta-mcp`). A new crate needs a
  layer in `layers.toml`. To settle in `design.md`.
- **Q5.** ~~Earlier git lifecycle work.~~ **Founder, 2026-11-12:** look in
  the inherited Zed code. `crates/git` has `checkpoint`,
  `restore_checkpoint`, `compare_checkpoints` and `diff_checkpoints`
  (`src/repository.rs`), which snapshot a working tree without moving a
  branch. `design.md` must say whether they can target a private git
  directory or whether the snapshot store is written beside them.
- **Q6.** Licence scope. The founder answered "yes" (2026-11-12) to whether
  the permissive licence covers the crates the checker links. Read here as:
  it covers `fanta-format` and `fanta-doc`, not only a thin checker crate.
  That is ADR 0026 option 2 for those crates and is confirmed, or corrected,
  when the founder accepts ADR 0026.

## Changes after acceptance

- 2026-11-12: the founder accepted ADR 0026 with Apache-2.0 for the engine,
  which confirms the reading of Q6: the licence covers `fanta-format` and
  `fanta-doc`.
