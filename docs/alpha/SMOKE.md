# Fanta alpha smoke test

Run on the build machine and again on a second macOS account, downloading the
DMG through a browser so quarantine is exercised the way a tester will hit it.

Start from a clean profile:

```
rm -rf ~/Library/Application\ Support/Fanta ~/Library/Logs/Fanta
```

| # | Step | Pass condition |
|---|---|---|
| 1 | Launch | Welcome page in the centre reads **Your design is code.** under the logo, with the subtitle about a git-tracked folder of `.fnx` source. Get Started lists New Design… / Open .fig or Fanta project… / **Connect Claude Code / Codex** and **no** Open Command Palette entry; a Bring Your Own Agent card shows the `claude mcp add -s user fanta -- …--mcp-stdio` line with Copy command / Copy Codex config. Threads rail left, agent panel open right, title bar with Sign In, no status bar, no untitled editor tab, no onboarding tour. Menus are Fanta / File / Edit / View / Window / Help. |
| 1a | Click **Copy command**, then `pbpaste`; click **Copy Codex config**, then `pbpaste` | The first prints the `claude mcp add -s user fanta -- /Applications/Fanta.app/Contents/MacOS/fanta --mcp-stdio` line; the second prints the `[mcp_servers.fanta]` block for `~/.codex/config.toml`. |
| 1b | Open a project, close every design tab, then double-click the empty centre pane. Also double-click empty space in the tab bar. | The empty pane shows the welcome page, not a blank pane; each double-click opens the New Design prompt, never an untitled text buffer. |
| 1c | Click the `+` button at the right of the tab bar | The menu offers **New Design…**, **Open…** and **Search Project** only: no New File, Open File, Search Symbols or terminal entries. |
| 2 | File > New Design..., choose `~/Desktop/Smoke` | Canvas opens with one page. `Smoke/` appears in the rail. Agent panel shows a composer, not "open a project". |
| 3 | Press `R`, drag a rectangle. Change its fill in the inspector. Edit > Undo, then redo. `cmd-s`. | Each step is visible on the canvas; `Smoke/pages/*/page.fnx` exists after save. |
| 4 | With an Anthropic key set: ask the agent to make the rectangle red, then to screenshot the page | `design_edit` changes the canvas; `design_screenshot` returns an image; `cmd-z` undoes the agent's edit. |
| 5 | Open the Code tab. Note `stat -f %m` on `page.fnx` before and after. Then edit `page.fnx` in another editor and save. | Source renders highlighted; typing does nothing; **mtime is unchanged**; the external edit updates both canvas and pane. `~/Library/Application Support/Fanta/languages/` stays empty. |
| 6 | Click every visible toolbar tool. Submit a prompt in the toolbar's AI box. Inspector > Export PNG. | Tools either draw or say they are unavailable; the AI box opens an agent draft; the PNG lands in `<project>/exports/`. |
| 7 | Quit and relaunch | `Smoke/` is back in the rail and the canvas reopens. |
| 8 | File > Open... `~/Desktop/basic.fig`; then File > Open... the `Smoke` folder | The `.fig` renders and materialises `basic/` beside itself; the folder opens straight onto the canvas. |
| 9 | `nettop -P -p $(pgrep -x fanta)` over five idle minutes plus one agent turn | Only `api.anthropic.com`, `api.fantaisa.net` and `cdn.agentclientprotocol.com`. Nothing from zed.dev, github.com or registry.npmjs.org. |
| 10 | `~/Library/Logs/Fanta/Fanta.log` | No panics. No `didn't find an action` lines. |

Timing-only, not a gate: open the 128 MB community UI kit and record how long it takes.
