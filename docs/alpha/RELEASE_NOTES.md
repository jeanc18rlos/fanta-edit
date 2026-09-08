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

Every feature sentence below was checked against this tree at commit `fe2612c`.
Where something was measured, the number is the measurement. Where nothing was
run, [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) and [`REHEARSAL.md`](REHEARSAL.md) say
so instead.

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
`tools/list` returns five tools, scoped to whichever design is focused:

| Tool | What it does |
|---|---|
| `get_editor_state` | project root, pages, which page is active, and each page's source file |
| `batch_get` | reads nodes; with no `ids` it lists the active page's tree at `depth: 2` |
| `batch_design` | applies a list of design operations to the canvas |
| `get_screenshot` | renders the open page (or a node) to PNG |
| `read_fnx_source` | returns a slice of a `.fnx` file — 64 KiB by default, 1 MiB ceiling |

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
(`git_init_if_needed`, `crates/fig_viewer/src/document.rs`) unless it already
sits inside a repository, in which case its changes show up in the repository
you already have.

That loop was measured on a live document, not asserted. One rectangle created
over MCP by an outside process, nobody at the keyboard, on the **first** edit
after importing a 9.6 MB `.fig`:

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

Editing `.fnx` in your own editor works in the other direction: the canvas
reloads and names the file that changed — *"&lt;file&gt; changed on disk —
canvas updated"*, or *"… merged into your unsaved canvas edits"* if you had
uncommitted canvas work.

## What else is in this build

- **Canvas editing**: shapes, frames, text, selection and transforms, auto
  layout (horizontal and vertical only — see Known Issues), gradients, shadows
  and blurs, components and instances, variables and modes.
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

- **80 workspace members deleted: 259 → 179.** Counted from the `members` list
  in `Cargo.toml` at `d25c2a8~1` and at `fe2612c`.
- **375,444 lines deleted across 1,291 files**, against 5,759 inserted.
  `git diff --shortstat d25c2a8~1 fe2612c`.
- **164 workspace crates** now link into the app binary, inside a **904-crate**
  total dependency graph. `cargo tree -p zed --edges normal`. The equivalent
  figure before the cuts is not quoted here, because measuring it would require
  checking out the old tree.
- **No longer linked into the app binary**, each one checked against
  `cargo tree -p zed --edges normal`: the debugger, vim, edit prediction, GitHub
  Copilot, collab, livekit, dev containers, the file finder, the project panel,
  the outline panel, onboarding, the extension host, wasmtime and the AWS SDK.
  Some of these still exist as workspace members — the claim is that the app
  does not link them, not that the directories are gone.
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
and say so when clicked; there is no Group or Ungroup; cross-document and image
paste do not work; and the first launch goes through macOS Gatekeeper. Design
projects no longer raise the Restricted Mode prompt — a folder that is *not* a
design project still does, and that dialog cannot be dismissed with Escape.

## What was verified for this release

The external-agent loop was proven mechanically against a live document by
[`script/smoke-mcp`](../../script/smoke-mcp): 18 assertions covering the
handshake, `ping`, the tool list, editor state naming each page's source file,
`read_fnx_source`, `batch_design` creating a node, the canvas going dirty, the
autosave reaching disk, the resulting git diff, `get_screenshot` and
`batch_get`. It was run twice against the built binary on a project freshly
imported from a 9.6 MB `.fig` — the second time after a full quit and a cold
reload of the 43 MB page — and passed **18 of 18** both times, exit 0. It also
fails correctly: pointed at a directory that is not a repository, or at a path
that is not an executable, it reports the failed assertion and exits 1.

Also verified directly, by running the binary built from this tree:
`fanta --version` prints `fanta 0.1.0-alpha.1`; `fanta --help | grep -ic zed`
returns `0`, and `--mcp-stdio` is documented in that help output;
`fanta --system-specs` opens with `Fanta: v0.1.0-alpha.1+stable.<sha>` and no
longer says `Zed`; and filtering `fanta --dump-all-actions` (1,031 actions)
through the namespace and action-type filter this build installs leaves 929
visible, with nothing matching "vim", "debugger", "project panel" or "new
file".

Everything behind a mouse click — the welcome page as rendered, the menus,
drawing with a tool, undo/redo by hand, the toolbar, the in-app agent panel —
was **not** exercised. [`REHEARSAL.md`](REHEARSAL.md) is the row-by-row record
of what was and was not driven, and [`SMOKE.md`](SMOKE.md) is the checklist a
human still has to walk before this DMG goes out.
