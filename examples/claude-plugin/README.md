# Fanta plugin for Claude Code (demo)

An installable Claude Code plugin that gives the agent Fanta's live design
tools — `get_editor_state`, `batch_get`, `batch_design`, `get_screenshot`
and the rest — against the running app, launching it first when it is not
running. The manual (non-plugin) setup for Claude Code and Codex lives in
[`../agent-plugin`](../agent-plugin); the packaged, versioned plugin is
spec 0006.

## Install

From a `fanta-edit` checkout, inside Claude Code:

```
/plugin marketplace add /path/to/fanta-edit
/plugin install fanta-canvas@fanta
```

If the Fanta app is already running, that is all: the plugin attaches to
its live MCP socket. To let the plugin launch the app itself, make the
binary findable before starting Claude Code:

```sh
export FANTA_BIN=/path/to/fanta-edit/target/release/fanta   # or put `fanta` on PATH
```

## Try it

With the app open on a project, ask Claude to, for example, "add a
rectangle named hello to the open Fanta page and screenshot it". The edit
autosaves into the project's SVG/CSS/JSON sources as a git diff.

## How it works

The app advertises its live MCP server (a Unix socket speaking
newline-delimited JSON-RPC) in `<data-dir>/fanta_live_mcp.json`.
[`fanta-bridge.py`](fanta-bridge.py) attaches to that socket and pumps MCP
stdio bytes straight through; when no live server is up it serves the
project headlessly with `fanta-mcp` (the same 14 tools, built with
`cargo build -p fanta-mcp`, no app needed), and only launches the app
when no headless server is available either. The bundled `fanta-canvas`
skill teaches the agent the edit loop. Linux and macOS only for now.

Codex users can skip the bridge's app paths entirely: point
`[mcp_servers.fanta]` at `fanta-mcp <project-dir>` for a headless-only
setup.

| Variable | Meaning |
| --- | --- |
| `FANTA_BIN` | the fanta binary for launching; default is `fanta` on `PATH` |
| `FANTA_MCP_BIN` | the headless server; default is `fanta-mcp` on `PATH` |
| `FANTA_PROJECT` | the project directory; default is the working directory when it holds a `fanta.json` |
| `FANTA_USER_DATA_DIR` | isolated profile, used to find the discovery file |
| `FANTA_LAUNCH_TIMEOUT` | seconds to wait for the live server (default 60) |
