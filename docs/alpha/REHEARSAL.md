# Alpha smoke-test rehearsal

A dry run of [`SMOKE.md`](SMOKE.md) plus the wave-2 additions, on a **debug**
build, driven from a terminal.

| | |
|---|---|
| Commit | `6e14229` (tree clean at build time) |
| Binary | `/Users/jeanrojas/fanta-edit/target/debug/fanta`, `cargo build -p zed`, 24s incremental relink of `cli` / `zed` / `fig_viewer` |
| Version | `fanta 0.1.0-alpha.1`; `--system-specs` reports `v0.1.0-alpha.1+stable.6e14229…` — release channel **stable**, not dev |
| Project under test | `/tmp/fanta-smoke/Smoke`, materialised by opening a copy of `~/Desktop/basic.fig` (9.6 MB, 29,304 nodes) |
| Disk | 27 GiB free of 694 GiB before and after; no release build attempted |
| New script | [`script/smoke-mcp`](../../script/smoke-mcp) |

## What could not be driven, and why

Everything behind a mouse click is **NOT VERIFIED**. This machine allowed
`screencapture`, which produced two useful live frames of the app; on the third
call macOS raised a *"Claude is requesting to bypass the system private window
picker"* permission dialog. Granting that is a system security setting, so it
was left alone — and screen capture has returned stale frames ever since. **That
dialog is still on screen and needs dismissing by hand.** No menu was opened, no
button was clicked, and no key was sent to the app during this rehearsal.

Where a row could not be exercised, the source-level fact is recorded next to it
so a human tester knows what to expect — but a source reading is not a pass.

## `script/smoke-mcp`

Takes a binary and a project directory, reuses a running app or launches one on
that project, speaks MCP through `fanta --mcp-stdio` (falling back to the unix
socket named in `~/Library/Application Support/Fanta/fanta_live_mcp.json`), and
asserts the loop end to end. `python3 -m py_compile script/smoke-mcp` passes.

| Run | Command | Result |
|---|---|---|
| Reuse path, stdio | `script/smoke-mcp ./target/debug/fanta /tmp/fanta-smoke/Smoke` against an already-running app | **18/18, exit 0** |
| Cold path, stdio | same, with no app running — the script launched it and waited for the canvas | **18/18, exit 0** |
| Socket fallback | `… /tmp/fanta-smoke/Smoke --socket` | **18/18, exit 0** |
| Negative: not a repo | `… /tmp/fanta-smoke/not-a-repo` | `FAIL project is a git repository`, **exit 1** — reports it, does not `git init` over it |
| Negative: no binary | `… /nope/fanta …` | `FAIL binary exists and is executable`, **exit 1** |

The eighteen assertions, and what the cold run actually printed:

| # | Assertion | Observed |
|---|---|---|
| 1 | binary exists and is executable | `target/debug/fanta` |
| 2 | project directory exists | `/tmp/fanta-smoke/Smoke` |
| 3 | project is a git repository | toplevel `/private/tmp/fanta-smoke/Smoke` |
| 4 | app is running with the project open | started the app, transport `--mcp-stdio` |
| 5 | `initialize` returns `serverInfo.name == "fanta"` | `{"name":"fanta","title":"Fanta","version":"0.1.0"}`, protocol `2025-06-18` |
| 6 | `ping` returns an empty object | `{}` |
| 7 | `tools/list` carries the five canvas tools | `batch_design, batch_get, get_editor_state, get_screenshot, read_fnx_source` |
| 8 | `get_editor_state` reports the project root | `/private/tmp/fanta-smoke/Smoke` |
| 9 | `get_editor_state` marks one page active | `Page 1` (index 0) |
| 10 | the active page names its source file | `pages/page-1/page.fnx` |
| 11 | the named source file exists on disk | yes |
| 12 | `read_fnx_source` returns that file's text | 43,145,940 chars |
| 13 | `batch_design` creates a rectangle | `created ['n_01M2173NBF653V6FBHQP6KWSWQ']` |
| 14 | the canvas reports unsaved work | `dirty=true` |
| 15 | **the edit reaches the `.fnx` on disk with no Cmd-S** | `page.fnx` mtime `1788894762 -> 1788894899` after 5.1s |
| 16 | **the autosaved edit shows up as a git diff** | `M pages/page-1/page.fnx`, `M pages/page-1/page.ids.json`, `M fanta.json`, `M doc/metadata.json` |
| 17 | `get_screenshot` renders the open page | 64,844 base64 chars of `image/png` |
| 18 | `batch_get` reads the node just made | `names ['smoke-mcp rectangle']` |

Assertions 15 and 16 are the product thesis, mechanically checked: an outside
process created a rectangle over MCP, nobody touched the keyboard, and about
five seconds later the change was a git diff.

## SMOKE.md, row by row

| Row | What was checked | Outcome | Evidence / reason |
|---|---|---|---|
| 1 Launch | Welcome page copy, Get Started entries, rail, menus, no untitled tab | **NOT VERIFIED** | The welcome page was never seen: a bare `fanta` with a saved session *restores the project* instead. What a live frame did show on the project window: menu bar `fanta / File / Edit / View / Window / Help`, `Sign In` at the right of the title bar, pages + layers rail on the left, agent panel on the right, canvas tab, no untitled editor tab, no status bar. Source (`crates/workspace/src/welcome.rs`) has `HERO_HEADLINE = "Your design is code."`, exactly three Get Started entries (New Design… / Open .fig or Fanta project… / Connect Claude Code / Codex), a Configure section with Open Settings, and **no** Command Palette entry. |
| 1a Copy command / Copy Codex config | Clipboard contents | **NOT VERIFIED** | Needs a human to click the two buttons. Source emits `claude mcp add -s user fanta -- <exe> --mcp-stdio` and the `[mcp_servers.fanta]` Codex hint from `crates/fig_viewer/src/live_mcp.rs`. |
| 1b Empty pane / double-click | Welcome page in an empty pane, New Design prompt, never an untitled buffer | **NOT VERIFIED** | Needs clicks. |
| 1c `+` tab-bar button | Menu offers New Design… / Open… / Search Project only | **NOT VERIFIED** | Needs a click. |
| 2 File > New Design… | Canvas opens with one page, folder in the rail, agent composer | **NOT VERIFIED** (the menu path) | The save-panel path was not driven. The equivalent result was produced instead by opening a `.fig`: `Smoke/` appeared with `fanta.json`, `pages/`, `components/`, `AGENTS.md`, `.gitignore`, `fnx.d.ts` and **`.git`**, and the canvas opened on it. |
| 3 Draw a rectangle, change fill, undo/redo, cmd-s | Canvas gestures | **NOT VERIFIED** | Needs the mouse. The same operations through MCP (`create_node` rectangle with a fill, then a save to `page.fnx`) pass — see the smoke-mcp table. Undo/redo was not exercised at all. |
| 4 Agent makes it red, then screenshots | `design_edit` / `design_screenshot`, cmd-z | **PARTIAL** | The external-agent equivalent passes: `batch_design` changed the canvas and `get_screenshot` returned a valid PNG that visibly contains the edit. The in-app agent panel (needs an Anthropic key) and cmd-z were not exercised. **SMOKE.md is stale here** — the tools are named `batch_design` and `get_screenshot`, not `design_edit` / `design_screenshot`. |
| 5 Code tab; mtime; external edit; languages dir | Four separate claims | **SPLIT** | *External edit updates the canvas:* **PASS** — `page.fnx` was rewritten from bash (fill `#FF3B30`→`#00C853`, 120×80→900×600, renamed "edited outside the app"); within seconds `batch_get` reported the new name, `get_screenshot` rendered a large green rectangle that was not there before, the layers rail showed the new name, and `dirty` stayed `false` (a reload, not an unsaved change). *Code tab renders highlighted / typing does nothing:* **NOT VERIFIED**, needs the GUI. *mtime unchanged:* **NOT VERIFIED**, and the claim now conflicts with wave-2 autosave — any canvas edit moves the mtime within ~1s. *`languages/` stays empty:* **PARTIAL** — this profile already held five entries from 18 July; nothing new was written during the rehearsal (directory mtime unchanged). |
| 6 Toolbar tools, AI box, Export PNG | Every visible tool | **NOT VERIFIED** | Needs clicks. |
| 7 Quit and relaunch | Project back in the rail, canvas reopens | **PASS** | Killed the app, relaunched with no arguments: the log shows `opening git repository at "/private/tmp/fanta-smoke/Smoke/.git"`, and `get_editor_state` on the new process reported project `Smoke`, three pages, `Page 1` active with 29,301 nodes, `is_editable: true`, and the external edit still present. |
| 8 Open a `.fig`, then open the folder | `.fig` renders and materialises a sibling project; folder opens onto the canvas | **PASS** (via the CLI, not the File > Open… dialog) | `fanta /tmp/fanta-smoke/Smoke.fig` rendered the document and created `/tmp/fanta-smoke/Smoke/` beside it, git-initialised. `fanta /tmp/fanta-smoke/Smoke` (the script's cold path) opened straight onto the canvas. The dialog itself was not used. |
| 9 `nettop` over five idle minutes plus an agent turn | Only the three allowed hosts | **PARTIAL** | Over ~1 idle minute `nettop -P -x -p <pid>` reported `0` bytes in and `0` bytes out, and `lsof -nP -i` showed **no outbound connections at all** — only one listener, `TCP 127.0.0.1:44438 (LISTEN)`. Nothing reached zed.dev, github.com or npmjs.org in that window. The full five minutes and the agent turn (needs an API key) were not run. |
| 10 `Fanta.log` | No panics, no `didn't find an action` | **PASS** | Across five launches: `didn't find an action` count **0**; the only "panic" matches are `INFO [crashes] panic handler registered`. The one ERROR is the known cosmetic `agent_ui/src/message_editor.rs:601 language not found`, once per launch — pre-existing, unrelated to this work. |
| Timing note | 128 MB community UI kit | **NOT RUN** | Data point instead: the 9.6 MB `basic.fig` took roughly 30–40s to reach a rendered canvas on this debug build, and the process settled at **5.1–6.3 GiB resident**. |

## Wave-2 additions

| Row | What was checked | Outcome | Evidence / reason |
|---|---|---|---|
| Welcome page says "Your design is code.", has a Connect entry and no Command Palette entry | | **NOT VERIFIED** | Never rendered on screen (see row 1). `crates/workspace/src/welcome.rs` contains all three facts. |
| Fanta > Settings > Open Settings File works from the welcome page | | **NOT VERIFIED** | Needs a menu click. `zed::OpenSettingsFile` exists and is bound to `cmd-alt-,`; the welcome page has an Open Settings entry. |
| Command palette shows nothing for "vim" | | **NOT VERIFIED** at runtime; **likely FAIL** | `--dump-all-actions` (1,031 actions) filtered through `HIDDEN_NAMESPACES` in `crates/zed/src/main.rs` leaves `terminal::ToggleViMode` ("terminal: toggle vi mode") visible, which a fuzzy search for `vim` matches. The `vim` namespace itself is hidden and only `vim::OpenDefaultKeymap` survives in the binary. |
| Command palette shows nothing for "debugger" | | **PASS by action dump**, not clicked | All five `debugger::*` actions fall in a hidden namespace; zero visible actions match. |
| Command palette shows nothing for "project panel" | | **FAIL by action dump**, not clicked | `pane::RevealInProjectPanel` ("pane: reveal in project panel") is in the visible `pane` namespace and is not in `hide_action_types`. |
| Command palette shows nothing for "new file" | | **FAIL by action dump**, not clicked | Only `workspace::NewFile` is hidden. `workspace::NewFileSplit`, `workspace::NewFileSplitHorizontal` and `workspace::NewFileSplitVertical` are all still visible. |
| File > New Window shows the welcome page, not an untitled buffer | | **NOT VERIFIED** | Needs a menu click. Related observation: launching `fanta` with no arguments restores the last session rather than showing the welcome page. |
| File > Review Changes opens the project diff | | **NOT VERIFIED** | Needs a menu click. |
| Edit menu Undo works on the canvas | | **NOT VERIFIED** | Needs a menu click. `cmd-z` → `fig_viewer::Undo` is bound in the `FigViewer && !Editor` context. |
| `cmd-a` selects all | | **NOT VERIFIED** | Needs a key press. `cmd-a` → `fig_viewer::SelectAll` is bound in that context. |
| `cmd-0` shows 100% | | **NOT VERIFIED** | Needs a key press. `cmd-0` → `fig_viewer::ResetZoom` is bound; the zoom control was visible in a live frame reading `10%`. |
| Code tab shows its file path and follows the canvas selection | | **NOT VERIFIED** | Needs the GUI. |
| Editing a `page.fnx` in another editor toasts on the canvas and updates it | | **PASS for the update, NOT VERIFIED for the toast** | See row 5. The document reloaded and the layers rail re-rendered with the new layer name; whether a notice appeared could not be observed (transient, and the frames available were covered by the trust modal). |

## Must fix before the DMG ships

Ordered by severity.

1. **The Restricted Mode modal is the first thing a tester sees, and it is
   Zed's.** Opening `/private/tmp/fanta-smoke/Smoke` raised a blocking
   *"Unrecognized Project"* dialog — *"Untrusted projects are opened in
   Restricted Mode to protect your system. Review .zed/settings.json for any
   extensions or commands configured by this project"*, listing "Language
   servers from running" and "MCP Server integrations from installing", with
   *Stay in Restricted Mode* / *Trust and Continue*. `SecurityModal::on_before_dismiss`
   refuses Escape, so it must be answered. The title bar then reads
   `⚠ Restricted Mode` for the session. Every project a designer creates will
   hit this. One-line fix: `"trust_all_worktrees": true` at
   `assets/settings/default.json:2559`. (The live MCP server is unaffected — the
   whole 18/18 run above happened with this modal on screen.)

2. **Zed branding is visible in shipped surfaces.** Observed on screen: the
   agent panel offers **"Try Zed Pro for Free"** and its composer placeholder
   reads **"Message the Zed Agent, @ to include context"**. Observed on the CLI:
   `fanta --help` documents `--user-data-dir` as defaulting to
   `~/Library/Application Support/**Zed**` (wrong as well as off-brand — it is
   `…/Fanta`), plus "Instructs **zed** to run as a dev server", "prevents
   **Zed** from starting" and "**zed**: copy system specs to clipboard".
   `fanta --system-specs` prints `**Zed**: v0.1.0-alpha.1+stable.6e14229 (Fanta)
   (Taylor's Version)`. `main.rs:390` prints `"zed is already running"` when a
   second instance is launched — reachable, since the channel is **stable** and
   the single-instance check is live.

3. **The command palette still lists actions that do nothing.** From the action
   dump: `workspace::NewFileSplit{,Horizontal,Vertical}` for "new file",
   `pane::RevealInProjectPanel` for "project panel", and `terminal::*` (which a
   search for "vim" fuzzy-matches through "toggle vi mode"). Add them to
   `hide_action_types` / `HIDDEN_NAMESPACES` in
   `crates/zed/src/main.rs:1434`. The project panel is unlinked, so
   "Reveal In Project Panel" cannot do anything at all.

4. **The first save after importing a `.fig` rewrites the entire page.** One
   MCP-created rectangle produced `25,311 insertions, 25,310 deletions` in
   `pages/page-1/page.fnx`. The cause is float formatting, not content: the
   materialise-on-open writer emits `21.762165069580078` where the
   save-from-memory writer emits `21.76216506958008`. Every subsequent edit is
   clean (**1 line added** for the second rectangle, measured), so this is a
   one-time mismatch between two write paths — but it lands exactly on the
   "every canvas edit is a reviewable diff" demo, on the first edit a new user
   makes after importing.

5. **`<Rect>` does not survive a reload.** A rectangle written as
   `<Rect … width={120.0} height={80.0} />` comes back after the project is
   closed and reopened as a verbose
   `<Vector … local_size={[120.0, 80.0]} path={{"segments": …}} />`, and stays
   that way. The load path attaches `local_size`, which is in
   `SUGAR_BLOCKING_ATTRS` (`crates/fanta-fnx/src/sugar.rs:239`), so
   `sugared_form` refuses to re-spell it. Consequence: readable source degrades
   into path data across restarts, and the next save produces a spurious diff
   on every rectangle in the file.

6. **`read_fnx_source` hands an agent 43 MB in one response.** One page of this
   9.6 MB `.fig` is 43,145,940 characters, returned as a single text block with
   no cap, range or pagination. Most MCP clients will not survive that. The
   tool needs a byte cap and a way to ask for a slice.

7. **`--mcp-stdio` is hidden from `--help`.** It is registered
   `#[arg(long, hide = true)]` (`crates/zed/src/main.rs:1580`), so the flag the
   entire "Connect Claude Code / Codex" story depends on cannot be found or
   confirmed from the CLI. `script/smoke-mcp` had to probe by running it. Drop
   the `hide`.

8. **Memory needs a number from the release build.** This debug build sat at
   **5.1–6.3 GiB resident** with one 9.6 MB `.fig` open, growing ~1.5 GiB in the
   first 30 seconds. A debug build is not evidence about the DMG, but nobody has
   measured the release build, and the smoke test's timing row (a 128 MB UI kit)
   is exactly where this would bite.

9. **`SMOKE.md` is out of date in two places.** Row 4 names `design_edit` and
   `design_screenshot`; the shipped tools are `batch_design` and
   `get_screenshot`. Row 5 asserts `page.fnx`'s "mtime is unchanged", which
   wave-2's debounced autosave contradicts for any row that touches the canvas
   first.

10. **Every save touches two files nobody edited.** `fanta.json` and
    `doc/metadata.json` each get a new `modified_at`, so the minimum diff for a
    one-node change is four files rather than two. Cheap noise to remove from
    the flagship `git diff`.

### Loose ends, not blockers

- One `TCP 127.0.0.1:44438 (LISTEN)` socket is open for the life of the process.
  The port matches the single-instance handshake scheme for uid 501 on the
  stable channel (`crates/zed/src/zed/mac_only_instance.rs`), which is
  consistent with this build reporting `stable`. Worth confirming that is all it
  is before shipping.
- The `.fig` import path is what materialises and git-inits a project on open;
  `File > New Design…` was never exercised in this rehearsal, so its
  git-initialisation is still only verified by reading
  `fig_viewer::document::write_project`.
- The scratch project used here is `/tmp/fanta-smoke/Smoke`, with the rehearsal
  history committed to its own git repo (`Baseline from basic.fig` →
  `After first MCP edit` → `After second MCP edit`). Nothing in
  `fanta-edit` was committed, staged or reverted.
