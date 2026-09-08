# Fanta alpha smoke test

Run on the build machine and again on a second macOS account, downloading the
DMG through a browser so quarantine is exercised the way a tester will hit it.

Start from a clean profile:

```
rm -rf ~/Library/Application\ Support/Fanta ~/Library/Logs/Fanta
```

[`REHEARSAL.md`](REHEARSAL.md) is the record of a dry run of this checklist. It
is the reference for what has and has not been exercised before: most rows below
are marked *NOT VERIFIED* there because the rehearsal could drive nothing behind
a mouse click. Rows 0 and 5 are the ones a machine can prove on its own.

| # | Step | Pass condition |
|---|---|---|
| 0 | `script/smoke-mcp /Applications/Fanta.app/Contents/MacOS/fanta <project-dir>` | **18 of 18 assertions pass, exit 0.** This is the fastest way to prove the whole loop: handshake, `ping`, the five tools, editor state naming each page's source, `read_fnx_source`, `batch_design` creating a node, the canvas going dirty, the debounced autosave reaching disk, the resulting git diff, `get_screenshot` and `batch_get`. It reuses a running app or launches one. Add `--socket` to test the Unix-socket path instead of `--mcp-stdio`. Do this row first — if it fails, stop. |
| 1 | Launch | Welcome page in the centre reads **Your design is code.** under the logo, with the subtitle *"A Fanta project is a git-tracked folder of .fnx source. Every canvas edit is a reviewable change; every source edit reloads the canvas."* Get Started lists **New Design...** / **Open .fig or Fanta project...** / **Connect Claude Code / Codex**, and **no** Open Command Palette entry; Configure lists Open Settings. Layers/pages rail left, agent panel right, title bar with Sign In, no status bar, no untitled editor tab, no onboarding tour. Menus are Fanta / File / Edit / View / Window / Help. **Note:** launching `fanta` with no arguments *restores the last session* rather than showing the welcome page, so use a clean profile or File > New Window for this row. |
| 1a | Click **Copy command**, then `pbpaste`; click **Copy Codex config**, then `pbpaste` | The first prints `claude mcp add -s user fanta -- /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio`; the second prints the `[mcp_servers.fanta]` block for `~/.codex/config.toml`. |
| 1b | Open a project, close every design tab, then double-click the empty centre pane. Also double-click empty space in the tab bar. | The empty pane shows the welcome page, not a blank pane; each double-click opens the New Design prompt, never an untitled text buffer. |
| 1c | Click the `+` button at the right of the tab bar | The menu offers **New Design…**, **Open…** and **Search Project** only: no New File, Open File, Search Symbols or terminal entries. |
| 2 | File > New Design..., choose `~/Desktop/Smoke` | Canvas opens with one page. `Smoke/` appears in the rail and on disk holds `fanta.json`, `pages/`, `components/`, `AGENTS.md` and **`.git`**. Agent panel shows a composer, not "open a project". |
| 3 | Press `R`, drag a rectangle. Change its fill in the inspector. Edit > Undo, then redo. `cmd-a`, then `cmd-0`. | Each step is visible on the canvas; Undo and Redo work from the **Edit menu** as well as `cmd-z`; `cmd-a` selects all; `cmd-0` shows 100% in the zoom control. |
| 4 | With an Anthropic key set: ask the in-app agent to make the rectangle red, then to screenshot the page | The canvas changes and an image comes back. The tools involved are named **`batch_design`** and **`get_screenshot`** — if you are checking a tool list, those are the names, along with `get_editor_state`, `batch_get` and `read_fnx_source`. `cmd-z` undoes the agent's edit. |
| 5 | Wait ~2 s after the last canvas edit without pressing anything, then `git -C <project> status --short`. Then edit `page.fnx` in another editor and save. | **The edit is already on disk** — autosave writes about a second after you stop, so `page.fnx` is modified with no `cmd-s`. Expect three modified files for a one-node change: `pages/…/page.fnx`, `pages/…/page.ids.json`, `doc/metadata.json`. Do **not** assert an unchanged mtime anywhere in this row; autosave moves it. The external edit then updates both canvas and Code pane, and raises a *"&lt;file&gt; changed on disk — canvas updated"* notice. |
| 5a | Open the Code tab | Source renders highlighted; the tab shows its file path; selecting a node on the canvas moves the Code tab to that node's source; typing does nothing (read-only by design). `~/Library/Application Support/Fanta/languages/` gains no new entries. |
| 6 | Click every visible toolbar tool. Submit a prompt in the toolbar's AI box. Inspector > Export PNG. | Tools either draw or raise *"&lt;control&gt; is not available in the Fanta alpha yet."* — never a click that does nothing. Expect that notice on Scale, Path Selection, Text-on-Path, Dev mode, file-attach, voice, and on Group/Ungroup. The AI box opens an agent draft; the PNG lands in `<project>/exports/`. |
| 7 | Quit and relaunch | `Smoke/` is back in the rail and the canvas reopens on the same page, with any external edits still present. |
| 8 | File > Open... `~/Desktop/basic.fig`; then File > Open... the `Smoke` folder | The `.fig` renders and materialises `basic/` beside itself, git-initialised; the folder opens straight onto the canvas. Neither raises the Restricted Mode dialog. |
| 8a | File > Open... a folder that is **not** a design project (any plain git checkout) | The inherited *"Unrecognized Project"* / Restricted Mode dialog **does** appear, and cannot be dismissed with Escape. This is expected — see KNOWN_ISSUES.md. Confirm the design projects in row 8 did not raise it. |
| 9 | `nettop -P -p $(pgrep -x fanta)` over five idle minutes plus one agent turn | **Allow-list, all sourced from the tree:** `api.anthropic.com` (`crates/anthropic`), `api.fantaisa.net` (the `server_url` default in `assets/settings/default.json`, for accounts and the managed provider), `cdn.agentclientprotocol.com` (`agent_registry_store.rs`), **`raw.githubusercontent.com`** and **`github.com/adobe-fonts`** (font downloads, `fanta-text/src/font_resolver/download.rs` — `raw.githubusercontent.com` is also used for ACP registry icons), **`nodejs.org`** (`node_runtime.rs:656`) and **`registry.npmjs.org`** (`node_runtime::npm_install_packages`, reachable if you install an external ACP agent). Do **not** assert "nothing from github.com" or "nothing from npm" — both assertions are wrong for this build. `zed.dev` should not appear. Over ~1 idle minute the rehearsal saw zero bytes either way and no outbound connections at all. |
| 10 | `~/Library/Logs/Fanta/Fanta.log` | No panics. No `didn't find an action` lines. **One expected ERROR per launch:** `agent_ui/src/message_editor.rs:601 language not found`, which is cosmetic. |
| 11 | Command palette: search "vim", "debugger", "project panel", "new file", "terminal" | Nothing actionable comes back for any of them. Entries prefixed `zed:` do still appear for other searches; that is a known deferral, not a failure of this row. |
| 12 | Fanta > Check for Updates | An information prompt, not an error: *"Alpha builds are updated by downloading a new DMG from Fanta."* |

Timing-only, not a gate: open the 128 MB community UI kit and record how long it
takes and what the process settles at in Activity Monitor. Nobody has run this;
the closest data point is a 9.6 MB `.fig` taking 30–40 s on a debug build.
