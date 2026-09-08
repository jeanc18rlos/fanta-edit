# Known issues, Fanta 0.1.0-alpha.1

Written from the code and from the alpha rehearsal, not from wishful thinking.
Every entry is something a tester can hit on purpose. Where a claim comes from
reading source rather than from running the app, it says so.

## First launch

- **The DMG is ad-hoc signed**, so macOS refuses to open it until you go through
  System Settings > Privacy & Security > Open Anyway, or run
  `xattr -dr com.apple.quarantine /Applications/Fanta.app`. See
  RELEASE_NOTES.md. This is not a bug you can work around by re-downloading.
- **A folder that is not a design project still raises a Restricted Mode
  prompt.** Design projects no longer do: a directory with `fanta.json` at its
  root and no `.zed` directory, and a lone `.fig`/`.fnx` file, are trusted as
  they open (`auto_trust_design_projects`, `crates/zed/src/zed.rs`). Verified on
  a freshly imported project — no *"Unrecognized Project"* dialog, and no
  `is not trusted` line in the log — against a plain git repo opened by the same
  binary, which still logs
  `Worktree "…/control" is not trusted` and raises the prompt. Open a folder
  that is not a design project and you get the old blocking dialog, in Zed's
  wording, which **cannot be dismissed with Escape**; the title bar then reads
  *Restricted Mode* for the session. `"trust_all_worktrees"` is deliberately
  still `false`: a worktree's own `.zed/settings.json` can name an MCP server
  command that this build will spawn, so trusting every folder blindly would be
  a drive-by code-execution hole. **The trap:** the palette still offers
  *zed: open project settings file*, which writes `.zed/settings.json` into the
  project — run it once and that design project re-arms the prompt on every
  future open.
- Launching a second `fanta <dir>` while the app is running prints
  `Fanta is already running` and exits without opening the project
  (`crates/zed/src/main.rs`; the single-instance check is live because
  `crates/zed/RELEASE_CHANNEL` is `stable`). Open the folder from the running
  app instead.

## Canvas and tools

- **Unwired faces stay visible and tell you so when clicked.** The toolbar and
  the layers/pages panels come from the vendored `fanta-gpui` crate, which has
  no host-side API to hide or disable a control (`tools.rs::is_stub`,
  `view.rs::handle_toolbar_action`). Rather than patch a vendored crate on the
  eve of the alpha, the host declines the action and raises a notice:
  *"<control> is not available in the Fanta alpha yet."* You will meet this on
  the Scale, Path Selection and Text-on-Path tools, on Dev mode, on the
  toolbar's file-attach and voice buttons, on auto-keyframe recording, on
  "mark ready for dev", on time-anchored comments, on page duplication and page
  links, and on a number of inspector controls.
- **There is no Group or Ungroup.** Both appear in the toolbar and the layers
  context menu, with ⌘G / ⇧⌘G hints, because the vendored panels draw them.
  Neither is implemented; clicking either raises the notice above.
- Grid auto layout is offered in the inspector and the engine ignores it — the
  adapter returns no operations at all
  (`gpui_adapters/design.rs::layout_mode_operations`). Horizontal and vertical
  auto layout work.
- Pass through, Linear Burn and Linear Dodge are listed as blend modes and do
  nothing; the engine has no equivalent, so the adapter emits no operation. The
  other fifteen modes map straight through.
- Effects are limited to drop shadow, inner shadow, layer blur and background
  blur. Every other effect kind reports *"Not supported by this document
  engine"* in the inspector.

## Editing and files

- **Copy and paste works inside one document only.** A cross-document paste is
  refused with a message. Pasting an **image or a file from another app** onto
  the canvas does nothing at all and says nothing — the paste handler drops
  `ClipboardEntry::Image` and `ClipboardEntry::ExternalPaths` on the floor
  (`view.rs::paste_selected_nodes`).
- **A bare `.fig` does not autosave.** Autosave is deliberately skipped until a
  project folder exists, because the first save scaffolds a directory next to
  the file and a timer should not do that behind your back
  (`view.rs::autosave_allowed`). Press `cmd-s` once to materialise the project;
  after that, autosave takes over.
- **Every save touches three files, not one.** A one-node change shows up as
  `M pages/…/page.fnx`, `M pages/…/page.ids.json` and `M doc/metadata.json`.
  The third is the doc's own `modified_at`, which the 3-way merge tie-breaks on,
  so it stays. `fanta.json` no longer carries a per-save timestamp and has
  dropped out of the diff. A project written by an older build loses that line
  from `fanta.json` the first time it is saved; one-time, and it still opens.
- **A rectangle gains one attribute the first time a project is reloaded.**
  The loader backfills a vector's SVG viewport, so `<Rect width height />`
  becomes `<Rect width height local_size={[w, h]} />` on the first save after a
  reopen — one line per rectangle, once, and it stays a readable `<Rect>`
  thereafter. (A viewport that genuinely crops the shape has no `<Rect>`
  spelling and still prints as a canonical `<Vector>`; that is correct, not a
  regression.)
- Export lives in the inspector, not the toolbar, and needs a project root, so
  the project has to have been written at least once. Files land in
  `<project>/exports/`.
- The Code tab is read-only by design. Edit `.fnx` in your own editor and the
  canvas follows the file. Note that with autosave on, `page.fnx`'s mtime moves
  about a second after any canvas edit — an unchanged mtime is no longer a
  useful check for anything.
- A design tab is restored on relaunch through its project folder; that was
  verified. Restoring a `.fig` that never materialised a project was not
  verified — if it does not come back, reopen it from File > Open Recent.
- Opening a very large community `.fig` (100 MB and up) is slow. A 9.6 MB file
  took roughly 30–40 s to reach a rendered canvas on a debug build.
- **Memory has not been measured on a release build.** The debug build sat at
  5.1–6.3 GiB resident with one 9.6 MB `.fig` open. A debug build says nothing
  about the DMG, but nobody has taken the release number either.

## Agents

- The agent needs a project open before it will chat. Opening a `.fig` or a
  Fanta project satisfies this.
- The managed Fanta provider requires signing in. Without it, set an Anthropic
  API key in the agent panel's settings; that is the alpha's default path.
- **`read_fnx_source` returns a slice, not a whole file.** It stops at 65,536
  bytes by default and reports `total_lines`, `total_bytes`, `next_offset` and a
  `notice` saying the response is truncated and how to continue; `offset`,
  `limit` and `max_bytes` (ceiling 1 MiB) ask for a different slice. The page of
  a 9.6 MB `.fig` that used to come back as 43,145,940 characters in one block
  is now 60,719. On a document that size, paging it is still the wrong move —
  use `get_editor_state` and `batch_get` for structure. `batch_get` with no
  `ids` now defaults to `depth: 2` rather than walking the whole scene, and any
  result over 256 KiB is refused with a message naming its size.
- `get_screenshot` is still unbounded: `max_dimension` is clamped at 4096, and a
  dense page at that size can produce several MB of base64. The default of 1024
  measured 64,856 characters and is safe; pass a large `max_dimension` only
  deliberately.
- Nothing removes the MCP discovery file
  (`~/Library/Application Support/Fanta/fanta_live_mcp.json`) when the app is
  killed, so it can name a dead process. The stdio bridge checks the recorded
  pid before trusting it, but a third-party client reading that file directly
  will not.

## Version control

- **Without Xcode Command Line Tools, a new project is not a git repository.**
  The first save shells out to `git init` on `PATH`
  (`document.rs::git_init_if_needed`). On a Mac with no developer tools, that
  invocation can pop the *"The git command requires the command line developer
  tools"* dialog, and the failure is logged and swallowed on purpose — a save
  must never fail because version control is unavailable. Result: the design
  saves, but there is no `.git`, so Review Changes has nothing to show and the
  headline "every canvas edit is a reviewable diff" quietly does not apply.
  Install the tools (`xcode-select --install`) or run `git init` in the project
  folder yourself.
- A project created inside an existing repository is deliberately not given its
  own nested `.git`; its changes show up in the repository you already have.

## Network and privacy

- **Telemetry is off in the default settings.** `assets/settings/default.json`
  sets `telemetry.diagnostics`, `telemetry.metrics` and
  `telemetry.anthropic_retention` to `false`. That is a statement about the
  default settings, not a claim that nothing leaves the machine — no one has
  audited every outbound path in this build.
- **Font downloads reach GitHub.** A document using a font you do not have
  installed triggers a fetch from `raw.githubusercontent.com`
  (the `google/fonts` repository) and, for Source Sans/Serif Pro, from
  `github.com/adobe-fonts/…/releases/download/…`
  (`crates/fanta-text/src/font_resolver/download.rs`). Faces are cached under
  `~/.cache/fanta/fonts` and not re-fetched.
- **The Node download path is still compiled in.** `node_runtime` is still
  linked into the binary (via `dap` → `editor`) and it downloads from
  `nodejs.org/dist/…`. Nothing was observed reaching it during the rehearsal,
  but a strict network allow-list must account for it.
- Because of the two entries above, a network test that asserts "nothing from
  github.com" is wrong. SMOKE.md step 9 has been corrected accordingly.

## Leftovers from the Zed fork

- **Zed branding was removed from the surfaces the rehearsal found, and there
  are leftovers it did not.** Gone: the agent panel's *"Try Zed Pro for Free"*
  upsell (the whole plan-marketing panel was removed — the panel now says
  *"Connect a Model"* and points at the API-key card), the *"Message the Zed
  Agent"* composer placeholder, and the `Zed:` / `(Taylor's Version)`
  `--system-specs` line, which now reads `Fanta: v0.1.0-alpha.1+stable.<sha>`.
  `fanta --help` no longer contains the string "Zed" at all. Still there:
  `workspace::OpenTerminal` / `OpenInTerminal` in the palette (nobody has
  clicked them to find out what they do); the agent panel renders
  *"Authenticate to Zed Agent"* if the native agent ever asks to authenticate,
  because that label falls back to the raw agent id; and
  `crates/agent_ui/src/ui/end_trial_upsell.rs` still sells a *"Fanta Pro"*
  subscription that does not exist, linking to Zed's billing page. Identifiers,
  the `zed://` URL scheme, `ZED_*` env vars and action namespaces are unchanged
  on purpose.
- **The command-palette leaks the rehearsal found are closed.** `terminal` was
  added to `HIDDEN_NAMESPACES` and `workspace::NewFileSplit{,Horizontal,Vertical}`
  plus `pane::RevealInProjectPanel` to `hide_action_types`
  (`crates/zed/src/main.rs`). Re-checked against `--dump-all-actions` on the
  rebuilt binary: "vim", "project panel", "new file" and "debugger" all return
  nothing. A raw subsequence search for `vim` still fuzzy-matches unrelated
  names such as "editor: find previous match" — matcher noise, not a dead
  action.
- **Forty-nine palette entries are still prefixed `zed:`.** The prefix is the
  action namespace, which was deliberately not renamed for the alpha (renaming
  namespaces breaks keymaps and every `zed::` action path in the tree). Counted
  by filtering `fanta --dump-all-actions` through the shipped palette filter:
  936 of 1,031 actions are visible and 49 of those read `zed: …` — including
  `zed: about`, `zed: quit`, `zed: open settings`, `zed: get merch`,
  `zed: open zed url`, `zed: reset database`, and
  `zed: copy system specs into clipboard` (the entry `fanta --help` used to
  point at). `zed: test crash` and `zed: test panic` are listed but inert: their
  handlers are only registered behind the server-side `panic` feature flag,
  which is off. This is the largest remaining branding leak and it is a
  deliberate deferral, not an oversight.
- One `ERROR … agent_ui/src/message_editor.rs:601 language not found` is logged
  once per launch — the agent composer asking for a Markdown grammar that is no
  longer registered. Cosmetic, and it pre-dates this work.
- One `TCP 127.0.0.1:<port> (LISTEN)` socket stays open for the life of the
  process. The port matches the stable-channel single-instance handshake
  (`crates/zed/src/zed/mac_only_instance.rs`).
- Panic backtraces in the shipped build are **bare addresses**: `script/bundle-mac`
  strips the binary. Symbolicate with the `fanta.dwarf` produced by the same
  build:

  ```
  atos -o fanta.dwarf -arch arm64 -l <load address> <address>
  ```

- "Check for Updates" does not error any more. `script/bundle-mac` exports
  `ZED_UPDATE_EXPLANATION`, so the check shows an info prompt — *"Fanta alpha
  builds do not update themselves"* / *"Alpha builds are updated by downloading
  a new DMG from Fanta."* — and the hourly poll of the Zed release endpoint is
  suppressed.

## Platform

- Apple Silicon only. `script/bundle-mac` builds for the host triple and this
  alpha is built on `aarch64-apple-darwin`; no Intel DMG is produced.
- macOS 14 or later. Windows and Linux are not supported in this alpha.

## Test suites

- The `sidebar` (8 failures) and `agent_ui` (14 failures plus one hang) test
  suites are red. These pre-date the alpha work and sit off every path it
  touched; the `agent_ui` failures were confirmed pre-existing by backtrace.
