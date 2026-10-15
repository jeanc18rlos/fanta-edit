# Agent surface rules

`design_surface` is the seam between agents and the canvas: the `DesignSurface` trait, `DesignOp` and query types, and `DESIGN_GUIDELINES`. It is implemented by `crates/fig_viewer/src/agent_surface.rs`, served to external agents by `crates/fig_viewer/src/live_mcp.rs`, and used by the built-in agent's tools in `crates/agent/src/tools/design_*_tool.rs` and `place_generation_tool.rs`. Read these rules before changing any of those files.

PR numbers point at the change that found each trap (`gh pr view <n>`).

## Keep agent guidance in sync

Agent-facing behavior is described in several places that nothing checks against each other. When you add, rename or change a `DesignOp`, an MCP tool or a tool's arguments, update in the same change:

1. `DESIGN_GUIDELINES` in `src/design_surface.rs` (served by `get_guidelines`).
2. The live MCP tool in `crates/fig_viewer/src/live_mcp.rs` and the built-in tool in `crates/agent/src/tools/`, as applicable.
3. The seeded project guide `AGENTS_MD` in `crates/fanta-format/src/project/layout.rs`. Add the SHA-256 of the replaced text to `previous_seeds`, or existing projects keep the old guide.
4. The tool table and tool count in `docs/fanta/agent-quickstart.md`, and `README.md` ("Connect an agent") if the count changes.
5. The agent prompt templates in `crates/agent/src/templates/` when the canvas workflow they describe changes.

Then run `cargo test --locked -p agent --lib test_fanta_specs` and `test_design_progress` (both CI gates), and `script/smoke-mcp` against a running app (see the `fanta-verify` skill).

## Traps

- Ids that agents send and receive use the prefixed `Display` form (`n_…` for nodes, `c_…` for components), while project files store bare ULIDs. Tools accept and return the prefixed form; convert only where project files are read or written. (#128)
- Prompt preparation calls `DesignSurface::project_root`, which does not build the project overview. Call `state` (full design metadata) only when the prompt actually needs it. (#126)
- Tool calls can arrive while the canvas entity is being updated. Defer work that reads the same entity to a spawned foreground task; see the re-entrancy rules in `crates/fig_viewer/AGENTS.md`. (#26)
