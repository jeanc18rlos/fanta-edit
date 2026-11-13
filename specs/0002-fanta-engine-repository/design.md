# Design: The fanta-engine repository

## Approach

### What moves

The engine group in [`layers.toml`](../../docs/architecture/layers.toml):
18 crates on 2026-11-12. `fanta-doc`, `fanta-design-ops`, `fanta-canvas`,
`fanta-tools`, `fanta-fnx`, `fanta-format`, `fanta-fig-interop`,
`fanta-text`, `fanta-render`, `fanta-present`, `fanta-headless`,
`fanta-api`, `fanta-worker` (`fanta-engine` until T3, #229), `fanta-runtime`
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
3. Rename the worker crate to `fanta-worker`. Done in #229. The binary, the
   image and the Cloud Run service keep the name `fanta-engine`, so the
   backend's deployment did not change.
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

## Provenance sweep (2026-11-13, T2)

Result: nothing in the engine traces to Zed or to other GPL code. Seventeen
crates are now `Apache-2.0`. `fanta-fig-interop` stays GPL-3.0 because of its
golden fixtures. The review of #231 added two more open licensing questions;
all three are for the founder and are listed under "Open licensing
questions" below. This is an engineering check, not legal advice; counsel
confirms before the repository is public (ADR 0026).

Method, on `main` at `3e4d7b5e`:

- **History.** 867 tracked files: the 18 crates (780), the vendored Skia
  binding (60) and the 27 files that move with them or that T4 decides on.
  65 commits added them. Each was diffed against its whole parent tree with
  `git show -M50% -C50% --find-copies-harder`: 83 copies or renames came from
  outside the engine paths (43 Skia binding files renamed from 0.84.0, 18
  `LICENSE-GPL` links, 8 `OFL.txt`, 9 files from `fig_viewer`, 5 from
  `design_surface`). The 14 files from `fig_viewer` and
  `design_surface` were first added between 2026-07-10 and 2026-10-15, after
  the Zed baseline `8b08b060`, with no copy source of their own except
  `design_surface/Cargo.toml`, a manifest modelled on `web_search`'s. Every commit
  in the repository has one author.
- **Content.** The 807 non-vendored files were searched for licence headers,
  SPDX identifiers, copyright lines, attribution phrases, project names and
  URLs. No file carries a licence header or names Zed.
- **Similarity.** Windows of 8 normalised lines from the 399 engine Rust
  files (167,437 windows) were looked up in 1,004 non-engine Rust files of
  this checkout, the 1,806 of the Zed baseline and the 1,046 of the pinned
  `fanta-ui`. 16 file pairs matched, 15 in `fig_viewer` and one in
  `agent/src/tools/design_screenshot_tool.rs`, all Fanta's files from after
  the baseline. None matched the baseline or `fanta-ui`.
- **Dependencies.** `cargo metadata --locked`: no engine crate reaches a
  workspace crate outside the group, dev-dependencies included. The normal
  and build closure of the 18 crates, over every target, holds 329
  third-party packages (328 from crates.io and the vendored `skia-bindings`;
  326 and `skia-bindings` for the 17 relicensed crates), and dev-dependencies
  add 14. No licence expression offers only GPL, LGPL or AGPL terms. Most
  are MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or CC0 expressions; the
  table below lists every expression with a term outside those families,
  recomputed on this branch after the merge with `main` at `7c16e095`.

| Licence expression | Packages | Note |
| --- | --- | --- |
| `MPL-2.0` | `option-ext` 0.2.0 | `fanta-text`, through `dirs`. For counsel's list |
| `CDLA-Permissive-2.0` | `webpki-roots` 1.0.8 | `fanta-text`, through `ureq`. A data licence for the root certificate list. For counsel's list |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `r-efi` 5.3.0 | Through `getrandom`, on UEFI targets only. The LGPL term is one of three alternatives |
| `(MIT OR Apache-2.0) AND NCSA` | `libfuzzer-sys` 0.4.10 | Through `rav1e`, under `cfg(fuzzing)` only |
| `ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)` | `aws-lc-sys` 0.40.0 | `MIT-0` appears as an alternative |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `linux-raw-sys` 0.11.0, `rustix` 1.1.2, `wasi` 0.11.1, `wasip2` 1.0.1, `wit-bindgen` 0.46.0 | |
| `Unlicense OR MIT` | `aho-corasick` 1.1.3, `byteorder` 1.5.0, `byteorder-lite` 0.1.0, `memchr` 2.7.6 | |
| `CC0-1.0 OR MIT-0 OR Apache-2.0` | `constant_time_eq` 0.3.1, `dunce` 1.0.5 | |
| `0BSD OR MIT OR Apache-2.0` | `adler2` 2.0.1 | |
| `Apache-2.0 OR BSL-1.0` | `ryu` 1.0.23 | |

Findings:

| Path | Origin | Licence | Verdict |
| --- | --- | --- | --- |
| `crates/fanta-fig-interop/tests/golden/op_ref_*.json` (4 files) | Layer names, bounds, colours and text of Adobe's Spectrum Figma community file, distilled with OpenPencil | Not recorded | Open. The founder confirms the file's licence and adds its attribution, or removes the fixtures (only the opt-in legacy report reads them). The crate stays GPL-3.0 until then |
| `crates/fanta-fig-interop/src/kiwi` | Rust port of the Kiwi wire format (evanw/kiwi) | MIT | Compatible; notice added to `NOTICE.md` |
| `crates/fanta-fig-interop/src/mapping`, `crates/fanta-tools/src/select/reparent.rs` and its call site in `select/mv.rs`, comments in `fanta-doc` and `fanta-render` | Ported from or mirroring "OpenPencil", which the comments label "op1" and "op2" without a URL. The comment in `mv.rs` names the drag-to-reparent source as "op1: vue/src/shared/input/drop-target.ts, core/src/editor/structure/reorder.ts". The sweep read "op1" as open-pencil/open-pencil and "op2" as ZSeven-W/openpencil; which repository each label means cannot be determined from this repository | MIT, both candidates | Compatible if the reading holds; both notices added. For counsel's list |
| `crates/fanta-tools/src/ink.rs`, `fixtures/ink` | Reimplementation of perfect-freehand's algorithm; the fixtures are its output. The generator `oracle/gen-ink.mjs` the header names is not in the tree | MIT | Compatible; notice added |
| `crates/fanta-text/assets` (14 fonts) | Inter, Source Sans 3, Source Serif 4, Source Code Pro | OFL-1.1 | Compatible to bundle; the fonts keep OFL-1.1 and their `OFL.txt` |
| `vendor/skia-bindings-0.153.3` | crates.io `skia-bindings` 0.153.3; the archive's SHA-256 and the 55 unchanged files match `FANTA-PROVENANCE.txt`, and `LICENSE` matches upstream's | MIT | Stays MIT, as `script/check-licenses` enforces |
| `option-ext` 0.2.0, `webpki-roots` 1.0.8 | crates.io dependencies of `fanta-text` | MPL-2.0; CDLA-Permissive-2.0 | Unmodified dependencies; for counsel's list. The dependency table above has the other expressions |
| `fanta-tools` (`select.rs`, `event.rs`, `tool.rs`), `fanta-canvas/src/snap.rs`, `fanta-doc/src/index.rs` | Comments say the design mirrors or is inspired by tldraw, Figma and Sketch | tldraw's licence is not open source | No port is claimed and none was found, but the code was not compared with tldraw's source; for counsel's list |
| Files moved from `fig_viewer` and `design_surface` into `fanta-headless` and `fanta-design-ops` | Fanta's editor code | Was GPL-3.0-or-later, Fanta's own | Relicensed by its owner |

### Open licensing questions

For the founder. No licence was changed for either.

**A. `fanta-format` embeds a file outside the relicensed crates.**
`crates/fanta-format/src/project/web.rs` (line 69) compiles
`web/fanta-canvas-web/src/fanta-canvas.js` into the crate with
`include_str!`, and `write_web_project_tree`, which `write_project_tree`
calls, writes it as `canvas.js` into every saved project. The file (15,969
bytes, added on 2026-11-12 in `bd466534` for spec 0008 T2) has no licence
header, `web/` has no licence file or package manifest, and `NOTICE.md`'s
Apache-2.0 scope names crates only, so the file is under the repository
default, GPL-3.0-or-later. The other three embedded web files (`index.html`,
`viewer.js`, `editor.js`) are inside the crate. The options:

- add `web/fanta-canvas-web` to the Apache-2.0 scope in `NOTICE.md`; or
- hold `fanta-format` back as `fanta-fig-interop` is, until the file is
  settled.

**B. Spectrum-derived material outside the four golden files.** A
case-insensitive search for "Spectrum" in the 17 relicensed crates finds 15
lines in 12 files:

| File | What it is |
| --- | --- |
| `crates/fanta-fnx/src/tests.rs` (2364), `small_numbers_preserve_all_recorded_spectrum_field_kinds` | A hand-built test structure of three nodes with placeholder ids. The layer names "Color Slider", "Vector" and "GlobeGrid" and ten near-zero numbers (for example `-6.123234262925839e-17`) are, by the test's name, recorded from the Spectrum file; the other values are placeholders |
| `crates/fanta-doc/src/boolean.rs` (342) | One coordinate, `0.22952167611888466`, which the comment calls "this imported Spectrum coordinate", in a test |
| `crates/fanta-text/src/layout/engine.rs` (805) | A test with two heading texts, "Workflow Icons" and "Scroll-Zoom Bar", and their box widths, 342 and 341, which the comment says Spectrum's frames have in Figma |
| `crates/fanta-text/src/font_resolver/substitute.rs` (15, 69, 96, 104) | Comments, and two width ratios (0.95 and 0.925) "tuned against the Spectrum `.fig`". The comments quote the label "224 selected" and the frame name "Item Counter" |
| `crates/fanta-text/src/font_resolver/bundled.rs` (34), `download.rs` (72) | Comments that name the file as the reason for the weights and font families chosen. No data |
| `crates/fanta-doc/src/spatial.rs` (5), `scene/tests.rs` (419), `resolve/instance.rs` (660), `crates/fanta-render/src/raster/vector.rs` (1196) | Comments that cite the file's size (44k nodes, 13k rounded rectangles) or a property of it ("every Spectrum icon is one vector"). The scene test is synthetic. No data |
| `crates/fanta-format/src/project.rs` (152), `project/session/tests.rs` (5245) | Names of two ignored diagnostic tests that read a file from an environment variable. No data |

In `fanta-fig-interop`, which is not relicensed, 29 files besides the four
golden files mention Spectrum (57 lines): 25 under `src` and 4 under
`tests`. The search finds only lines that say "Spectrum"; values taken from
the file without that word nearby are not found by it.

Not checked: the engine's history before 2026-07-07, which lives in
`squidred-dev/fantaisa-engine` (private, one contributor, no licence file);
similarity against sources outside this repository (current Zed, OpenPencil,
tldraw, Skia); JavaScript, CSS and C++ similarity; whether the fonts are
byte-identical to their upstream releases.

The root `LICENSE-APACHE` opens with Zed Industries' copyright line, so the
crates' links to it are for the licence text only; T4 gives the engine
repository its own `LICENSE`.

## Verification

Each acceptance criterion maps to a task's check in `tasks.md`.
