# Fanta

**Your design is code.**

> A Fanta project is a git-tracked folder of .fnx source. Every canvas edit is a
> reviewable change; every source edit reloads the canvas.

Fanta is a design app whose file format is source. Open a `.fig` or start a new
design and Fanta writes a project folder — `fanta.json`, a `pages/` tree of
`.fnx` source, `components/`, an `AGENTS.md` seed — and `git init`s it. You draw
on a canvas; the folder changes; `git diff` shows you exactly what you did.

That makes agents first-class rather than bolted on. Claude Code and Codex
connect over MCP and edit the same source your cursor edits, and the canvas
reloads as the files change. Nothing is hidden inside an opaque binary document.

This is an **alpha**. It is rough in places, and
[`docs/alpha/KNOWN_ISSUES.md`](docs/alpha/KNOWN_ISSUES.md) is honest rather than
short.

```
cargo run -p zed                     # build and run; the binary is named fanta
claude mcp add -s user fanta -- \
  /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio   # connect an agent
script/smoke-mcp <fanta-binary> <project-dir>   # prove the whole loop, 18 checks
```

[Build from source](#build-from-source) · [Connect an agent](#connect-an-agent)
· [Install the DMG](#install)

## Requirements

- **Apple Silicon.** The bundle script builds for the host triple and the alpha
  is built on `aarch64-apple-darwin`; there is no Intel build.
- **macOS.** Only macOS 26.6.2 — the build machine's version — has been tested.
  The deployment target is 10.15.7 and no minimum is declared in the bundle, but
  no other version has been run.

## Install

Download the DMG, open it, drag **Fanta** to Applications.

The alpha DMG is ad-hoc signed, so macOS blocks the first launch. Either go
through **System Settings > Privacy & Security > Open Anyway** after the
unverified-developer dialog, or clear the quarantine attribute yourself:

```
xattr -dr com.apple.quarantine /Applications/Fanta.app
```

The full step-by-step is in
[`docs/alpha/RELEASE_NOTES.md`](docs/alpha/RELEASE_NOTES.md).

## Connect an agent

Fanta runs a local MCP server whenever the app is open — on by default. Register
it once:

```
claude mcp add -s user fanta -- /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio
```

For Codex, add this to `~/.codex/config.toml`:

```toml
[mcp_servers.fanta]
command = "/Applications/Fanta.app/Contents/MacOS/fanta"
args = ["--mcp-stdio"]
```

Both strings are what the welcome page's **Copy command** and **Copy Codex
config** buttons put on your clipboard. When a client connects, the Fanta window
toasts *"Agent connected: &lt;client&gt;"*. The server exposes six tools
against the focused design: `get_editor_state`, `get_guidelines`, `batch_get`,
`batch_design`, `get_screenshot` and `read_fnx_source`. `batch_design` speaks
the `design_surface::DesignOp` vocabulary — create, style, auto-layout, align,
distribute, group, duplicate, componentise — as one undo step per call.

Then ask for something — *"add a 200x100 blue rectangle called Hero to this
page"*. The agent's edit lands on the canvas, autosaves to the project folder
about a second later, and shows up in `git diff` like any other change.

## Build from source

```
cargo run -p zed
```

The binary is named `fanta`. Useful entry points:

- `cargo build -p zed` — the app.
- `./script/clippy` — lints. Use this rather than `cargo clippy` directly.
- `./script/bundle-mac` — builds and signs `Fanta.app` and the DMG. It strips
  the shipped binary and always emits `fanta.dwarf` alongside it, so panic
  backtraces are symbolicated with
  `atos -o fanta.dwarf -arch arm64 -l <load address> <address>`.
- `./script/smoke-mcp <fanta-binary> <project-dir>` — drives the whole
  canvas-to-agent-to-git loop against a running app and asserts 18 things about
  it. The fastest way to know a build works.

The design engine lives in the vendored `crates/fanta-*` crates; the app shell
is a fork of Zed with the IDE removed — 179 workspace members remain, of which
164 link into the app binary, inside a 904-crate dependency graph
(`cargo tree -p zed --edges normal`). The debugger, vim, edit prediction,
Copilot, collab, livekit, dev containers, the file finder, the project panel,
the outline panel, onboarding, the extension host, wasmtime and the AWS SDK are
no longer linked.

## Getting started in the app

1. **File > Open...** a `.fig` file, or **File > New Design...** to start empty.
   Opening a `.fig` writes a Fanta project folder beside it on open, and
   `git init`s it; that folder, not the original `.fig`, is what your edits go
   to. Large files take a while: on a release build, a 9.6 MB `.fig` reaches a
   canvas that answers in about 2 s and a 128 MB one in about 50 s (measured
   over MCP on 2026-09-09, launch to a canvas that reports a node count — nobody
   watched the window, so this is not time-to-a-frame-you-can-look-at; see
   [`docs/alpha/REHEARSAL.md`](docs/alpha/REHEARSAL.md)).
2. Draw. Edits autosave about a second after you stop — no `cmd-s` needed.
   `cmd-g` groups the selection, `cmd-alt-g` wraps it in a frame,
   `cmd-shift-g` ungroups; pasting an image from another app places it.
3. **File > Review Changes** shows the diff your edits made; **File > Commit…**
   commits them.
4. Open the **Code** tab to read the `.fnx` behind what you are looking at. It
   is read-only in the app on purpose: edit it in your own editor and the canvas
   follows the file.
5. To use the built-in agent panel instead of an external one, paste an
   Anthropic API key under its settings, or sign in for the managed provider.

## Known issues in this alpha

The full list is
[`docs/alpha/KNOWN_ISSUES.md`](docs/alpha/KNOWN_ISSUES.md). The ones you will
meet first:

- Parts of the toolbar and inspector are visible but not wired up. They say so
  when clicked rather than failing silently.
- Grid auto-layout, unsupported effect families, Pattern/Image/Video/Shader
  paints, bound or unsupported gradients, Linear Burn, and Linear Dodge are
  explicitly unavailable; shadow blend is read-only. Fill/stroke visibility,
  supported solid and finite unbound identity-transform gradient payloads, and
  per-paint blend-mode changes now produce undoable edits in the current
  candidate, with exact-artifact native coverage still outstanding.
- Canvas copy and paste works within one document only. Pasting an image works;
  pasting any other kind of file still does nothing.
- The complete manual smoke checklist has not passed against one artifact.
  Focused native fixtures have exercised Scale, Path Select, generation and
  restart recovery; other pointer-only surfaces remain source or automated-test
  evidence until the final packaged-app pass.
- Memory is high. A large document settles at about 2 GiB, and the first edit
  plus its autosave took that to 5.8 GiB on a 128 MB file — about 3.8 GiB for
  one rectangle — before later autosaves brought it back to about 5 GiB.
- Opening a folder that is *not* a design project raises an inherited
  Restricted Mode dialog that Escape will not dismiss. A design project Fanta
  scaffolded is trusted automatically; one you received as a zip or a copied
  folder is only trusted if its `.git` has no hooks and a config every line of
  which is on a short allowlist of keys known to be inert, so a shared project
  can prompt too.
- Without Xcode Command Line Tools installed, a new project saves but is not
  git-initialised.
- Very large community `.fig` files are slow to open: a 128 MB, 40,141-node one
  took 48-54 s across three runs.
- On a very wide page, an unpaged `batch_get` listing overflows the 256 KiB
  response cap — 967,017 bytes on a 40,141-node document. Page it with
  `offset`/`limit`, new in this change: the page that refused has 8,920 direct
  children and lists in 45 windows of 200.

## Feedback

There is no public issue tracker for the alpha yet — report back through
whoever gave you the build. Logs live in `~/Library/Logs/Fanta/`, and attaching
`Fanta.log` makes almost every report easier to act on. One ERROR line in there
is known and cosmetic — the Metal renderer's `scene too large … retrying`. The
`language not found` line older builds logged once per launch is gone: across
six launches of a release build on 2026-09-09 the log recorded zero ERROR lines
and zero panics. Anything at ERROR level is worth sending.

If the app crashed, the shipped binary is stripped, so a backtrace is bare
addresses. `script/bundle-mac` writes `fanta.dwarf` next to the binary it built;
symbolicate with `atos -o fanta.dwarf -arch arm64 -l <load address> <address>`.

## License

GPL-3.0-or-later. Fanta is a fork of Zed; see `NOTICE.md` and `LICENSE-GPL`.
Source for every binary we ship is this repository, including the vendored
engine crates under `crates/fanta-*`.
