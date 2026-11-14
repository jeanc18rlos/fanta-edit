# Tasks: Measure generated code against designs

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges.

- [ ] **T1.** `web/fanta-compare` with `compare` and the pixel score: the
  engine request, the pinned browser, fonts for both sides, blocked network,
  JSON output, `--threshold` and difference images; the workflow that runs
  the package's tests (R1, R2, R5; mapping by `id` only). Verify: A1, which
  sets the tolerance, and A4. Also reports whether an engine run reaches the
  network for fonts and whether a component master renders.
- [ ] **T2.** Style score, token reuse and explicit mapping:
  `data-fanta-id`, the map file, the HTML and SVG property table, unmapped
  and unread nodes in the output (R3, R4). Verify: A2, A3.
- [ ] **T3.** CI example: `examples/compare-ci/` with a workflow that runs
  `compare` and fails on drift, and the page in `docs/fanta/` that explains
  it (R11). Verify: A7.
- [ ] **T4.** The design set: `fanta-engine import-fig`, `extract`,
  `sources.json` with hashes, the 24 screens as committed projects, the
  attribution file, the size budget and `split.json` (R7, R13). Needs
  `FANTA_BENCHMARK_SOURCES`. Verify: A9, the mappable share per screen in
  the pull request, `script/architecture map` for the new dependency.
- [ ] **T5.** JSON baseline: the converter into Pencil and the recorded
  `batch_get` output per screen with its conversion score, or the neutral
  node tree if fewer than 20 screens convert (R6's second condition, Q4).
  Verify: a test that every design has a baseline file; the conversion
  scores in the pull request.
- [ ] **T6.** `benchmark`: the four conditions, the runner command, the
  React scaffold, geometric mapping, three runs, the result file and the R9
  verdict with the worst properties (R6, R8, R9 as code, R10's list).
  Verify: A5 and A6 with the stub runner; one real run on the tuning set
  attached to the pull request.
- [ ] **T7.** The reported run: held-out set, React, the models the founder
  names; commits the result file and states the verdict (R9, R10). Publishing
  the set waits for spec 0009 T8. Verify: A6 and the React half of A8 on the
  committed file.
- [ ] **T8.** Expo target: a React Native scaffold scored through Expo's web
  build, labelled as such in the result (R12). Verify: A8.
- [ ] **T9.** Bevy UI target, pixel score only (R12). Verify: one design
  scored from a Bevy screenshot; no style section in its result.
