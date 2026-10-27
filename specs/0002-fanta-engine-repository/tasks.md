# Tasks: The fanta-engine repository

- [x] **T0.** Preserve the never-pushed engine history now, whatever happens
  to this spec: push `feature/artifact-ir-workspace-session` (and
  `codex/migrate-fanta-engine-crates`) from `~/fanta-engine-migration` to a
  private remote. Done 2026-10-27: both pushed to
  `squidred-dev/fantaisa-engine`, and `git rev-list --count --branches --not
  --remotes` there prints 0.
- [ ] **T1.** Founder answers Q1 to Q5 and accepts ADRs 0024 and 0026.
  Verify: the records' status.
- [ ] **T2.** Fix the engine crates' licence fields in the editor (R9).
  Verify: `script/check-licenses`.
- [ ] **T3.** If Q2 says so, rename the cloud worker crate, paired with its
  deployment change in `fanta-backend`. Verify: both repositories' CI.
- [ ] **T4.** Create `fantaisa-ai/fanta-engine` (private) from the
  extraction, with its root manifest, toolchain, README, rules and layer map
  (R1, R8). Verify: A1.
- [ ] **T5.** Engine CI on Linux and macOS (R2). Verify: A2 and A8's layer
  check.
- [ ] **T6.** Editor switch pull request: pins, `script/bump-engine`, the
  committed-path rule, `layers.toml`, deletion of the in-tree crates and the
  docs (R3 to R6). Verify: A3 to A6.
- [ ] **T7.** Consumers pin the engine repository: `fanta-sdk` and the
  backend's worker image (R7). Verify: A7.
- [ ] **T8.** Copy the July and August history (`fantaisa-engine`'s
  `feature/artifact-ir-workspace-session`) into the engine repository as an
  archive branch, then the secret scan and licence check, then make the
  repository public and archive `squidred-dev/fantaisa-engine` (R1, R9).
  Verify: A8.
- [ ] **T9.** Founder marks ADR 0024 accepted and 0008 superseded, and this
  spec implemented.
