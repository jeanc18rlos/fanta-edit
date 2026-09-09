# Fanta 0.1.0-alpha.1

**Your design is code.**

> A Fanta project is a git-tracked folder of .fnx source. Every canvas edit is a
> reviewable change; every source edit reloads the canvas.

Those two lines are quoted verbatim from `HERO_HEADLINE` and `HERO_SUBTITLE` in
`crates/workspace/src/welcome.rs`, so this document and the app say the same
thing. It is also the whole product. Open a `.fig` or start a new design and
Fanta writes a project folder — `fanta.json`, a `pages/` tree of `.fnx` source,
`components/`, an `AGENTS.md` seed — and `git init`s it. You draw on a canvas;
the folder changes; `git diff` shows you what you did. An agent editing the same
folder is doing exactly what your cursor does.

Every feature sentence below was re-checked against the working tree of
`perf/large-documents-ai-alignment` — the uncommitted work on top of
`16309b2` — by grepping for the thing it claims. Where something was measured,
the number is the measurement and the command that produced it, and the commit
it was taken at; every measurement in this document was taken at `16309b2` or
earlier, and **no binary has been built from this tree**. The tree changed the
autosave write path, image decoding, drag rendering, the layers tree and the
`.fig` import pipeline, so the memory and timing figures describe code this
tree replaced; [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) says so entry by entry.
Where nothing was run, [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) and
[`REHEARSAL.md`](REHEARSAL.md) say so instead — and `REHEARSAL.md`'s "could not
verify" rows are not restated here as features.

## Install

The DMG is **ad-hoc signed** unless it was built with Developer ID credentials,
so macOS will refuse to open it on first launch. Every tester hits this in the
first minute. It is expected, and there are two ways past it.

1. Download `Fanta.dmg`, open it, drag **Fanta** to Applications.
2. Double-click Fanta. macOS shows an unverified-developer dialog — some
   wording of *"Apple could not verify 'Fanta' is free of malware that may harm
   your Mac or compromise your privacy"*, or on older systems
   *"Fanta cannot be opened because the developer cannot be verified"*. Click
   **Done** (or **Cancel**). Do not click *Move to Trash*.
3. Open **System Settings > Privacy & Security**, scroll down to the Security
   section, and you will see *"Fanta was blocked to protect your Mac."* Click
   **Open Anyway** next to it, authenticate, and confirm **Open Anyway** in the
   dialog that follows.
4. Fanta launches. You only do this once.

Or skip the whole dance from a terminal, which strips the quarantine attribute
the download put on the bundle:

```
xattr -dr com.apple.quarantine /Applications/Fanta.app
```

**Apple Silicon only.** `script/bundle-mac` builds for the host triple and this
alpha was built on `aarch64-apple-darwin`; no Intel or universal DMG is
produced.

**macOS version: only one has been tested.** The build machine runs macOS
26.6.2 and that is the only version this alpha has been launched on. The
compiler deployment target is `10.15.7` (`crates/zed/build.rs`) and the bundle
declares no `LSMinimumSystemVersion`, so nothing in the build *stops* an older
macOS from opening it — but nobody has tried, and this is not a support claim.

## Connect Claude Code or Codex

Fanta runs a local MCP server over a Unix socket whenever the app is open. It is
**on by default** (`FantaLiveMcpSettings` defaults `enabled` to `true` in
`crates/fig_viewer/src/live_mcp.rs`); turn it off with
`"fanta_live_mcp": { "enabled": false }` in settings. `fanta --mcp-stdio`
bridges a stdio MCP client onto that socket, and it is documented in
`fanta --help`.

Claude Code — this is the exact string the welcome page's **Copy command**
button puts on your clipboard (`CONNECT_CLAUDE_CODE_COMMAND`):

```
claude mcp add -s user fanta -- /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio
```

Codex — add this to `~/.codex/config.toml`; it is what the welcome page's
**Copy Codex config** button copies (`CONNECT_CODEX_CONFIG`):

```toml
[mcp_servers.fanta]
command = "/Applications/Fanta.app/Contents/MacOS/fanta"
args = ["--mcp-stdio"]
```

**What to expect when it works.** The client's handshake completes, and the
Fanta window raises a toast reading *"Agent connected: &lt;client name&gt;"*.
`tools/list` returns six tools, scoped to whichever design is focused
(`server.add_tool(…)` in `crates/fig_viewer/src/live_mcp.rs`):

| Tool | What it does |
|---|---|
| `get_editor_state` | project root, pages, which page is active, each page's source file, the selection and its bounds; pass `empty_space: [w, h]` to also get a free spot for a new frame |
| `get_guidelines` | Fanta's design guidelines for agents (`design_surface::DESIGN_GUIDELINES`); read once per session |
| `batch_get` | reads nodes; with no `ids` it lists the active page's tree at `depth: 2` |
| `batch_design` | applies a list of design operations to the canvas as one undo step — the `design_surface::DesignOp` vocabulary, from `create_node` to `group`, `align` and `create_component` |
| `get_screenshot` | renders the open page (or a node) to PNG |
| `read_fnx_source` | returns a slice of a `.fnx` file — 64 KiB by default, 1 MiB ceiling |

`get_guidelines` and most of the `batch_design` vocabulary landed on this
branch and have only been exercised by unit tests; the five tools the release
build was smoke-tested with were the other five, and `script/smoke-mcp` drives
exactly one op, `create_node`.

**What to expect when it does not.** With the app closed, the bridge prints one
line and exits 2 rather than pretending to be connected:

```
Fanta is not running, or its live MCP server is off (settings: fanta_live_mcp.enabled)
```

## Every canvas edit is a git diff

Canvas edits **autosave**. About a second after you stop editing
(`AUTOSAVE_DEBOUNCE`, `crates/fig_viewer/src/view.rs`) the document is written
into the project folder — no `cmd-s` needed, though `cmd-s` still works. The
debounce stands down while the pointer is down, while a text session is open,
while a prototype is playing and while a keyframe is being dragged
(`autosave_allowed`), so what lands on disk is a finished edit rather than a
frame of a drag.

A new project is `git init`-ed the first time it is written
(`git_init_if_needed`, `crates/fig_viewer/src/document.rs`) unless it
already sits inside a repository, in which case its changes show up in the
repository you already have. That call shells out to `git` on `PATH`, and a
failure is logged and swallowed so that a save never fails because version
control is unavailable — which on a Mac with no Xcode Command Line Tools means
the design saves and the folder is *not* a repository. See
[`KNOWN_ISSUES.md`](KNOWN_ISSUES.md).

That loop was measured on a live document at `fe2612c`, not asserted. One
rectangle created over MCP by an outside process, nobody at the keyboard, on the
**first** edit after importing a 9.6 MB `.fig`:

```
 doc/metadata.json          | 2 +-
 pages/page-1/page.fnx      | 1 +
 pages/page-1/page.ids.json | 7 +++++++
 3 files changed, 9 insertions(+), 1 deletion(-)
```

and the `page.fnx` half of that diff is one readable line:

```diff
+      <Rect blend_mode="normal" fills={[{"kind": "solid", "color": fnxColor("#FF3B30")}]} height={80.0} name="smoke-mcp rectangle" opacity={1.0} width={120.0} x={0.0} y={0.0} />
```

The same edit against the previous build produced **25,311 insertions and
25,310 deletions**. The cause was float formatting, not content: `serde_json`'s
default number parser is accurate only to within 1 ULP, so a float printed by
the import path came back one bit different from the same float printed by the
save path. Both `fanta-fnx` and `fanta-format` now enable serde_json's
`float_roundtrip` feature, and `fanta.json` no longer carries a per-save
timestamp, which took it out of the diff entirely.

The write behind that diff changed on this branch and the diff has not been
re-taken. `FigItem::save` now persists `Doc::clone_for_persist()` — the
document without its undo history and selection, which were never written —
through `fanta_format::write_project_tree_cached`, which fingerprints each
page and component and reuses last save's bytes for any design that did not
change (`ProjectWriteCache`, `crates/fanta-format/src/project/write.rs`). The
crate's contract, and its tests (`tests/write_cache.rs`), is that the bytes are
identical to a cold write and that the same write-if-changed and prune steps
still run, so the three-file diff above is what the code says you should still
get; nobody has run `git status` after a save from this tree.

Editing `.fnx` in your own editor works in the other direction: the canvas
reloads and names the file that changed — *"&lt;file&gt; changed on disk —
canvas updated"*, or *"… merged into your unsaved canvas edits"* if you had
uncommitted canvas work.

## What else is in this build

A warning about this section: it is a list of what the code contains, not a list
of what was tested. The only canvas operations ever driven end to end are the
ones `script/smoke-mcp` performs — create a rectangle with a fill, read it back,
render it, and watch it reach git. Everything else below is a source reading.
[`SMOKE.md`](SMOKE.md) is the checklist that would turn these into claims.

- **Canvas editing**: shapes, frames, text, selection and transforms, auto
  layout (horizontal and vertical only — grid is ignored, see Known Issues),
  gradients, shadows and blurs (four effect kinds only), components and
  instances, and variables with modes (`variable_binding.rs`,
  `mode_overrides.rs`).
- **Group, Frame selection and Ungroup** — `cmd-g`, `cmd-alt-g` and
  `cmd-shift-g` (`assets/keymaps/default-macos.json`), the toolbar's commands
  of the same names and the layers context menu, all built by
  `crates/fig_viewer/src/structure.rs` as one undo step each. Members keep
  their world position; a page, a component master or (for ungroup) an
  instance is refused with a canvas notice. New on this branch; unit-tested,
  never pressed.
- **Pasting an image** from another app, or a `png`/`jpg`/`jpeg`/`webp`/`gif`/
  `bmp`/`tiff`/`tif` file from Finder, places it as a bitmap layer centred on the
  viewport and adds it to the project's assets (`paste_selected_nodes` and
  `paste_image`, `crates/fig_viewer/src/view.rs`). Other file types are still
  ignored. New on this branch; source reading.
- **Toolbar AI commands** (*Generate a design*, *Replace content*, *Rewrite
  text*, *Translate text*, *Rename layers*) and the toolbar's AI box open a
  draft in the Agent Panel, prefixed with the page and selection, that you
  still have to send (`route_toolbar_agent_prompt`, `view.rs`). *Remove
  background* and *Generate an image* say they are unavailable.
- **Large-document paths reworked**, all as source readings: the render
  thread receives per-node patches for a drag instead of a scene copy
  (`ScenePatch`, `crates/fig_viewer/src/canvas.rs`), the layers panel builds
  only the rows it can show (`gpui_adapters/layers.rs::layers_tree`), images
  decode on first draw under a 1.5 GiB cap (`fanta_render::asset::LazyAssetResolver`),
  selections over 512 layers draw one outline (`SELECTION_OUTLINE_CAP`), and
  the `.fig` importer no longer decodes `byte[]` fields one value per byte or
  deep-clones the change tree to resolve shared styles
  (`crates/fanta-fig-interop`). None of it has been timed in the app.
- **Prototype flows and presentation**, motion clips with a timeline, and pinned
  comments. These are compiled in but were **not exercised** in the alpha smoke
  run at all; treat them as unverified.
- **A read-only Code tab** showing the `.fnx` and JSON behind what is on screen,
  with the file path, following the canvas selection.
- **Export** to PNG, JPG, SVG and PDF (`ExportFormat`,
  `crates/fig_viewer/src/export.rs`), from the inspector, into
  `<project>/exports/`.
- **A built-in agent panel**. Point it at your own Anthropic API key in the
  agent panel's settings; signing in additionally enables the managed Fanta
  provider.
- **File > Review Changes** and **File > Commit…**
  (`crates/zed/src/zed/app_menus.rs`) open the project diff and the commit flow
  on the design folder itself.
- **A working Edit menu on the canvas**: Undo, Redo, Cut, Copy, Paste and
  Select All are wired to the canvas, not only to text editors.

## What came out of the binary

Two cut waves ran before this alpha. Every number here was measured against this
repository just now, with the command that produced it.

- **80 workspace members deleted: 259 → 179.** Counted just now from the
  `members` list in `Cargo.toml` at `d25c2a8~1` and at `16309b2`.
- **375,496 lines deleted across 1,292 files.** `git diff --shortstat
  d25c2a8~1 HEAD`, re-run just now. The insertion side of that stat is not
  quoted, because it moves every time one of these documents is edited.
- **164 of the 179 workspace members link into the app binary**, inside a
  **904-crate** total dependency graph. Re-measured just now with
  `cargo tree -p zed --edges normal --offline`, intersected against the package
  name in each member's `Cargo.toml`. The fifteen members that do not link are
  `etw_tracing`, `eval_utils`, `go_to_line`, `gpui_linux`, `gpui_web`,
  `gpui_wgpu`, `gpui_windows`, `json_schema_store`, `languages`,
  `markdown_preview`, `outline`, `outline_panel`, `project_panel`, `vim` and
  `windows_resources`. The equivalent figure before the cuts is not quoted here,
  because measuring it would require checking out the old tree.
- **No longer linked into the app binary.** Each of these was checked by name
  against that dependency list and is absent: `debugger_ui`, `vim`, `zeta`
  (edit prediction), `copilot`, `collab`, `livekit_client`, `dev_container`,
  `file_finder`, `project_panel`, `outline_panel`, `onboarding`,
  `extension_host`, `edit_prediction_button`, `wasmtime` and `aws-sdk-s3`.
  Some still exist as workspace members — the claim is that the app does not
  link them, not that the directories are gone. `terminal_view` **is** still
  linked, deliberately: an external ACP agent runs in one.
- **Fanta registers six grammars and zero language servers.** `fanta_languages`
  registers `fnx`, `tsx`, `typescript`, `json`, `jsonc` and `regex` for syntax
  highlighting, with no LSP adapters, no toolchain listers and no task providers
  — which is what stops the app downloading Node and starting a TypeScript
  server when you open a design.
- **The shipped binary is stripped** by `script/bundle-mac`, and the same script
  always emits `fanta.dwarf`. The stripped binary's size has **not** been
  measured for this DMG; an earlier wave recorded 350 MB → 263 MB and that
  figure is not re-quoted as current.

## Known issues

[`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) is the honest list and it is not short.
The headlines: parts of the toolbar and inspector are visible but not wired up
and say so when clicked; cross-document paste does not work; nothing on this
branch — Group/Ungroup, image paste, the new agent ops, the reworked save and
render paths — has been driven in a running build; the memory and timing
figures are from the previous build; and the first launch goes through macOS
Gatekeeper. A design
project Fanta scaffolded opens without the Restricted Mode prompt; a folder that
is *not* a design project — and a design project whose copied `.git` carries
anything this build does not recognise as inert — still raises it, and that
dialog cannot be dismissed with Escape.

## What was verified for this release

The external-agent loop was proven mechanically against a live document by
[`script/smoke-mcp`](../../script/smoke-mcp): 18 assertions covering the
handshake, `ping`, the tool list, editor state naming each page's source file,
`read_fnx_source`, `batch_design` creating a node, the canvas going dirty, the
autosave reaching disk, the resulting git diff, `get_screenshot` and
`batch_get`. At `fe2612c` it was run twice against the built binary on a project
freshly imported from a 9.6 MB `.fig` — the second time after a full quit and a
cold reload of the 43 MB page — and passed **18 of 18** both times, exit 0. The
Unix-socket fallback (`--socket`) passed 18/18 earlier, at `6e14229`, and has
not been re-run since. It also fails correctly: pointed at a directory that is
not a repository, or at a path that is not an executable, it reports the failed
assertion and exits 1.

**The release build has since been made and smoke-tested.** `script/bundle-mac`
at `16309b2` exited 0, and `script/smoke-mcp` was then run twice against the
shipped app installed at `/Applications/Fanta.app` — once reusing a running
instance, once launching it cold — and passed **18 of 18** both times. So the
loop is proven in the artefact a tester downloads, not only in a dev build. The
one thing that build cannot prove about itself is Gatekeeper: it was never
downloaded through a browser, so it carries no quarantine attribute and the
"Apple could not verify" dialog above has still never been seen. Row G of
[`SMOKE.md`](SMOKE.md) is the only way to settle that.

**Nothing on `perf/large-documents-ai-alignment` has been driven.** The smoke
script has not been re-run against a build of this tree; if it were, its
`tools/list` assertion would still pass — it checks that the five names in
`REQUIRED_TOOLS` are present, not that there are five (`script/smoke-mcp`) —
and its one `batch_design` op, `create_node`, is unchanged.

The CLI checks below were re-run against that release binary, not carried over.

- `fanta --version` → `fanta 0.1.0-alpha.1`.
- `fanta --help | grep -ic zed` → `0`, and `--mcp-stdio` appears in that help
  output. The flag is no longer `hide = true` — verified in the tree just now at
  `crates/zed/src/main.rs:1598`.
- `fanta --system-specs` → `Fanta System Specs (from CLI):` /
  `Fanta: v0.1.0-alpha.1+stable.<sha> (debug build)`. No `Zed` anywhere.
- `fanta --dump-all-actions` → **1,031** actions, of which **929** survived
  `HIDDEN_NAMESPACES` and `hide_action_types`, **44** of those in the `zed`
  namespace, with **zero** matches for "vim", "debugger", "project panel" or
  "new file" — and one for "terminal", `agent: new terminal thread`, which is a
  real feature rather than a leftover. This is a derivation over the action
  registry, **not** the palette as a human sees it: nobody has typed into the
  palette in a running build. What *was* re-checked in the tree just now is the
  filter itself — `crates/zed/src/main.rs` hides the `terminal`, `terminal_panel`
  and `vim` namespaces, and `hide_action_types` names `workspace::NewFile`,
  `NewFileSplit`, `NewFileSplitHorizontal`, `NewFileSplitVertical` and
  `pane::RevealInProjectPanel`, which are exactly the four rows the rehearsal
  recorded as failing.

Measured against the tree just now rather than against a binary: 179 workspace
members, 164 of them linked, a 904-crate graph, and the deletion totals above.
`~/Library/Logs/Fanta/Fanta.log` (103 KB, five launches of that older binary)
holds **13 ERROR lines — twelve `language not found`, one `scene too large`**,
**zero** `didn't find an action` lines, and no panic beyond
`panic handler registered`. Re-counted just now; the log itself pre-dates
`16309b2`.

Everything behind a mouse click — the welcome page as rendered, the menus,
drawing with a tool, undo/redo by hand, the toolbar, the in-app agent panel —
was **not** exercised. [`REHEARSAL.md`](REHEARSAL.md) is the row-by-row record
of what was and was not driven, and [`SMOKE.md`](SMOKE.md) is the checklist a
human still has to walk before this DMG goes out.
