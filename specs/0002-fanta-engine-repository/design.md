# Design: The fanta-engine repository

## Approach

### What moves

The engine group in [`layers.toml`](../../docs/architecture/layers.toml):
18 crates on 2026-11-12. `fanta-doc`, `fanta-design-ops`, `fanta-canvas`,
`fanta-tools`, `fanta-fnx`, `fanta-format`, `fanta-fig-interop`,
`fanta-text`, `fanta-render`, `fanta-present`, `fanta-headless`,
`fanta-api`, `fanta-engine` (renamed `fanta-worker` by T3), `fanta-runtime`
and `fanta-site` were in the dry run below. `fanta-bundle` (#158),
`fanta-mcp` and `fanta-wasm-engine` were added after it. The extraction
takes the group as `layers.toml` lists it on the day, not this list. With
them go
`vendor/skia-bindings-0.153.3`, `.github/actions/prepare-skia-source`,
`script/prepare-skia-source`, `fixtures/ink` (used by `fanta-tools`),
`crates/VENDORED.md`, and the cloud contract, project schema and parity docs.

`fig_viewer`, `design_surface`, `fanta_ui`, `fanta_languages`, `fanta` and the
shell stay in the editor.

### Extraction

The dry run of 2026-10-27, from a local clone:

```bash
git clone --no-local --single-branch --branch main <editor> engine
```

```bash
git -C engine filter-repo --path crates/fanta-doc --path crates/fanta-fnx --path crates/fanta-design-ops --path crates/fanta-canvas --path crates/fanta-tools --path crates/fanta-fig-interop --path crates/fanta-format --path crates/fanta-text --path crates/fanta-render --path crates/fanta-present --path crates/fanta-headless --path crates/fanta-api --path crates/fanta-engine --path crates/fanta-runtime --path crates/fanta-site --path vendor/skia-bindings-0.153.3 --path fixtures/ink --path .github/actions/prepare-skia-source --path script/prepare-skia-source --path crates/VENDORED.md --path docs/cloud-engine-contracts.md --path docs/fanta/engine-parity.md --path docs/fanta/project-schema.md
```

It kept 274 of 800 commits (5.9 MB against 46 MB) in under a second. A root
manifest generated from the editor's (members, `[workspace.package]`, the 39
workspace dependencies the engine uses, `[workspace.lints]`, the
`skia-bindings` patch, and `exclude` for the vendored binding, as the editor
does) resolved 320 packages with no GPUI-family or Zed crate. The seven crates
without Skia passed `cargo check` in 10.8 seconds.

The real extraction adds `--path crates/fanta-bundle --path crates/fanta-mcp
--path crates/fanta-wasm-engine`, and names the worker `crates/fanta-worker`
with `--path crates/fanta-engine` kept so its history before the rename
comes along. T4 repeats the checks on the 18 crates: the numbers above are
the 15-crate dry run's. `fanta-wasm-engine` builds for
`wasm32-unknown-unknown` only, so engine CI checks it for that target, as
the editor's `lint` stage does today. T4 also lists the files outside
`crates/` that only the three newer crates use (`script/smoke-mcp`,
`script/serve-project`, `web/fanta-canvas-web`) and says for each whether it
moves; that is not decided here.

The real extraction repeats this, then commits on top: the root
`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, the profiles the engine
needs (the Skia opt-level settings from the editor's `[profile.*.package]`),
a README, `AGENTS.md` with the engine crates' rules, its own `layers.toml`
and a copy of `script/architecture`. The first tag is `v0.1.0` (Q3).

### How the editor consumes it

```toml
[workspace.dependencies]
fanta-doc = { git = "https://github.com/fantaisa-ai/fanta-engine", rev = "<sha>" }
# … one entry per engine crate the editor uses …

[patch.crates-io]
skia-bindings = { git = "https://github.com/fantaisa-ai/fanta-engine", rev = "<sha>" }
```

Today the editor uses eleven engine crates directly: ten from `fig_viewer`,
`fanta-design-ops` from `design_surface`, and `fanta-format`, which
`workspace` shares with `fig_viewer`. Profiles apply only from the root
workspace, so the editor keeps its Skia opt-level settings. In the editor's
`layers.toml` the engine becomes an external family, `fanta-engine`, that
the desktop layer may use and the shell may reach only through the existing
`workspace` seam.

`script/bump-engine <sha>` rewrites every entry, runs `cargo metadata`, and
fails if the `Cargo.lock` diff contains anything but `source` lines.

### Developing both together

A scratch file passed with `cargo --config <file>` patches the engine crates
to a local checkout by path. It is never committed. `script/architecture
check` gains a rule: a committed path dependency must point inside the
repository.

### CI

The engine repository runs the `engine`, `import` and `source-order` stages
(moved out of `script/verify-fanta-release` into a `script/verify-engine`),
the runtime build from `fanta-runtime.yml`, the Skia source compatibility
check and `script/architecture verify`, on Linux and macOS. It starts with
the shape the editor's CI is moving to (plan of 2026-11-12): one
compile-free gate command, tests on Linux first, one macOS job for what
only Metal and Graphite can witness, and nightly fuzzing of the parsers.
The editor keeps
`editor`, `workspace`, `ui`, `build` and `lint`, plus the render parity test
`headless_engine_renders_match_editor_screenshots`, which now guards the pin.

### Order

1. Founder answers Q1 to Q5. Done 2026-11-12.
2. Sweep the engine crates for code of Zed, GPL or other third-party
   origin, then set their licence fields to `Apache-2.0` in the editor, so
   the extracted history starts right.
3. Rename the worker crate to `fanta-worker`, with the backend's deployment
   change.
4. Create `fantaisa-ai/fanta-engine` as private, push the extraction and
   tag `v0.1.0`.
5. Engine CI green.
6. The editor switch pull request, when no open pull request touches an
   engine crate.
7. Consumers switch: `fanta-sdk` (`fetch-engine` and its pin file) and the
   backend's worker image.
8. Copy the July and August history (`fantaisa-engine`'s
   `feature/artifact-ir-workspace-session`) into the new repository as an
   archive branch, run the secret scan, have counsel confirm the licence
   (ADR 0026), then make the repository public;
   archive `fantaisa-engine` and delete the empty `squidred-dev/fanta-engine`
   placeholder.

## Interfaces and data

No API changes. The engine's public surface is what the editor and the hosts
use today; the switch pull request must compile against it unchanged.

## Alternatives

ADR 0024 compares vendoring, sibling paths, a one-way mirror and crates.io.

## Risks

- **A cross-cutting change gets slower.** Two merges and a pin bump instead
  of one pull request. Mitigation: the bump script, and batching engine
  changes.
- **The switch breaks the editor build.** The switch pull request is
  reversible: revert it and the in-tree crates come back from history.
- **A pinned commit becomes unreachable.** Pin merge commits on `main`
  only, never branch heads.
- **Build settings drift between the two workspaces.** The engine's CI
  builds with its own settings; the editor's CI with its own. Differences show
  up as test differences, which the parity test catches.

- **Relicensed code that is not Fanta's.** Apache-2.0 on a file copied
  from Zed or another GPL source would be a licence violation. T2's sweep
  comes before the field change, and counsel confirms before the repository
  is public.

## Verification

Each acceptance criterion maps to a task's check in `tasks.md`.
