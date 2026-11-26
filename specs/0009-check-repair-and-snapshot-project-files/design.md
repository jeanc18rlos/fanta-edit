# Design: Check, repair and snapshot project files

## Approach

Everything that reads or writes project files already lives in
`fanta-format` (persistence layer), so the checker and the snapshot store go
there too. No new crate and no change to `layers.toml`.

**Diagnostics.** `SourceDiagnostic` and `SourceSeverity`
(`project/session/types.rs`) already carry a stable code, a node id and a
message for the legacy FNX path. They gain optional `file`, `line` and
`column`, and replace the free-text `Vec<String>` that
`project/web_read.rs`, `read_project_tree_with_warnings` and
`WorkspaceSession::open_warnings` pass around today. `report.json` writes
the same records.

**Whole-project check.** `check_project` (`project/check.rs`) works in two
passes. It first reads every source file on its own, so a file that does not
parse, holds conflict markers or repeats an id is reported with its position
instead of ending the run. It then loads the project through the editor's
reader, adds what that reader reports, and checks what only the loaded
document can show: instances of missing components, paints of missing
assets, and, by projecting the document the way the writer would, files and
elements that are gone from disk. After a file error the loader's own
reports are left out, because they echo the cause.

**Collecting load.** Before T3 a design record that did not parse failed
the whole open, and a design SVG or stylesheet that did not parse was read
past without a word: the page loaded from its records and the next save
wrote over the broken file. Now the per-file pass of the check runs when a
project opens. A page or component folder holding a file that does not
parse (its header, its records, its SVG or its stylesheet) is left out of
the document and out of the session's index, and is reported as
`design.unavailable` with the parse finding beside it.

A save must then not treat the folder as stale. Three things keep it:
the writer skips folders named by `design.unavailable` findings it is
handed; every session, also one opened later without a document, carries
those findings; and before removing any file the writer checks for itself
whether the file's folder parses. The last one is what holds when a caller
passes no findings. A new design whose name maps to an unavailable folder
fails the save instead of writing into it.

A file outside a design folder (`fanta.json`, `doc/*.json`, a set's
`set.json`) that does not parse still fails the open: there is no design to
leave out.

**Command.** A `[[bin]]` target `fanta-check` in `fanta-format`
(`src/check_main.rs`): one path argument and `--json`, parsed from
`std::env::args`. It replaces `examples/check_project.rs`, which already
loads a project without writing and prints the first error; the binary keeps
that behaviour and adds the diagnostics. Its dependency tree is `fanta-format`'s own, which the
persistence layer already forbids from reaching Skia. This answers Q4.

**Snapshots.** A native-only module `project/history.rs` in `fanta-format`
drives the `git` binary against a private bare repository at
`<project>/.fanta/history.git`, with `GIT_DIR` and `GIT_WORK_TREE` set so
the user's repository is never read or written. It follows the method of
Zed's `checkpoint` in `crates/git/src/repository.rs` (stage everything into
an index, `write-tree`, `commit-tree`) but cannot call it: the engine layers
may not depend on shell crates, and headless saves need snapshots too. This
answers Q5.

- Each snapshot is a parentless commit under its own ref,
  `refs/snapshots/<utc timestamp>`. Git still shares unchanged files between
  snapshots, any two can be diffed, and pruning is deleting refs and running
  `git gc`; a chained history could not drop its middle.
- Restore records a snapshot of the current state, then makes the working
  files equal to the chosen snapshot's tree, removing files it lacks.
- `.fanta/` holds its own `.gitignore` containing `*`, so the user's git
  ignores it and their `.gitignore` is not edited. Walkers of the whole
  project directory skip it: the change scans in `fanta-mcp` and
  `script/serve-project` by name, the headless change set and the cloud
  worker because they skip hidden entries, and the editor through
  `file_scan_exclusions`. The loader, the check, the save, `.fant` export
  and `fanta-site` read named directories only and never reach it.
- A snapshot holds every file under the project except `.fanta/` and a
  `.git` directory, whatever the project's `.gitignore` says, with no
  line-ending or filter conversion. The user's and the system's git
  configuration are shut out of every command.

**Wiring.** The save path in `fanta-format` (`project/write.rs`) calls the
store, so the editor, `fanta-headless`, `fanta-mcp` and the cloud worker
all get R6 from one place. A snapshot is taken before a save when the
session's expected disk hashes differ from disk (the condition the
reconcile already computes), and after every successful save.

**Editor.** `fig_viewer`'s `document.rs` already debounces watcher events
(`schedule_watcher_check`) and reconciles external changes. After a
reconcile it runs `check_project` on the background executor and stores the
result on the document entity; a banner shows the count and opens the list.
A design left out by the collecting load renders as unavailable (R11).
Restore is an action listing snapshots by time.

**Agents.** One tool, `check_project`, defined with the other shared tool
schemas and served by `live_mcp.rs` and `fanta-mcp`. The repair skill is
text: it ships in the plugin's skills and in the guide seeded into
projects, and tells the agent to run the check, read the snapshot diff with
plain `git --git-dir=.fanta/history.git`, edit, and re-check. In the app
the agent's existing edit review shows the diff for acceptance.

## Interfaces and data

```rust
// fanta-format, project/session/types.rs (extended)
pub struct SourceDiagnostic {
    pub severity: SourceSeverity,      // Error | Warning | Info
    pub code: &'static str,            // e.g. "ref.missing_token"
    pub file: Option<String>,          // project-relative, forward slashes
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub node_id: Option<String>,       // prefixed Display form
    pub node_name: Option<String>,
    pub message: String,
}

// fanta-format, project/check.rs
pub fn check_project(dir: &Path) -> Result<Vec<SourceDiagnostic>>;

// fanta-format, project/history.rs (not built for wasm)
pub struct Snapshot { pub id: String, pub taken_at: SystemTime, pub reason: SnapshotReason }
pub enum SnapshotReason { BeforeSave, AfterSave, BeforeRestore }
pub struct History { /* project directory, git binary */ }
impl History {
    pub fn new(dir: impl Into<PathBuf>) -> Self;          // `git` from PATH
    pub fn with_git(self, git: impl Into<OsString>) -> Self; // the editor's bundled git
    pub fn record(&self, reason: SnapshotReason) -> Result<Snapshot>;
    pub fn list(&self) -> Result<Vec<Snapshot>>;           // newest first
    pub fn restore(&self, id: &str) -> Result<Snapshot>;   // returns the BeforeRestore snapshot
    pub fn prune(&self, now: SystemTime) -> Result<()>;    // 30 days, then one per day
}
```

- `fanta-check <dir> [--json]`: exit 0 with no `error` diagnostics, 1 with
  some, 2 when the directory is not a project. JSON is the array of
  diagnostics, sorted by file, line, code.
- Code families: `parse.*`, `id.*`, `ref.*`, `record.*`, `edit.*`,
  `unknown.*`, `manifest.*`, `snapshot.*`. The list lives in
  `docs/` beside the project schema; a test compares it with the codes in
  the source (A9).
- `report.json` changes from strings to these records. It is a generated
  file with no reader in the repository other than tests.
- Corpus: `crates/fanta-format/tests/corpus/<case>/` holds a project and
  `expected.json` (codes with file and line).
- Seeded guide: changing `AGENTS_MD` needs the replaced text's SHA-256 in
  `previous_seeds` (`fanta-format/AGENTS.md`).

## Changes during implementation

- **T7.** The repair skill is one file,
  `examples/claude-plugin/skills/fanta-repair/SKILL.md`, compiled into the
  app as a built-in skill as well, so the plugin and the in-app agent
  cannot drift. The seeded project guide carries the short form and names
  the skill.
- **T7.** No new code shows the in-app proposal: the agent repairs by
  editing files, and file edits already go through the agent's edit review.
- **T7.** The corpus folders carry no history, so the four deletion cases
  are repairable only with a snapshot. The A7 run gave them one.
- **T7.** A7 holds per design, not per file: `ref.missing_token` and
  `edit.not_kept` name the design's SVG while the repair is in the
  stylesheet beside it. The skill allows exactly that.

- **T6.** The editor shows the check's result as a notice that stays until
  dismissed (counts and the first finding), not as a banner with a list:
  the full list is one `fanta-check` or `check_project` call away, and a
  list view is a feature of its own. The findings are kept on the document
  for that view to read.
- **T6.** The check runs after every reload from disk, which is what an
  outside change triggers. An outside change that conflicts with unsaved
  edits does not reload, so it is not checked until the conflict is
  resolved.
- **T6.** `check_project` is an MCP tool in the live and the headless
  server. The built-in agent has no tool of that name; it reads the same
  guidance and can run `fanta-check`.

- **T5.** The snapshots are taken by the two save functions (the editor's
  and `fanta-headless`'s), not inside the writer: the writer also serves
  exports and Save As staging folders, which must not gain a `.fanta`
  directory.
- **T5.** "Changed outside Fanta" is decided by git, not by the session's
  hashes: a snapshot is taken before a save when the files differ from the
  newest snapshot. Every save ends with one, so a difference is an outside
  change. A save that changes nothing records nothing.
- **T5.** Without git the save goes ahead and its report carries
  `snapshot.unavailable`. Asking before a save that would rewrite an
  outside change is not built; it belongs with the findings list in T6. The
  App Store build runs no git and takes no snapshots.
- **T5.** Restore is a native prompt listing five snapshots at a time, not
  a picker. It saves first, so unsaved edits are in a snapshot too.
- **T5.** A3 needed two more cases to open, so the collecting load of T3
  grew: a design whose `nodes.json` is gone is left out like one that does
  not parse, and an asset listed in the index with no file is a finding
  where it used to fail the open. `fanta-check` no longer reports a missing
  `nodes.json` for a legacy design, which keeps its nodes elsewhere.

- **T4.** The store is a `History` value holding the directory and the git
  binary, not free functions: the editor ships its own git and has to say
  which one to run. A snapshot's id is its time in milliseconds since the
  Unix epoch, which is also its ref name; `taken_at` is a `SystemTime`.
- **T4.** `.fanta/` ignores itself instead of being added to the project's
  `.gitignore`: the same effect without editing a file the user owns.
- **T4.** Measured on a generated project of 9,000 designs (36,000 files):
  the first snapshot takes 1.6 s, each later one about 0.1 s, listing 1 ms.
  The save-latency risk below stays open until T5 measures a real save.

- **T3.** The design said the reader gains a collecting mode. Built
  instead as a pass before the read that names the folders to skip, reusing
  the check's per-file pass, so the editor and `fanta-check` cannot disagree
  on what "does not parse" means. Cost: design files are read twice on open.
- **T3.** Protection moved into the writer and the session. Passing the
  load's findings to the save was not enough: the editor opens a new
  session after an outside change, and that session did not have them.
- **T3.** The editor shows the unavailable designs in a notice when the
  project opens. A notice after a reload, and the list of findings, come
  with T6.

- **T2.** The collecting load moved to T3. The check reports broken files
  from its own per-file pass, which gives positions the loader's error does
  not, so changing the load path was not needed to ship the checker and is
  now done only where the editor needs it.
- **T2.** The loader now reports an inline `style` attribute on a node's
  element (`edit.not_kept`). Building the corpus showed that a colour
  changed that way, which is how vector applications save paint, was dropped
  with no report.
- **T1.** Positions: the SVG reader gives offsets; the CSS reader does not,
  so CSS findings are placed by searching the file for the selector.

## Alternatives

- **A new `fanta-check` crate in the hosts layer.** Cleaner name, but a
  crate, a layer entry, a licence file and a map change for forty lines of
  argument handling. Worth doing only if ADR 0026 ends up licensing the
  checker differently from `fanta-format`.
- **Snapshots as a hidden ref in the user's repository.** Rejected by the
  founder's answer to Q1.
- **Snapshots as plain directory copies.** No git needed, but no sharing of
  unchanged files and nothing for the repair agent to diff with standard
  tools, which the founder asked for.
- **A git library (`gix`) instead of the binary.** Removes the dependency on
  an installed git at the cost of a large new dependency in the engine. The
  inherited editor already requires the binary. Revisit if the cloud worker
  image lacks git.
- **Calling Zed's `checkpoint` from the editor only.** Leaves headless and
  MCP saves without snapshots, and it writes objects into the user's
  repository.

## Risks

- **Positions.** Findings the loader raises about a node's styling point at
  the node's element in the SVG, not at the declaration in the stylesheet.
- **Collecting load reaches far.** Callers assume a loaded document is
  complete. A design left out must not be treated as deleted by the next
  save: the writer has to leave an unparsed design's files untouched (A10).
  This is the change most likely to lose data if done wrong, so it lands
  alone (T3) behind its own test.
- **No git binary.** `record` fails; the save still proceeds, and the
  failure is a `snapshot.unavailable` diagnostic shown in the editor. A save
  that would rewrite an externally changed file then asks first.
- **Save latency.** Two git invocations per save on a large project
  (9,034 designs were imported in spec 0007's live test). Measure in T4 with
  the `fanta-perf` method; if it shows, snapshot after save on a background
  task and keep only the before-save snapshot synchronous.
- **Stray `.fanta/` readers.** Any directory walker that does not skip it
  will index git objects. T4 greps for walkers and adds a test per one.
- **Licence.** `fanta-format` and `fanta-doc` are Apache-2.0 since #231
  (spec 0002 T2, ADR 0026). `fanta-format` embeds
  `web/fanta-canvas-web/src/fanta-canvas.js`, which is outside the relicensed
  crates; T8 waits for the founder's decision on that file, recorded as an
  open item in spec 0002's `design.md`.

## Verification

| Criterion | How |
| --- | --- |
| A1, A2, A9 | `cargo test -p fanta-format` over the corpus and the code list |
| A3, A10 | `fig_viewer` tests: open each corpus case, save, restore, compare bytes |
| A4 | `fanta-format` test in a temporary git repository and outside one |
| A5 | `fig_viewer` test through `queue_watcher_paths_for_test` |
| A6 | `fanta-mcp` test and the live MCP smoke test, compared with the binary's output |
| A7 | Manual run of the skill on the repairable corpus cases, recorded in the pull request |
| A8 | A workflow step running `cargo tree -p fanta-format -e normal` and failing on `gpui` or `skia` |

Release stages touched: engine, source-order, editor, workspace.
