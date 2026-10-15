---
name: crash-investigation
description: Investigate and fix a Fanta crash, panic or hang from a macOS crash report, log or user description - symbolicate with the release's symbols, find the root cause, write a reproduction test, then make a minimal fix. Use when given a crash report, panic message, hang trace, or a "Fanta crashed when…" report.
---

# Fanta crash investigation and fix

Fanta does not upload crash reports (`"diagnostics": false`; see `docs/fanta/disabled-services-binnacle.md`). The Sentry upload in `script/bundle-mac` is inherited from Zed and targets Zed's organization; do not use it.

## 1. Gather the report

Ask the user for what they have:

- the macOS crash report from `~/Library/Logs/DiagnosticReports/` (`fanta-*.ips`), or the text of the crash dialog;
- the app log `~/Library/Logs/Fanta/Fanta.log`, where panic messages and backtraces are written;
- hang traces from `~/Library/Application Support/Fanta/hang_traces/`;
- the version (About Fanta) or release tag, and what they were doing.

## 2. Symbolicate

Release binaries are stripped. Each "Build macOS release" run uploads `fanta.dwarf` as the artifact `fanta-symbols-aarch64-<commit>`, kept for 90 days.

1. Find the release commit (`gh release view <tag> --json targetCommitish`) and the run that built it (`gh run list --workflow release-macos.yml --commit <sha>`).
2. Download the symbols: `gh run download <run-id> -n fanta-symbols-aarch64-<sha>`.
3. For each frame in the `.ips`, take the `fanta` image's load address from `usedImages` and run `atos -arch arm64 -o fanta.dwarf -l <load address> <frame address>...`.

If the artifact has expired, say so. Do not symbolicate against a rebuild and present the result as exact.

## 3. Find the root cause

Read the trace from the crash site upward:

1. The crash site: the function and line where the panic or abort happens.
2. The immediate cause: which operation failed (out-of-bounds index, `unwrap` on `None`, a slice on a non-char boundary, a GPUI double lease).
3. The in-app frames: skip crash-handler, signal, `parking_lot` and standard-library frames.
4. The data flow: how the bad value or state reached the crash site.

Read the crashing function, its callers, and how its inputs are computed. Check `git log` on those files for a recent change that introduced the bug. Then name the user action or sequence of actions that produces the bad state: the crash came from real use. Do not guess when the code can be read.

## 4. Reproduce in a test

Write a minimal failing test in the existing test module of the crate where the bug lives, following that module's patterns:

- Drive it through user-level actions (open, edit, save, type, drag) rather than constructing corrupt state directly.
- It should fail in the same function with the same kind of error as the report.
- Keep only what is needed to trigger the crash, and keep comments short.

GPUI tests: see the `gpui-test` skill. Re-run a failing seed with `SEED=<n>`.

If you can only reproduce the same class of bug, say how it differs from the report.

## 5. Fix

1. Confirm the test fails with the expected panic.
2. Identify the invariant being violated and where it breaks.
3. Fix the root cause, not the symptom. Do not just add a bounds check if the real problem is a wrong offset. Keep behavior unchanged for every non-crashing case, and keep the diff focused.
4. Add a comment only if the fix is non-obvious.
5. The reproduction test passes, the crate's tests pass (`cargo test --locked -p <crate>`), and `./script/clippy -p <crate>` is clean.

## 6. Report

Summarize the root cause, the fix and the verification commands. Look for related reports in this repository (`gh issue list --search "<panic text or function>"`) and list them for a human to confirm. Never add closing keywords to issues automatically. If the trap could recur, add it to the crate's `AGENTS.md` (see the root "Rules hygiene" section).
