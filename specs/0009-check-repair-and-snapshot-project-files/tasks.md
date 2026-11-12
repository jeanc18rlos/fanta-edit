# Tasks: Check, repair and snapshot project files

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [ ] **T1.** Structured diagnostics: extend `SourceDiagnostic` with file,
  line and column; replace the `Vec<String>` warnings in `web_read.rs`,
  `read.rs` and `WorkspaceSession::open_warnings`; write the records to
  `report.json`; add the documented code list (R2, R4). Verify: A9, and the
  existing hand-edit tests assert codes where they asserted strings.
- [ ] **T2.** Collecting load, `check_project`, the `fanta-check` binary and
  the hostile-edit corpus (R1, R3, R9's boundary, R10). Verify: A1, A2, A8.
- [ ] **T3.** A project with errors opens and saves; an unparsed design is
  shown as unavailable and its files are left untouched by the save (R11).
  Verify: A10.
- [ ] **T4.** Snapshot store `project/history.rs`: record, list, restore,
  prune; `.fanta/` ignored by git and skipped by every directory walker
  (R6, R7 at the format layer). Verify: A4, one test per walker, save timing
  on a large project.
- [ ] **T5.** Snapshots on the save path for the editor and the headless
  engine, and the restore action in the editor (R6, R7). Verify: A3.
- [ ] **T6.** Automatic check: the editor runs it after an external change
  and shows the result; the `check_project` tool in the live and headless
  MCP servers (R5). Verify: A5, A6, and the `design_surface` sync checklist.
- [ ] **T7.** Repair skill in the plugin and the seeded project guide; the
  in-app agent's proposal shown as a diff (R8). Verify: A7, and the
  seed-upgrade test with the replaced guide's hash.
- [ ] **T8.** Publish the format's code list, the checker binary as a
  release artifact and the corpus under the licence ADR 0026 sets (R9,
  R10). Blocked until the founder accepts ADR 0026.
