# Tasks: Architecture map and boundaries

All in the pull request that introduced the process.

- [x] **T1.** Write `docs/architecture/layers.toml` and add `description` to
  the five crates that lacked one (R1). Verify: `script/architecture check`.
- [x] **T2.** Add `script/architecture` and `script/test-architecture`
  (R2–R8). Verify: the unit tests.
- [x] **T3.** Generate `docs/architecture/crate-map.md` (R6). Verify:
  `script/architecture map --check`.
- [x] **T4.** Add the record templates, indexes and the first records (R7,
  R8). Verify: `script/architecture index --check`.
- [x] **T5.** Add the Architecture workflow (R9).
- [ ] **T6.** Measure the workflow's duration on its first runs and record
  it here (A8).
