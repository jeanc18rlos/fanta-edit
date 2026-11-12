# Design: Measure generated code against designs

Written 2026-11-12 against `main` at `9716f162`. Nothing here is built. The
founder reviews this file and [tasks.md](tasks.md) before any task starts.

## Approach

**Where it lives.** A Node package, `web/fanta-compare/`, beside
`web/fanta-canvas-web/` (Q5). It is the first package under `web/` with a
dependency: Playwright, pinned to an exact version, which also pins the
Chromium build (R5). The package is outside the Cargo workspace, so
`layers.toml` and the crate map do not change. The engine crates gain no
browser dependency.

**The engine half.** `compare` calls the `fanta-engine` binary as it exists:
`fanta-engine run --request <file>` with a `dir` source
(`crates/fanta-engine/src/main.rs`). The run copies the project to a
temporary directory and never writes to the source. Two steps are used:

- `render` (`RenderTarget` in `fanta-headless`): a PNG of a page or of one
  node, returned as `png_b64` with `width`, `height` and the world `bounds`
  it shows.
- `project` (`scene::page_scene`): the style projection agents read. Per
  node it has the prefixed id, kind, name, world bounds, fills, text style
  (family, size, weight, line height), auto layout (gap, padding, sizing
  mode), corner radius and `bindings`.

Three facts about these steps shape the design:

- `render` has no scale parameter. `render_screenshot` computes
  `zoom = min(max_dimension / longer side, 2)` and `max_dimension` is capped
  at 1568 (`MAX_RENDER_DIMENSION`). The script reads each node's size from
  the design's `map.json` and asks for `max_dimension` equal to its longer
  side, which gives a zoom of 1 for nodes up to 1568 units. The scale
  actually used is read back from the response (`width / bounds[2]`) and the
  browser screenshot is taken at that device scale factor. A node longer
  than 1568 units is compared at the reduced scale and the output says so.
- `project` takes a page, lists at most 200 children per level
  (`DEFAULT_CHILD_LIMIT`) and reduces its depth to fit a traversal budget. A
  node that `map.json` names and the listing lacks is reported as `unread`
  and counted, never skipped.
- `PageRef` accepts an index or a prefixed id, not a name. The script
  resolves a name (R1) through the `inspect` step.

**Fonts (R2).** The engine resolves fonts through `fanta-text`, which
registers every font in the directories named by `FANTA_FONT_DIRS` and
reports families it lacks in the response (`source.fonts.missing`). The
script points `FANTA_FONT_DIRS` at the project's `assets/fonts/` and gives
the browser the same files as `@font-face` rules. A missing family is in the
output, and fails the run unless `--allow-missing-fonts` is passed.

**The browser half.** One Chromium page per target, with a fixed viewport
width taken from the design root, animations and transitions disabled by an
injected stylesheet, and every request outside the target's own origin or
directory aborted and listed (R5).

**Mapping (R4, Q6).** Emitted SVG carries the node id on every element the
writer owns: `start_element` in `crates/fanta-format/src/project/web.rs`
writes `id="{node.id}"` through `Display`, the prefixed form. The corpus
shows it:
`crates/fanta-format/tests/corpus/deleted-component/pages/components/page.svg`
has `<g id="n_01M78J2NED7QX15H9HKM844EQ8" class="components" …>`, the
stylesheet beside it selects `#n_…`, and `map.json` keys the same form.
`nodes.json` keys the bare ULID, the persisted form. The scorer uses the
prefixed form everywhere and never reads `nodes.json` (the two-spellings
trap in the root rules).

A target element maps to a design node in one of three ways, tried in this
order:

1. `data-fanta-id="n_…"` on the element. An `id` is unique per document and
   React code repeats components, so generated code cannot rely on `id`.
2. `id="n_…"`, which is what Fanta's own files have (A1).
3. A map file, `{"n_…": "<css selector>"}`, passed with `--map`.

Two limits found in the emitter:

- Content inside a component instance has no element id: expansion ids are
  not stable across saves, so the writer emits it inline-styled
  (`emit_virtual`). An instance is one mapped node; what is inside it is
  compared by pixels only.
- A node drawn as a raster fallback is an `<image>` with the node's id. It
  is mapped and compared by pixels; it has no styles to compare.

**Pixel score (R2).** For each mapped node: the engine's render of that node
and the browser's screenshot of the mapped element, at the same scale. The
two PNGs are compared inside the same pinned browser (canvas pixel data), so
the package needs no image library. A pixel matches when no channel differs
by more than the tolerance. The node's score is the share of matching
pixels; a size difference counts the uncovered area as not matching. A node
below the pass share gets a difference image under `--out`. The channel
tolerance and the pass share are set by the A1 run in T1 and recorded in the
package's `README.md`.

**Style score (R3).** Design values come from the `project` step. Target
values come from `getComputedStyle` and `getBoundingClientRect`. Which
target property answers which design value depends on the element:

| Design value | HTML element | SVG element |
| --- | --- | --- |
| fill of a shape or frame | `background-color` | `fill` |
| text colour | `color` | `fill` |
| font family, size, weight, line height | the same properties | the same; no line height |
| padding, gap | `padding-*`, `gap` | `--fanta-padding`, `--fanta-gap` |
| corner radius | `border-radius` | `rx` |
| size, when sizing is fixed | bounding box | bounding box |
| position inside the compared root | bounding box | bounding box |

The SVG column exists because A1 and A2 score the page Fanta's writer
emits, which is SVG and has no box model. Position is not in R3's list: A2
asks that an 8 px shift names the node and the property, and the pixel score
alone names only the node.

**Token reuse (R3).** The writer emits a bound property as
`var(--collection-variable, literal)` in the design's stylesheet
(`Declarations::push_bound`). The script reads those declarations through
the browser's CSSOM, which gives the custom property name for each bound
node and property. To learn whether the target expresses the value through
the property, the script overrides the custom property on the target's root
with a sentinel value and reads the computed style again: if it changed, the
code uses the token. This works whatever produced the CSS. A token whose
value in the target differs from the design's `tokens.css` is a finding of
its own, with the token named (A7).

**Benchmark (R6–R10).** `benchmark` runs in the same package.

- *Design set.* 24 screens, four from each of the six source files, each
  extracted to its own small Fanta project and committed under
  `web/fanta-compare/benchmark/designs/`. The split is one screen per source
  file for tuning (6) and the rest held out (18), written to `split.json` in
  the pull request that adds the set, before any run exists (R7).
- *Conditions.* The model receives one of: the engine's render of the
  screen as a PNG; the Pencil JSON (below); the screen's project directory;
  the project directory with `fanta-mcp` attached. The task text and the
  React scaffold are identical.
- *Runner.* A command template in the run configuration starts one agent
  process per design, condition and run, in an empty working copy of the
  scaffold. The first runner is `claude -p` in non-interactive mode, because
  one runner can then attach an image, files or an MCP server without the
  harness changing between conditions. The result records the runner's
  version and the model id.
- *Target.* A React scaffold with a locked dependency set
  (`web/fanta-compare/targets/react/`). The model writes under `src/` only.
  The harness builds it and scores the built page from disk.
- *Mapping in the benchmark.* A screenshot holds no node ids and Pencil's
  ids are its own, so two of the four conditions cannot emit Fanta ids. To
  keep the conditions comparable, the benchmark does not use ids in any
  condition: for each design node it picks the target element whose box
  overlaps the node's design box most (at least half, by intersection over
  union), writes that as the map file of option 3, and scores with it.
  Design nodes with no such element are unmapped. The share of
  `data-fanta-id` attributes the Fanta conditions emit correctly is reported
  beside the scores and does not enter them.
- *Verdict (R9).* Each score is turned into an error, `1 − score`, averaged
  over the held-out screens and the three runs. The hypothesis holds when
  the Fanta-files error is at most 0.8 times the better baseline's error, on
  the pixel score and on the style score.
- *Result (R8, R10).* One JSON file per run under `benchmark/results/`, with
  the per-property error totals that R10 needs when the verdict is no.

**Pencil baseline (Q4).** Tried on 2026-11-12 on Linux:

- The Pencil MCP tools of the desktop app (`get_editor_state`,
  `open_document("new")`, `get_guidelines`) all failed with "failed to
  connect to running Pencil app: desktop": the app was not running.
- The same tools through Pencil's CLI (`@pencil.dev/cli` 0.2.5, headless
  mode, `pencil interactive -o <new file>`) worked on a new document in a
  temporary directory. No existing document was opened.
- There is no import. The tool reference (`pencil interactive --help`) and
  the `.pen` schema have no tool or field that reads `.fig` or SVG. The
  schema's `imports` are other `.pen` files; images are fills that point at
  a relative file.
- Writing and reading back works. A frame with layout, padding, gap and
  corner radius, a text node, a path with SVG path data, an image fill and a
  colour variable bound as `$surface` were inserted with `batch_design` and
  `set_variables`; `batch_get` and `get_variables` returned all of them.
- `save()` reported success and left a 0-byte file, in both attempts.
  Whether that is the sandbox or the CLI is not known.

So the designs can reach Pencil only through a converter we write: Fanta's
`project` output to `batch_design` operations, 25 per call. The baseline is
then "Pencil's JSON of our translation of the design", and whatever the
converter loses counts against Pencil. Proposed, for the founder to confirm:

- Use it, with a check. For each screen the converter writes into Pencil,
  Pencil's own render (`export_nodes`) is scored against the engine's render
  with the pixel score. A screen that converts below the pass share is named
  in the result.
- The `batch_get` output is recorded once per screen and committed, so a
  benchmark run needs neither Pencil nor an account.
- If fewer than 20 of the 24 screens convert, or the founder prefers, the
  fallback of Q4 applies: the neutral node tree is the `project` step's JSON
  with ids and names kept, which the model gets as a file.

**Large fixtures.** The six `.fig` files (241,977,744 bytes) are never
committed. The repository's precedent for a file it cannot hold is
`FANTA_FIG_FIXTURE`: an environment variable names the path, and the tests
that need it print a line and pass when it is unset
(`crates/fanta-fig-interop/tests/common/mod.rs`,
`script/verify-fanta-release` stage `import-preservation`). The benchmark
follows it:

- `web/fanta-compare/benchmark/sources.json` is committed: for each file
  its name, title, author, licence, Community URL, size and SHA-256.
- `FANTA_BENCHMARK_SOURCES` names the directory that holds the files. Only
  the extraction command reads it. It checks each file's hash and refuses
  one that differs.
- When the variable is unset, extraction exits with a message that names
  the variable and the URLs. Nothing else needs the sources: `compare`, the
  benchmark and every test run on the committed extracted screens.
- The extracted set has a budget, checked by a test: 30 MB in all and 2 MB
  per file. A screen that does not fit is not chosen.
- A second copy of the sources is uploaded as release assets where R10
  publishes the set. Until then the Community pages and the founder's copy
  are the only ones; if a page changes, the hash check says so and the
  committed screens still run.

Extraction needs `.fig` import without the editor. Today that is only
`crates/fig_viewer/examples/fig_to_project.rs`, which writes without a
renderer, so nodes outside the SVG subset draw nothing. T4 adds a local
`import-fig` subcommand to the `fanta-engine` binary (not a step of the
wire contract) that imports, keeps the named frames and saves through
`fanta-headless`. `fanta-engine` then depends on `fanta-fig-interop`, which
the layer map allows (hosts reach persistence through application).

## Interfaces and data

```text
fanta-compare compare <project-dir> --design <page or component name | n_… id>
    --target <url | file.html> [--selector <css>] [--map <file.json>]
    [--threshold <0..1>] [--out <dir>] [--engine <path>] [--allow-missing-fonts]
fanta-compare benchmark --config <run.json> [--set tuning|held-out] --out <result.json>
fanta-compare extract --screens <screens.json>      # needs FANTA_BENCHMARK_SOURCES
```

`--engine` defaults to `$FANTA_ENGINE`, then `target/release/fanta-engine`.
Exit codes: 0 pass, 1 below the threshold, 2 could not run.

```json
{
  "design": {"project": "…", "root": "n_…", "name": "Home"},
  "target": "dist/index.html",
  "browser": "chromium 147.0.7727.15",
  "engine": "<EngineInfo.version>",
  "scale": 1,
  "tolerance": {"channel": 0, "pass_share": 0},
  "score": {"pixel": 0.0, "style": 0.0, "token_reuse": 0.0},
  "nodes": [
    {"id": "n_…", "name": "Card", "pixel": 0.0, "diff": "diff/n_….png",
     "failures": [{"property": "fill", "design": "#FFFFFF", "target": "#FAFAFA"}]}
  ],
  "tokens": [{"token": "--theme-surface", "design": "#FFFFFF", "target": "#FAFAFA"}],
  "unmapped": ["n_…"],
  "unread": [],
  "fonts": {"missing": []},
  "blocked_requests": []
}
```

Keys and arrays are sorted and the output holds no time or absolute path
(A4). `score.pixel` and `score.style` are means over mapped nodes;
`token_reuse` is the share of bound design properties the target expresses
through the custom property. `--threshold` applies to the lower of the pixel
and style scores.

Benchmark files, all under `web/fanta-compare/benchmark/`:

- `sources.json`, `ATTRIBUTION.md`: R13. A test fails when a design names a
  source without an attribution entry (A9).
- `designs/<source>-<screen>/`: one Fanta project per screen.
- `split.json`: `{"tuning": […], "held_out": […]}`.
- `pencil/<design>.json`: the recorded `batch_get` output and the map from
  Pencil ids to Fanta ids.
- `prompts/`: the task text and one input section per condition.
- `results/<date>-<model>.json`: model id, runner and version, date, prompt
  hashes, split, per design, condition and run both scores and the unmapped
  count, then the R9 verdict and, when it fails, the worst properties.

## Alternatives

- **A Rust `compare` in the engine.** Needs a browser driver in the engine
  crates. The founder's answer to Q5 rules it out.
- **A `scale` field on `RenderTarget`.** Cleaner than deriving the scale
  from `max_dimension`, and removes the 1568 limit. It changes the cloud
  wire contract (`deny_unknown_fields` on the request), so it is left out
  until the limit is shown to cost accuracy.
- **Design values from the emitted CSS instead of the `project` step.** One
  source fewer, but the emitted CSS has no line height, and the layout
  values are custom properties in the writer's own spelling.
- **An image library (`pixelmatch`, `pngjs`).** Two more pinned
  dependencies for something the pinned browser already does.
- **Ids as the benchmark's mapping.** Fair to no baseline: the screenshot
  condition would have every node unmapped. Reported beside the scores
  instead.
- **Calling model APIs directly.** The conditions with files and with MCP
  tools need an agent loop; writing one for the benchmark would measure our
  loop. A runner command keeps the loop someone else's and the same for all
  four conditions.
- **Git LFS or a submodule for the `.fig` files.** The repository uses
  neither, and every clone and CI checkout would pay for files one command
  reads.
- **The neutral node tree as the only JSON baseline.** Cheaper and needs no
  third-party tool, but it is Fanta's own projection under another name,
  which the spec already calls the weaker comparison.

No decision record is proposed. The package adds no crate and no layer rule.
If the founder wants the runner choice or the fixture home recorded, each is
one short ADR.

## Risks

- **Text rasterization sets the tolerance.** Skia and Chromium draw text
  differently. If A1 only passes with a tolerance so loose that A2's colour
  change also passes, the pixel score cannot carry the verdict alone. T1
  reports both numbers; A2 is the check.
- **Instances.** UI kits are built from instances, and an instance's inside
  has no ids. Style scores on the benchmark screens may rest on few nodes.
  T4 records, per screen, the share of visible nodes that are mappable, and
  a screen below a floor set there is not chosen.
- **Geometric mapping favours layouts that already match.** A node the model
  placed far from its design box is unmapped, not wrong. Unmapped counts are
  in the result, and both scores count an unmapped node as zero.
- **Fonts fetched at run time.** `fanta-text` can download fonts
  (`fetch_downloadable_font`). T1 checks whether an engine run reaches the
  network and, if it does, fails the comparison on a missing family instead.
- **Components as the design.** `RenderTarget` needs a page. Whether a
  component master can be rendered and listed through it was not checked;
  T1 does, and R1's "or component" waits for T2 if it cannot.
- **Pencil.** The CLI is proprietary, its saved file came out empty here,
  and its behaviour can change between versions. Recording the JSON once
  removes it from benchmark runs, not from the conversion.
- **Cost.** 24 screens, four conditions and three runs are 288 agent runs
  per model and target. The held-out run is made once per model.
- **Licence.** The shadcn kit's page says CC BY 4.0 and its description
  mentions MIT (the attribution file kept with the sources). Both allow
  redistribution with notice; the committed attribution file records both.
- **R10 has no home yet.** Spec 0009's T8, which publishes the conformance
  fixtures, is blocked on ADR 0026. T7 commits the result here either way.

## Verification

| Criterion | How |
| --- | --- |
| A1, A4 | `node --test` in `web/fanta-compare`: the `pristine` corpus project against its own `page.svg`, run twice, outputs compared byte for byte |
| A2, A3 | The same test with four edited copies of the page and one with an element's id removed; asserts the node and property named |
| A5, A6 | `benchmark` with a stub runner that copies a prepared answer per condition; asserts the file's shape, the split and the verdict on fixed numbers |
| A7 | `examples/compare-ci/`: a workflow file and a script that changes one token and expects exit 1 with the token in the output |
| A8 | The result files of T7 and T8, read by the reviewer; the Expo section carries `"scored_on": "web build"` |
| A9 | `node --test`: every directory under `designs/` has a source with an attribution entry; the size budget |

The tests need a built `fanta-engine` and the pinned Chromium, which
`check.yml` has neither of today. T1 adds a workflow that runs them when
`web/fanta-compare/` or the engine crates change. Release stage touched by
T4: engine.

Checked on the development machine, 2026-11-12: Node 22.22.2, Google Chrome
148.0.7778.167, Playwright's Chromium build 1217 (147.0.7727.15, the build
Playwright 1.59.1 installs).
