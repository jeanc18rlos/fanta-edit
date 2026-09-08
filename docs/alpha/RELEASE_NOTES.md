# Fanta 0.1.0-alpha.1

**Your design is code.**

> A Fanta project is a git-tracked folder of .fnx source. Every canvas edit is a
> reviewable change; every source edit reloads the canvas.

That is the welcome page's own wording, and it is the whole product. Open a
`.fig` or start a new design and Fanta writes a project folder: `fanta.json`, a
`pages/` tree of `.fnx` source, `components/`, an `AGENTS.md` seed and a
`.git`. You draw on a canvas; the folder changes; `git diff` shows you what you
did. An agent editing the same folder is doing exactly what your cursor does.

## Install

The DMG is **ad-hoc signed** unless it was built with Developer ID credentials,
so macOS will refuse to open it on first launch. Every tester hits this in the
first minute. It is expected, and there are two ways past it.

1. Download `Fanta.dmg`, open it, drag **Fanta** to Applications.
2. Double-click Fanta. macOS shows an unverified-developer dialog — some
   version of *"Apple could not verify Fanta is free of malware"* or
   *"cannot be opened because the developer cannot be verified"*. Dismiss it.
3. Open **System Settings > Privacy & Security**, scroll to the Security
   section, and click **Open Anyway** next to the message about Fanta. Confirm,
   and launch Fanta again.

Or skip the dialog entirely from a terminal:

```
xattr -dr com.apple.quarantine /Applications/Fanta.app
```

Requirements: macOS 14 or later, Apple Silicon. The bundle script builds for the
host architecture and this alpha is built on `aarch64-apple-darwin`; there is no
Intel DMG.

## Connect Claude Code or Codex

Fanta runs a local MCP server over a Unix socket whenever the app is open. It is
**on by default** — turn it off with `"fanta_live_mcp": { "enabled": false }` in
settings. `fanta --mcp-stdio` bridges an stdio MCP client onto that socket.

Claude Code — the same line the welcome page's **Copy command** button puts on
your clipboard:

```
claude mcp add -s user fanta -- /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio
```

Codex — add to `~/.codex/config.toml`, and the welcome page's **Copy Codex
config** button:

```toml
[mcp_servers.fanta]
command = "/Applications/Fanta.app/Contents/MacOS/fanta"
args = ["--mcp-stdio"]
```

The server exposes five tools against whatever design is focused:
`get_editor_state`, `batch_get`, `batch_design`, `get_screenshot` and
`read_fnx_source`. A window toasts *"Agent connected: <client>"* when a client
completes its handshake.

With the app closed, the bridge prints one line and exits 2 — it does not
silently pretend to be connected:

```
Fanta is not running, or its live MCP server is off (settings: fanta_live_mcp.enabled)
```

## Every canvas edit is a git diff

Canvas edits **autosave**. Roughly a second after you stop editing, the document
is written to the project folder — no `cmd-s` needed, though `cmd-s` still
works. The debounce holds off while the pointer is down, while a text session is
open, while a prototype is playing and while a keyframe is being dragged, so
what lands on disk is a finished edit rather than a frame of a drag.

A new project is `git init`-ed the first time it is written, unless it is
already inside a repository. So the flagship loop is real and was measured on
this build: an external agent created a rectangle over MCP, nobody touched the
keyboard, and about five seconds later `git status --short` reported
`M pages/page-1/page.fnx`.

Editing `.fnx` in your own editor works in the other direction: the canvas
reloads and says so by name — *"<file> changed on disk — canvas updated"*.

## What else is in this build

- **Canvas editing**: shapes, frames, text, selection and transforms, auto
  layout (horizontal and vertical), gradients, shadows and blurs, components and
  instances, variables and modes.
- **Prototype flows and presentation**, motion clips with a timeline, and pinned
  comments. These ship but were not exercised in the alpha smoke run; treat them
  as alpha-grade.
- **A read-only Code tab** showing the `.fnx` and JSON behind what is on screen,
  with the file path, following the canvas selection.
- **Export** to PNG, JPEG, SVG and PDF, from the inspector, into
  `<project>/exports/`.
- **A built-in agent panel**. Point it at your own Anthropic API key in the
  agent panel's settings; signing in additionally enables the managed Fanta
  provider.
- **File > Review Changes** and **File > Commit…** open the project diff and the
  commit flow on the design folder itself.

## What came out of the binary

Two cut waves ran before this alpha, and the numbers below are from the
repository rather than from memory:

- **80 workspace members deleted**: 259 → 179 (`Cargo.toml` at `d25c2a8~1` and
  at `HEAD`).
- **375,076 lines deleted** across 1,296 files (50,528 in `d25c2a8`, 324,548 in
  `6e14229`).
- **163 workspace crates** now link into the app binary, out of a 970-crate
  dependency graph — down from 1,139 crates before the cuts.
- The debugger, vim, edit prediction, Copilot, the whole collab/livekit/
  libwebrtc stack, the extension host (and with it wasmtime and cranelift), the
  AWS SDK, fourteen unregistered model providers, fifteen tree-sitter parsers
  and every language server are gone. Fanta registers grammars for FNX,
  TypeScript and JSON and no language servers at all.
- The shipped binary is stripped. The wave that added stripping recorded
  350 MB → 263 MB; that figure has not been re-measured for this DMG. Debug
  symbols are always produced as `fanta.dwarf`.

## Known issues

[`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) is the honest list and it is not short. The
headlines: parts of the toolbar and inspector are visible but not wired up and
say so when clicked; there is no Group/Ungroup; cross-document and image paste
do not work; and the first launch goes through macOS Gatekeeper. Design projects
no longer raise the Restricted Mode prompt — a folder that is *not* a design
project still does, and that dialog cannot be dismissed with Escape.

## What was verified for this release

The external-agent loop was proven mechanically on a live document by
[`script/smoke-mcp`](../../script/smoke-mcp) (18 assertions, exit 0):
handshake, tool list, editor state naming each page's source file,
`read_fnx_source`, `batch_design`, autosave reaching disk, the resulting git
diff, `get_screenshot` and `batch_get`. It was run twice on a project freshly
imported from a 9.6 MB `.fig`, the second time after a full quit and reload, and
passed 18/18 both times.

The diff that run produces is the product thesis, so here it is verbatim. One
rectangle created over MCP, nobody at the keyboard, on the **first** edit after
the import:

```
 doc/metadata.json          | 2 +-
 pages/page-1/page.fnx      | 1 +
 pages/page-1/page.ids.json | 7 +++++++
 3 files changed, 9 insertions(+), 1 deletion(-)
```

```diff
+      <Rect blend_mode="normal" fills={[{"kind": "solid", "color": fnxColor("#FF3B30")}]} height={80.0} name="smoke-mcp rectangle" opacity={1.0} width={120.0} x={0.0} y={0.0} />
```

Everything behind a mouse click — the welcome page, menus, drawing with a tool,
undo/redo, the toolbar, the in-app agent panel — was **not** exercised. See
[`REHEARSAL.md`](REHEARSAL.md) for the row-by-row record and
[`SMOKE.md`](SMOKE.md) for the checklist a human still has to walk.
