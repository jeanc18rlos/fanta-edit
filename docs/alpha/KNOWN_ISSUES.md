# Known issues, Fanta 0.1.0-alpha.1

Written from the code and from the alpha rehearsal, not from wishful thinking.
Every entry is something a tester can hit on purpose. Where a claim comes from
reading source rather than from running the app, it says so. Re-checked entry by
entry against the tree at `fe2612c`; entries the fix pass closed have been
deleted rather than reworded.

## First launch

- **The DMG is ad-hoc signed**, so macOS refuses to open it until you go through
  System Settings > Privacy & Security > **Open Anyway**, or run
  `xattr -dr com.apple.quarantine /Applications/Fanta.app`. The full sequence is
  in [`RELEASE_NOTES.md`](RELEASE_NOTES.md). This is not a bug you can work
  around by re-downloading.
- **A folder that is not a design project still raises a Restricted Mode
  prompt.** Design projects no longer do: a directory with `fanta.json` at its
  root and no `.zed` directory, and a lone `.fig`/`.fnx` file, are trusted as
  they open (`auto_trust_design_projects`, `crates/zed/src/zed.rs`). Open
  anything else — a plain git checkout, a downloads folder — and you get the
  inherited blocking *"Unrecognized Project"* dialog, in Zed's wording, which
  **cannot be dismissed with Escape**; the title bar then reads *Restricted
  Mode* for the rest of the session.
  `"trust_all_worktrees"` is deliberately still `false` in
  `assets/settings/default.json`: a worktree's own `.zed/settings.json` can name
  a context-server command that this build will spawn, and trusting a worktree
  also un-suppresses git hooks, `diff.external` and credential helpers. Flipping
  it globally would have been a real security regression, so the guard is narrow
  on purpose.
- Launching a second `fanta <dir>` while the app is running prints
  `Fanta is already running` and exits without opening the project
  (`crates/zed/src/main.rs:390`; the single-instance check is live because
  `crates/zed/RELEASE_CHANNEL` is `stable`). Open the folder from the running
  app instead.

## Canvas and tools

- **Unwired faces stay visible and tell you so when clicked.** The toolbar and
  the layers/pages panels come from the vendored `fanta-gpui` crate, which
  exposes no host-side API to hide or disable a control — the comment at
  `view.rs:6837` records this. Hiding them would have meant patching a vendored
  crate on the eve of the alpha, so that was **deliberately deferred**; instead
  the host declines the action and raises a notice reading
  *"&lt;control&gt; is not available in the Fanta alpha yet."*
  (`view.rs::notify_unavailable`). You will meet it on the **Scale**, **Path
  Selection** and **Text-on-Path** tools (they have a tool object but no
  behaviour, so arming them would silently swallow every drag), on **Dev mode**,
  on the toolbar's **file-attach** and **voice** buttons, on **auto-keyframe
  recording**, on **"mark ready for dev"**, on **time-anchored comments**, on
  **page duplication** and **page links**, and on a number of inspector
  controls.
- **There is no Group or Ungroup.** Both are drawn — in the toolbar and in the
  layers context menu, with ⌘G / ⇧⌘G hints — because the vendored panels draw
  them. Neither is implemented anywhere in `crates/fig_viewer`; clicking either
  raises the notice above.
- Grid auto layout is offered in the inspector and the engine ignores it: the
  adapter logs a line and returns no operations at all
  (`gpui_adapters/design.rs::layout_mode_operations`). Horizontal and vertical
  auto layout work.
- Pass Through, Linear Burn and Linear Dodge are listed as blend modes and do
  nothing; the engine has no equivalent, so the adapter emits no operation
  (`gpui_adapters/design.rs`). The other blend modes map straight through.
- Effects are limited to drop shadow, inner shadow, layer blur and background
  blur. Every other effect kind is reported as unsupported in the inspector.

## Editing and files

- **Copy and paste works inside one document only.** A cross-document paste is
  refused with a message. Pasting an **image or a file from another app** onto
  the canvas does nothing at all and says nothing — the paste handler matches
  `ClipboardEntry::Image` and `ClipboardEntry::ExternalPaths` to `None` and
  drops them (`view.rs:2712`).
- **A bare `.fig` does not autosave.** Autosave is deliberately skipped until a
  project folder exists, because the first save scaffolds a directory next to
  the file and a timer should not do that behind your back: `autosave_allowed`
  requires `item.project_root().is_some()`. Press `cmd-s` once to materialise
  the project; after that, autosave takes over.
- **Every save touches three files, not one.** A one-node change shows up as
  `M pages/…/page.fnx`, `M pages/…/page.ids.json` and `M doc/metadata.json`.
  The third is the doc's own `modified_at`, which the 3-way merge tie-breaks on,
  so it stays. `fanta.json` no longer carries a per-save timestamp and has
  dropped out of the diff; a project written by an older build loses that line
  from `fanta.json` the first time it is saved — one-time, and it still opens.
- **A rectangle gains one attribute the first time a project is reloaded.**
  The loader backfills a vector's SVG viewport, so `<Rect width height />`
  becomes `<Rect width height local_size={[w, h]} />` on the first save after a
  reopen — one line per rectangle, once, and it stays a readable `<Rect>`
  thereafter. (A viewport that genuinely crops the shape has no `<Rect>`
  spelling and still prints as a canonical `<Vector>`; that is correct, not a
  regression.)
- Export lives in the inspector, not the toolbar, and needs a project root, so
  the project has to have been written at least once. Files land in
  `<project>/exports/` (`export.rs`).
- The Code tab is read-only by design. Edit `.fnx` in your own editor and the
  canvas follows the file. Note that with autosave on, `page.fnx`'s mtime moves
  about a second after any canvas edit — **an unchanged mtime is no longer a
  useful check for anything**, and [`SMOKE.md`](SMOKE.md) has been corrected
  accordingly.
- A design tab is restored on relaunch through its project folder; that was
  verified in the rehearsal. Restoring a `.fig` that never materialised a
  project was **not** verified — if it does not come back, reopen it from
  File > Open Recent.
- Opening a very large community `.fig` (100 MB and up) is slow. A 9.6 MB file
  took roughly 30–40 s to reach a rendered canvas on a debug build; the 128 MB
  case in the smoke checklist has never been run.
- **Memory has not been measured on a release build.** The debug build sat at
  5.1–6.3 GiB resident with one 9.6 MB `.fig` open, growing about 1.5 GiB in the
  first thirty seconds. A debug build says nothing about the DMG, but nobody has
  taken the release number either.

## Agents

- The agent panel needs a project open before it will chat. Opening a `.fig` or
  a Fanta project satisfies this.
- The managed Fanta provider requires signing in. Without it, set an Anthropic
  API key in the agent panel's settings; that is the alpha's default path.
- **`read_fnx_source` returns a slice, not a whole file.** It stops at 65,536
  bytes by default (`DEFAULT_SOURCE_BYTES`) and reports `total_lines`,
  `total_bytes` and `truncated`, with a `notice` naming the arguments for the
  next slice; `offset`, `limit` and `max_bytes` (ceiling 1 MiB,
  `MAX_SOURCE_BYTES`) ask for a different one. The page of a 9.6 MB `.fig` that
  used to come back as 43,145,940 characters in one block now measures 60,719.
  On a document that size, paging through it is still the wrong move — use
  `get_editor_state` and `batch_get` for structure. `batch_get` with no `ids`
  now defaults to `depth: 2` rather than walking the whole scene, and any result
  over 256 KiB (`MAX_JSON_RESPONSE_BYTES`) is refused with a message naming its
  size and how to narrow it.
- `get_screenshot` is not bounded the same way: a dense page at a large
  `max_dimension` can produce several MB of base64. The default of 1024 measured
  64,844 characters in the rehearsal and is safe; pass a large `max_dimension`
  only deliberately.
- Nothing removes the MCP discovery file
  (`~/Library/Application Support/Fanta/fanta_live_mcp.json`) when the app is
  killed, so it can name a dead process — the module comment at
  `mcp_stdio.rs:11` says so. The stdio bridge defends itself by checking the
  recorded pid with `kill(pid, 0)` before trusting it, but a third-party client
  reading that file directly will not.

## Version control

- **Without Xcode Command Line Tools, a new project is not a git repository.**
  The first save shells out to `git init -q` on `PATH`
  (`document.rs::git_init_if_needed`). On a Mac with no developer tools that
  invocation can raise the *"The git command requires the command line developer
  tools"* dialog, and the failure is logged and swallowed on purpose — a save
  must never fail because version control is unavailable. Result: **the design
  saves, but there is no `.git`**, so Review Changes has nothing to show and the
  headline "every canvas edit is a reviewable diff" quietly does not apply.
  Install the tools (`xcode-select --install`) or run `git init` in the project
  folder yourself.
- A project created inside an existing repository is deliberately not given its
  own nested `.git` — `git_init_if_needed` walks the ancestors — so its changes
  show up in the repository you already have.

## Network and privacy

- **Telemetry is off in the default settings.** `assets/settings/default.json`
  sets `telemetry.diagnostics`, `telemetry.metrics` and
  `telemetry.anthropic_retention` to `false`. That is a statement about the
  default settings, **not** a claim that nothing leaves the machine — no one has
  audited every outbound path in this build.
- **Font downloads reach GitHub.** A document using a font you do not have
  installed triggers a fetch from `raw.githubusercontent.com` (the
  `google/fonts` repository) and, for Source Sans/Serif Pro, from
  `github.com/adobe-fonts/…/releases/download/…`
  (`crates/fanta-text/src/font_resolver/download.rs`). Faces are cached under
  `~/.cache/fanta/fonts` and not re-fetched.
- **The Node and npm download paths are still compiled in.** `node_runtime` is
  still linked into the binary; it downloads Node from `nodejs.org/dist/…`
  (`node_runtime.rs:656`) and exposes `npm_install_packages`, which reaches
  `registry.npmjs.org` when an external ACP agent is installed. Nothing was
  observed reaching either during the rehearsal, but a strict network allow-list
  has to account for both.
- Because of the entries above, a network test that asserts "nothing from
  github.com" or "nothing from npm" is **wrong**. [`SMOKE.md`](SMOKE.md) step 9
  carries the full allow-list, each host cited to the file that names it.
- One `TCP 127.0.0.1:<port> (LISTEN)` socket stays open for the life of the
  process. The port matches the stable-channel single-instance handshake
  (`crates/zed/src/zed/mac_only_instance.rs`). Over roughly one idle minute the
  rehearsal saw zero bytes in or out and no outbound connections at all; the
  full five-minute idle test and the agent-turn test were never run.

## Leftovers from the Zed fork

- **Forty-four palette entries are still prefixed `zed:`.** The prefix is the
  action namespace, deliberately not renamed for the alpha — renaming namespaces
  breaks keymaps and every `zed::` action path in the tree. Measured by
  filtering `fanta --dump-all-actions` through the namespace and action-type
  filter this build installs: **929 of 1,031 actions are visible, and 44 of
  those are in the `zed` namespace** — including `zed: about`, `zed: quit`,
  `zed: open settings`, `zed: open zed url`, `zed: reset database` and
  `zed: copy system specs into clipboard`. `zed: test crash` and
  `zed: test panic` are listed but their handlers are only registered behind the
  server-side `PanicFeatureFlag` (`crates/zed/src/zed.rs:200`), so they do
  nothing unless that flag is on. This is the largest remaining branding leak
  and it is a deliberate deferral, not an oversight.
- **Zed branding survives in two places the rehearsal did not reach.**
  `crates/agent_ui/src/ui/end_trial_upsell.rs` still offers *"Upgrade to Fanta
  Pro"* and *"Your Fanta Pro Trial has expired"* for a subscription that does
  not exist; and `conversation_view.rs:2318` renders
  `"Authenticate to {agent_display_name}"`, which falls back to the raw agent id
  and can read *"Authenticate to Zed Agent"*. Identifiers, the `zed://` URL
  scheme, `ZED_*` env vars and action namespaces are unchanged on purpose.
- **This build never updates itself.** `script/bundle-mac` exports
  `ZED_UPDATE_EXPLANATION='Alpha builds are updated by downloading a new DMG
  from Fanta.'`, so "Check for Updates" shows that as an information prompt
  instead of erroring, and the poll of the Zed release endpoint is suppressed.
  Get new alpha builds by downloading a new DMG.
- One `ERROR … agent_ui/src/message_editor.rs:601 language not found` is logged
  once per launch — the agent composer asking for a Markdown grammar that this
  build no longer registers. **Cosmetic**, and it pre-dates this work; the
  current `~/Library/Logs/Fanta/Fanta.log` holds twelve of them across the
  session's launches and nothing else at ERROR level.
- Panic backtraces in the shipped build are **bare addresses**, because
  `script/bundle-mac` strips the binary. Symbolicate with the `fanta.dwarf`
  produced by the same build:

  ```
  atos -o fanta.dwarf -arch arm64 -l <load address> <address>
  ```

## Platform

- **Apple Silicon only.** `script/bundle-mac` builds for the host triple and
  this alpha was built on `aarch64-apple-darwin`; no Intel or universal DMG is
  produced.
- **Only one macOS version has been tested: 26.6.2, the build machine's.** The
  compiler deployment target is `10.15.7` (`crates/zed/build.rs`) and the bundle
  sets no `LSMinimumSystemVersion`, so nothing in the build refuses an older
  system — but nothing has been run on one either. Windows and Linux are not
  supported.

## Test suites

- The `sidebar` (8 failures) and `agent_ui` (14 failures plus one hang) test
  suites are **red**. These pre-date the alpha work and sit off every path it
  touched; the `agent_ui` failures were confirmed pre-existing by backtrace in
  an earlier pass. Those counts were **not re-run for this documentation pass**
  — they are carried over from the pass that measured them.
