---
status: draft
date: 2026-10-27
owner: Jean Rojas
decisions: [0023, 0024, 0025, 0026]
repositories: [fanta-editor, fanta-engine, fanta-sdk, fanta-backend]
---

# The fanta-engine repository

## Problem

The engine can only be reached by cloning the editor and its 128 inherited
Zed crates. The cloud worker builds from the editor repository, Fanta-JS
checks out the whole editor to build the WebAssembly runtime, and the planned
SDK has nothing small to depend on. Two earlier attempts at a boundary failed
in opposite ways: a sibling path dependency nobody else could build (ADR
0006), then vendoring that tied the engine to the editor (ADR 0008). ADR 0024
proposes a third attempt; this spec says what it must achieve.

## Goals

- The engine lives in its own public repository, with its history.
- The editor, the cloud worker and the SDK all consume that repository, and
  no second copy of the engine exists anywhere.
- Developing the engine and the editor together stays practical.

## Non-goals

- Publishing to crates.io.
- Moving `fig_viewer` or any other editor crate.
- Changing the engine's public API or its crate names, except the cloud
  worker rename if Q2 says so.
- Choosing the licence. ADR 0026 decides it; this spec only blocks going
  public until it is decided.

## Requirements

- **R1.** The engine repository must contain the engine group of
  `layers.toml` with its history since 2026-07-07. The July and August
  history, the 109 commits of `squidred-dev/fantaisa-engine`'s
  `feature/artifact-ir-workspace-session` branch, must be copied there as an
  archive branch and linked from the README.
- **R2.** It must build and test from a clean clone without the editor,
  `fanta-ui` or sibling checkouts, in CI on Linux and on macOS (Metal and
  Graphite).
- **R3.** The editor must depend on engine crates by exact git revision.
  After the switch no engine crate remains in the editor, and `Cargo.lock`
  changes only in the moved packages' sources.
- **R4.** The editor's CI must stay green, including the render parity test
  and the 51 named regressions.
- **R5.** One command must bump every engine pin in the editor and refuse a
  lockfile diff beyond source lines.
- **R6.** Co-development must use a local `cargo --config` path patch, and CI
  must reject a committed path dependency outside the repository.
- **R7.** The cloud worker image and the runtime and SDK builds must pin the
  engine repository instead of the editor.
- **R8.** The engine repository must run the layer check on its own crates.
- **R9.** Before the repository becomes public, the licence must be decided,
  the SPDX identifiers fixed, and a secret scan of its history must be clean.

## Acceptance criteria

- **A1 (R1).** For each engine crate, `git log --oneline -- <path> | wc -l`
  in the engine repository equals the editor's count at the extraction commit.
- **A2 (R2).** Engine CI is green on Linux and macOS from a clean clone, and
  `cargo tree` there lists no `fanta-gpui` package and no Zed crate.
- **A3 (R3).** The editor pull request that switches to the pin deletes the
  engine directories, passes `cargo metadata --locked`, and its `Cargo.lock`
  diff touches only the moved packages' sources.
- **A4 (R4).** That pull request's CI is green.
- **A5 (R5).** The bump command updates every entry and is idempotent.
- **A6 (R6).** A committed `path = "../…"` dependency fails CI in both
  repositories.
- **A7 (R7).** `fanta-sdk` builds its WebAssembly from the engine pin, and the
  Cloud Run image builds from the engine repository.
- **A8 (R8, R9).** `script/architecture verify` passes in the engine
  repository, the licence fields match ADR 0026, and the secret scan reports
  nothing.

## Open questions

- **Q1.** Which licence (ADR 0026)?
- **Q2.** Rename the cloud worker crate from `fanta-engine` to free the name?
  Recommended: `fanta-worker`.
- **Q3.** Tag versions (`v0.x.y`) from the first day, or pin revisions until
  an outside consumer exists? Recommended: tags from the first day, so
  consumers and the changelog speak the same language.
- **Q4.** Should the cloud contract (`docs/cloud-engine-contracts.md`) move to
  the engine repository, with the backend keeping its copy? Recommended: yes,
  since the engine owns the wire types.
- **Q5.** Keep the vendored `skia-bindings` in the engine repository, or give
  it its own? Recommended: keep it; it changes only with Skia upgrades.

## Changes after acceptance

- None.
