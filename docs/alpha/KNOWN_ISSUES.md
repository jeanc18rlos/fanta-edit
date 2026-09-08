# Known issues, Fanta 0.1.0-alpha.1

Written from the code and from the alpha rehearsal, not from wishful thinking.
Every entry is something a tester can hit on purpose. Where a claim comes from
reading source rather than from running the app, it says so. Re-checked entry by
entry against the tree at `16309b2`, by grepping for the thing each one claims;
entries a fix pass closed have been deleted rather than reworded, and entries
that only a human at the keyboard could confirm are labelled as source readings.

A release build was made from this commit and driven afterwards, so the entries
about the shipped CLI, the MCP loop and memory are measurements rather than
readings. Everything reachable only by a mouse click is still a source reading:
nobody has clicked a menu in this build. Numbers carried over from an earlier
pass say which pass measured them.

[`REHEARSAL.md`](REHEARSAL.md) is the committed record of what was and was not
actually driven. Nothing here claims more than that record supports.

## First launch

- **The DMG is ad-hoc signed**, so macOS refuses to open it until you go through
  System Settings > Privacy & Security > **Open Anyway**, or run
  `xattr -dr com.apple.quarantine /Applications/Fanta.app`. The full sequence is
  in [`RELEASE_NOTES.md`](RELEASE_NOTES.md). This is not a bug you can work
  around by re-downloading.
- **A folder that is not a design project still raises a Restricted Mode
  prompt.** Design projects normally do not: `auto_trust_design_projects`
  (`crates/zed/src/zed.rs:453`) trusts a lone `.fig`/`.fnx` file, and trusts a
  directory only when **all** of these hold — it has `fanta.json` at its root,
  it has no `.zed` directory, and `git_repository_cannot_execute_anything`
  finds no `.git` at all, **or** a `.git` that is a genuine directory with no
  hooks beyond git's own `.sample` files and a config every line of which is on
  a short **allowlist** of keys known to be inert (`git_setting_is_inert`:
  a handful of `core`, `remote`, `branch`, `pull`, `push`, `fetch`, `gc`,
  `init`, `user` and `submodule` keys, with `remote.url` additionally rejected
  when it starts `ext::`). It is an allowlist rather than a list of dangerous
  keys because the dangerous set is not even bounded by the file — `include.path`
  and `includeIf` pull settings in from anywhere else on disk. So an ordinary
  config key that nobody thought to list, an include directive, or any line that
  is not a plain `key = value` all mean "not understood", and the user is asked.
  Every other uncertainty — an unreadable config, a `.git` file or symlink
  pointing elsewhere, an unexpected hook — falls back to asking too.
  Open anything else — a plain git checkout, a downloads folder — and you get
  the inherited blocking *"Unrecognized Project"* dialog, in Zed's wording,
  which **cannot be dismissed with Escape**; the title bar then reads
  *Restricted Mode* for the rest of the session.
  `"trust_all_worktrees"` is deliberately still `false`
  (`assets/settings/default.json:2565`): a worktree's own `.zed/settings.json`
  can name a context-server command that this build will spawn, and trusting a
  worktree also un-suppresses git hooks, `diff.external` and credential helpers.
  Flipping it globally would have been a real security regression, so the guard
  is narrow on purpose.
- **A design project you received as a zip or a copied folder can still prompt,
  and so can one whose git config you customised.** That is the guard above
  doing its job, not a bug: a copied `.git` carries its own hooks and config,
  and those run commands. A plain `git init` or `git clone` config is entirely
  on the allowlist, so the common cases are silent — but a config key the
  allowlist does not name, however harmless, is treated as "not understood" and
  you get the prompt. If a shared project prompts, either answer the prompt
  yourself or delete the copied `.git` and re-`git init` it.
- **Open Project Settings is hidden from the command palette on purpose.**
  `zed::OpenProjectSettingsFile` and `zed_actions::OpenProjectSettings` are in
  `hide_action_types` (`crates/zed/src/main.rs`) because both write a
  `.zed/settings.json` into the open design, which permanently disqualifies it
  from the auto-trust above and leaves you facing the unescapable prompt on
  every later open. There is no in-app way to set project-local settings for a
  design; use the user settings file.
- Launching a second `fanta <dir>` while the app is running prints
  `Fanta is already running` and exits without opening the project
  (`crates/zed/src/main.rs:390`; the single-instance check is live because
  `crates/zed/RELEASE_CHANNEL` is `stable`). Open the folder from the running
  app instead.
- **`fanta` with no arguments restores your last session, so you may never see
  the welcome page again.** The rehearsal hit this: after a project had been
  opened once, a bare launch reopened it rather than showing
  *"Your design is code."* and its **Connect Claude Code / Codex** entry. If you
  are looking for the Copy command / Copy Codex config buttons, use
  **File > New Window**. The welcome page has never been confirmed on screen at
  all — see [`REHEARSAL.md`](REHEARSAL.md) row 1 — so its rendered layout is a
  source reading only.

## Canvas and tools

- **Unwired faces stay visible and tell you so when clicked.** The toolbar and
  the layers/pages panels come from the vendored `fanta-gpui` crate, which
  exposes no host-side API to hide or disable a control — the comment above the
  `tool_kind` match in `view.rs::handle_toolbar_action` records this
  ("the vendored toolbar has no host-side API to hide a tool"). Hiding them would
  have meant patching a vendored crate on the eve of the alpha, so that was
  **deliberately deferred**; instead
  the host declines the action and raises a notice reading
  *"&lt;control&gt; is not available in the Fanta alpha yet."*
  (`view.rs::notify_unavailable`). You will meet it on the **Scale**, **Path
  Selection** and **Text-on-Path** tools (they have a tool object but no
  behaviour, so arming them would silently swallow every drag), on **Dev mode**,
  on the toolbar's **file-attach** and **voice** buttons, on **auto-keyframe
  recording**, on **"mark ready for dev"**, on **time-anchored comments**, on
  **page duplication** and **page links**, and on a number of inspector
  controls.
- **There is no Group or Ungroup.** Both are drawn — in the toolbar
  (`fanta-gpui/src/organisms/toolbar/model.rs`, with a `⇧ ⌘ G` hint) and in the
  layers context menu (`organisms/layers/model.rs`, `⇧⌘G`) — because the
  vendored panels draw them. `grep -rni ungroup crates/fig_viewer` returns
  **nothing**: neither is implemented on the host side at all, so both fall to
  the `other => notify_unavailable(…)` arm and raise the notice above.
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
  refused with a message — *"cross-document canvas paste is not supported yet;
  assets, components, and variables were left unchanged"*
  (`clipboard.rs:130`). Pasting an **image or a file from another app** onto the
  canvas does nothing at all and says nothing — the paste handler matches
  `ClipboardEntry::Image` and `ClipboardEntry::ExternalPaths` to `None` and
  drops them (`view.rs:2712`).
- **A `.fig` with no project folder does not autosave.** Autosave requires a
  project root — `autosave_allowed` checks `item.project_root().is_some()` —
  because the write that creates one scaffolds a whole directory next to the
  file, and a timer should not do that behind your back. In normal use you will
  not meet this: opening a `.fig` **materialises the project directory on open**
  (`document.rs`, the `None =>` arm of the load match), so the editor is
  project-backed from the first frame. You only fall into the in-memory mode
  when that write **fails** — a full disk, a read-only folder — which logs
  `materializing Fanta project at … on open failed` and removes the partial
  directory. If you see a `.fig` whose edits are not reaching disk, check the
  log for that line, then press `cmd-s` to retry the materialisation.
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
- Export runs in the inspector and needs a project root, so the project has to
  have been written at least once. Files land in `<project>/exports/`
  (`export.rs:260`), in PNG, JPG, SVG or PDF (`ExportFormat`). The toolbar's
  Export command is **not** a second flow: it forces the inspector sidebar open
  and defers to the same `export_selection`
  (`view.rs::export_from_toolbar`), because every failure it can hit — unsaved
  project, unexportable bounds — is reported as inspector feedback and would
  otherwise look like a click that did nothing.
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
- **Memory is high, and the first edit costs about 2.3 GiB that is not given
  back.** Measured on the release build with one 9.6 MB, 29,301-node `.fig`:
  importing it settles at 4.3 GiB resident, reopening the saved project settles
  at 5.0 GiB, and the first edit-plus-autosave peaks at 7.6 GiB and stays near
  7.4 GiB. The cost was isolated to the autosave write path — a screenshot adds
  96 MiB and reading the source adds 41 MiB — and it is a one-time high-water
  mark rather than a leak: four further edits stayed flat. On a 36 GiB machine
  with a file this size that is survivable. Nobody has opened the 128 MB
  community UI kit that the smoke checklist's timing row calls for, and that is
  where this would bite.

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
  The first save shells out to `git init -q` with a bare `git` resolved through
  `PATH` (`document.rs:1787`, `git_init_if_needed`). On a Mac with no developer
  tools, `/usr/bin/git` is the shim that raises the *"The git command requires
  the command line developer tools"* dialog and exits non-zero; the failure is
  then logged as a warning and swallowed on purpose — a save must never fail
  because version control is unavailable. Result: **the design saves, but there
  is no `.git`**, so Review Changes has nothing to show and the headline "every
  canvas edit is a reviewable diff" quietly does not apply. Note this path does
  **not** use the `git` binary `script/bundle-mac` ships inside the app
  (`Contents/MacOS/git`) — that one backs the git *UI*
  (`git::repository`'s `bundled_git_binary_path`), not this `init`. Install the
  tools (`xcode-select --install`) or run `git init` in the project folder
  yourself.
- A project created inside an existing repository is deliberately not given its
  own nested `.git` — `git_init_if_needed` walks the ancestors — so its changes
  show up in the repository you already have.
- **Git-initialisation has only ever been observed on the `.fig` import path.**
  The rehearsal materialised its project by opening a `.fig` and saw `.git`
  appear. `File > New Design…` reaches the same `write_project` and therefore the
  same `git_init_if_needed`, but that menu path has never been driven — it is a
  source reading, not an observation. [`SMOKE.md`](SMOKE.md) row 2 is the row
  that would settle it.

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
  breaks keymaps and every `zed::` action path in the tree. Derived by taking the
  1,031-action registry from `fanta --dump-all-actions` and applying
  `HIDDEN_NAMESPACES` and `hide_action_types` as they stand in
  `crates/zed/src/main.rs`: **929 of 1,031 actions stay visible, and 44 of those
  are in the `zed` namespace** — including `zed: about`, `zed: quit`,
  `zed: open settings`, `zed: open zed url`, `zed: reset database` and
  `zed: copy system specs into clipboard`. `zed: test crash` and
  `zed: test panic` are listed but their handlers are only registered behind the
  server-side `PanicFeatureFlag` (`crates/zed/src/zed.rs:200`), so they do
  nothing unless that flag is on. This is the largest remaining branding leak
  and it is a deliberate deferral, not an oversight. Two caveats: this is a
  filter-list derivation, not the palette as rendered — nobody has typed into
  the palette in a running build — and the 1,031-action registry came from a
  binary built at `493e364`, which no longer exists in this tree, so the counts
  are carried over rather than current. The filter lists themselves were
  re-read at `16309b2`.
- **Searching the palette for "terminal" returns one entry, and it works.**
  `agent: new terminal thread` (`agent_ui::NewTerminalThread`) survives the
  filter because `terminal_view` is still linked into the app: it is how an
  external ACP agent runs. The `terminal` and `terminal_panel` namespaces are
  otherwise hidden, so nothing else matches. Searches for "vim", "debugger",
  "project panel" and "new file" return **zero** visible actions by the same
  derivation.
- **Two wrong strings survive in the agent panel, in surfaces the rehearsal did
  not reach.** `crates/agent_ui/src/ui/end_trial_upsell.rs` offers *"Upgrade to
  Fanta Pro"* and *"Your Fanta Pro Trial has expired"* — Fanta-branded, but for
  a subscription that does not exist. And `conversation_view.rs:2318` renders
  `format!("Authenticate to {}", agent_display_name)`, where
  `agent_display_name` falls back to the raw agent id
  (`conversation_view.rs:1393`); the built-in agent's id is the literal string
  `"Zed Agent"` (`crates/agent/src/agent.rs:2558`,
  `ZED_AGENT_ID = AgentId::new("Zed Agent")`), so that callout can read
  *"Authenticate to Zed Agent"*. The rest of the CLI and the agent composer were
  de-branded in the fix pass — `--user-data-dir` now documents
  `~/Library/Application Support/Fanta`, and there is no "Try Zed Pro for Free"
  or "Message the Zed Agent" string left in `crates/agent_ui`. Identifiers, the
  `zed://` URL scheme, `ZED_*` env vars and action namespaces are unchanged on
  purpose.
- **This build never updates itself.** `script/bundle-mac` exports
  `ZED_UPDATE_EXPLANATION='Alpha builds are updated by downloading a new DMG
  from Fanta.'`, so "Check for Updates" shows that as an information prompt
  instead of erroring, and the poll of the Zed release endpoint is suppressed.
  Get new alpha builds by downloading a new DMG.
- One `ERROR … agent_ui/src/message_editor.rs:601 language not found` is logged
  once per launch — the agent composer asking for a Markdown grammar that this
  build no longer registers. **Cosmetic**, and it pre-dates this work.
- A second ERROR can appear under load and is also **cosmetic**:
  `gpui_macos::metal_renderer failed to render: scene too large: … retrying
  with larger instance buffer size`. It says what it does — the renderer grows
  its instance buffer and draws the frame. Re-counted just now, the current
  `~/Library/Logs/Fanta/Fanta.log` (103 KB, five launches) holds **13 ERROR
  lines: twelve `language not found` and one `scene too large`**, zero
  `didn't find an action` lines, and no `panic` match beyond
  `panic handler registered`. That log was written by the older binary and
  pre-dates `16309b2`.
- Panic backtraces in the shipped build are **bare addresses**, because
  `script/bundle-mac` runs `/usr/bin/strip` over `Contents/MacOS/fanta` (and
  `cli`) before signing. The same script runs `dsymutil --flat` first, so the
  build leaves `target/<triple>/release/fanta.dwarf` next to the DMG — outside
  the `dmg/` staging directory, so it is never packaged and has to be kept by
  hand. Symbolicate with:

  ```
  atos -o fanta.dwarf -arch arm64 -l <load address> <address>
  ```

  For a `-d` (debug) bundle `dsymutil` is skipped entirely and there is no
  `fanta.dwarf` at all.

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
  — they are carried over from the pass that measured them, and the numbers may
  have drifted. Treat them as "these two suites are known red", not as a
  current count. Re-running them was skipped again for this pass: there is no
  built binary in `target/` at all, so `cargo test -p sidebar -p agent_ui` would
  mean a cold compile of the workspace, and the disk headroom on this machine
  has swung between 15 and 45 GiB all day. Both crates still exist
  (`crates/sidebar`, `crates/agent_ui`); nothing in this alpha's work touches
  them.
