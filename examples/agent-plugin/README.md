# Fanta from Claude Code and Codex

A working example of the agent plugin
([spec 0006](../../specs/0006-a-fanta-plugin-for-claude-code-and-codex/spec.md)):
register one command and your agent gets Fanta's live design tools —
`get_editor_state`, `batch_get`, `batch_design`, `get_screenshot`,
`read_fnx_source`, `add_comment` and the rest — against the running app,
launching it first when it is not running.

The pieces: the app's live MCP server (Unix socket, advertised in
`<data-dir>/fanta_live_mcp.json`), the `fanta --mcp-stdio` stdio bridge
built into the binary, and [`fanta-agent-bridge`](fanta-agent-bridge), the
attach-or-launch wrapper in this directory.

## Claude Code

Prefer the installable plugin in [`../claude-plugin`](../claude-plugin); the
command below is the manual equivalent.

```sh
claude mcp add --scope project --transport stdio fanta \
  --env FANTA_BIN=/path/to/fanta \
  -- python3 /path/to/fanta-edit/examples/agent-plugin/fanta-agent-bridge
```

## Codex

In `~/.codex/config.toml`:

```toml
[mcp_servers.fanta]
command = "python3"
args = ["/path/to/fanta-edit/examples/agent-plugin/fanta-agent-bridge"]
env = { FANTA_BIN = "/path/to/fanta" }
```

## Configuration

| Variable | Meaning |
| --- | --- |
| `FANTA_BIN` | the fanta binary; default is `fanta` on `PATH` |
| `FANTA_PROJECT` | project directory to open when the wrapper launches the app |
| `FANTA_USER_DATA_DIR` | isolated profile, passed to app and bridge alike |
| `FANTA_LAUNCH_TIMEOUT` | seconds to wait for the live server (default 60) |

## Try it

With the server registered, ask your agent to, for example, "add a rectangle
named hello to the open Fanta page and screenshot it". The edit autosaves
into the project's source as a git diff — `script/smoke-mcp` proves the same
loop end to end in CI form.

This example is the manual version of what the packaged plugin will install;
the spec covers the rest (bundled skills, version handshake, preview).
