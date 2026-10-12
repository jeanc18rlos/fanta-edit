# Connect an agent to a Fanta project

Current source guidance, **11 October 2026**. Connection instructions should come
from the installed app to target its executable and user-data directory.

The [12 October recorded example](../alpha/DEMO_GUIDE.md#an-agent-edits-your-design)
uses an external Opus 5.5 agent through Fanta MCP for one bounded CTA edit.
It shows the resulting native layers, saved FNX and Undo/Redo in the accepted
signed app. The request runs through the external client; only the actual result
and its inspection appear in the app footage.

## Connect and verify

1. Open the intended saved project in Fanta. Keep that instance running.
2. Choose **Connect Codex or Claude Code** and copy the generated instructions.
3. For Claude Code, run the copied project-scoped stdio registration command
   from the project folder. For Codex, place the copied `[mcp_servers.fanta]`
   configuration in the project's `.codex/config.toml` and review its trust prompt.
4. Restart the agent, inspect its MCP connection and call `get_editor_state`.
   Confirm the project root, active page and selection before requesting a write.
   The Fanta window also reports the connecting client.
5. Read `get_guidelines`, inspect the target with `batch_get`, and use a bounded
   task such as the [sample label change](../alpha/DEMO_GUIDE.md#agent-extension).

Do not copy an old QA executable path or invent a fixed socket address. The
`--mcp-stdio` bridge reads the running instance's discovery file and connects over
a local Unix socket. The setting `fanta_live_mcp.enabled` is on by default; set
it to `false` to disable the local server. With the app closed or server disabled,
the bridge fails visibly instead of claiming a connection.

## Tool scopes

The inspected source registers these **13 local MCP tools**. Their target is the
focused open design. A tool listing is an interface inventory, not proof every
end-to-end use has passed on a particular installer.

| Tool | Scope and behavior |
| --- | --- |
| `get_editor_state` | Project root, page/component source paths, active page and selection |
| `get_guidelines` | Design guidance and the current project's design specification |
| `batch_get` | Read selected IDs or paginated page children; narrow large results |
| `get_design_system` | Read components, typed variables, modes and bindings |
| `read_fnx_source` | List source files or read a bounded, line-based slice |
| `validate_fnx_source` | Validate a complete FNX or managed JSON candidate without writing |
| `get_screenshot` | Render the current page or an addressed node |
| `batch_design` | Apply typed design operations as one undoable content transaction |
| `import_image` | Register an image in the open project's asset workflow |
| `prepare_design_asset` | Open a composer for user review; does not submit generation |
| `list_comments` | Read project comments |
| `reply_comment` | Add a reply to an existing comment |
| `report_agent_activity` | Report stable agent identity, activity and location |

`batch_get` defaults to at most 200 direct children, with `offset` and `limit`
for subsequent pages. Source reads report truncation and continuation details.
Never treat a truncated slice as a whole file.

## Edit source safely

Use paths returned by `get_editor_state`. Read complete relevant source, preserve
node IDs and build a complete candidate. Call `validate_fnx_source` before saving
it with the agent's normal file tools. There is no local MCP `edit_fnx_source`
tool. Inspect the reloaded canvas and Git diff after the write.

The in-app **Code** workspace edits supported FNX and managed JSON. Invalid or
conflicting drafts retain the last valid scene and file. An unsaved source draft
locks conflicting canvas writes. **Saved Code** in Dev mode is read-only.
Multiple buffer saves are sequential rather than one atomic transaction.

`batch_design` validates typed content operations and groups a successful content
batch in Undo history. External file edits follow reconciliation rules and must
be reviewed. Keep Fanta focused on the intended project while an agent works.

## Hosted services and privacy

The local canvas bridge is separate from `https://api.fantaisa.net/mcp` and its
metered hosted generation services. A local connection does not configure hosted
credentials or establish spending limits. Use the configured route and approved
controls, review the current catalog and avoid paid generation in the first task.
Share only a project you intend the connected agent to read.

For connection problems, check the same Fanta instance is open, its local server
is enabled, and the copied user-data directory matches. Recopy the instructions
after an app-path change. Ask in [Fanta Community](https://discord.gg/qrFtymBYxk)
for connection and workflow help. Send private credential/account problems to
[support@fantaisa.net](mailto:support@fantaisa.net).
