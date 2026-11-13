# Tasks: The fanta-engine repository

- [x] **T0.** Preserve the never-pushed engine history now, whatever happens
  to this spec: push `feature/artifact-ir-workspace-session` (and
  `codex/migrate-fanta-engine-crates`) from `~/fanta-engine-migration` to a
  private remote. Done 2026-10-27: both pushed to
  `squidred-dev/fantaisa-engine`, and `git rev-list --count --branches --not
  --remotes` there prints 0.
- [x] **T1.** Founder answers Q1 to Q5 and decides ADRs 0024 and 0026.
  Done 2026-11-12: the answers are in `spec.md`, ADR 0026 is accepted
  (Apache-2.0), and ADR 0024 is approved; its status changes at T9.
- [ ] **T2.** Sweep the 18 engine crates and the files that move with them
  for code of Zed, GPL or other third-party origin and list what is found;
  then set the crates' `license` to `Apache-2.0` in the editor and update
  `NOTICE.md` (R9). Verify: `script/check-licenses`, and the sweep's list in
  the pull request.
  Partly done 2026-11-13: the sweep is recorded in `design.md` and 17 crates
  are Apache-2.0. `fanta-fig-interop` waits for the founder's decision on its
  golden fixtures.
- [ ] **T3.** Rename the cloud worker crate from `fanta-engine` to
  `fanta-worker` (Q2), paired with its deployment change in `fanta-backend`.
  Verify: both repositories' CI.
- [ ] **T4.** Create `fantaisa-ai/fanta-engine` (private) from the
  extraction, with its root manifest, toolchain, README, rules and layer
  map, and tag `v0.1.0` (R1, R8, Q3). Verify: A1.
- [ ] **T5.** Engine CI on Linux and macOS (R2). Verify: A2 and A8's layer
  check.
- [ ] **T6.** When no open pull request touches an engine crate: editor
  switch pull request: pins, `script/bump-engine`, the
  committed-path rule, `layers.toml`, deletion of the in-tree crates and the
  docs (R3 to R6). Verify: A3 to A6.
- [ ] **T7.** Consumers pin the engine repository: `fanta-sdk` and the
  backend's worker image (R7). Verify: A7.
- [ ] **T8.** Copy the July and August history (`fantaisa-engine`'s
  `feature/artifact-ir-workspace-session`) into the engine repository as an
  archive branch, then the secret scan, the licence check and counsel's
  confirmation (ADR 0026), then make the
  repository public and archive `squidred-dev/fantaisa-engine` (R1, R9).
  Verify: A8.
- [ ] **T9.** Founder marks ADR 0024 accepted and 0008 superseded, and this
  spec implemented.
