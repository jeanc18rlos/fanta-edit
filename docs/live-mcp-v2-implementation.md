# Live MCP v2 — phase 1 implementation spec

Target worktree: `/path/to/fanta-edit` (branch `feat/live-mcp-v2`).
Engine crates resolve via `../fanta-engine-migration`. Everything below was
verified against the codebase (anchors included). Follow repo idiom; keep
comments to non-obvious constraints only.

## 1. New crate `crates/design_surface` (the seam)

Purpose: let `crates/agent` (which must NOT depend on fig_viewer/workspace)
drive the design editor. Deps: gpui, anyhow, serde, serde_json, schemars,
collections. Contents:

- Typed DTOs (Serialize+Deserialize+JsonSchema): `DesignOp` (untagged or
  `op`-tagged enum): CreateNode{node_type: frame|rectangle|ellipse|text,
  parent: Option<String>, name, x, y, width, height, fill: Option<String hex>,
  text: Option<String>, font_size: Option<f32>}, SetProps{id, name?, x?, y?,
  width?, height?, opacity?, fill?, corner_radius?, text?, hidden?, locked?},
  Reparent{id, parent, index: Option<usize>}, Delete{id}, Select{ids},
  SetViewport{center:[f64;2]?, zoom?}.
- `NodeQuery { ids: Option<Vec<String>>, page: Option<usize>, depth: Option<u32>, include_geometry: bool }`
- `ScreenshotTarget { page: Option<usize>, node: Option<String>, max_dimension: Option<u32> }`
- `pub trait DesignSurface: 'static { ... }` — methods (all `&self, cx: &mut App`):
  `state(&self, cx) -> Result<serde_json::Value>` (project name/root, pages
  [index,name,root id], active page, selection ids, viewport, node counts,
  is_editable, dirty),
  `get_nodes(&self, q: NodeQuery, cx) -> Result<Value>`,
  `apply(&self, ops: Vec<DesignOp>, label: String, cx) -> Result<Value>`
  (returns created ids + per-op status),
  `screenshot(&self, t: ScreenshotTarget, cx) -> Task<Result<Vec<u8>>>` (PNG),
  `read_source(&self, path: Option<String>, cx) -> Result<Value>` (list fnx
  files or return one file's text).
- Global registry: `struct DesignSurfaceRegistry(Option<Rc<dyn DesignSurface>>)`
  impl gpui::Global; `pub fn register(cx, provider)`, `pub fn active(cx) ->
  Option<Rc<dyn DesignSurface>>`. (Follow the SkillIndex global pattern —
  agent/src/agent.rs:1472 `cx.set_global`.)

## 2. Provider impl in `crates/fig_viewer` (new file `src/agent_surface.rs`)

Registered from `fig_viewer::init` (fig_viewer.rs:62 area): create provider
holding a `Rc<RefCell<Option<WeakEntity<FigItem>>>>` for the active item;
track focus by observing FigView creation/focus (FigView already knows its
`Entity<FigItem>` via `.item()`). Simplest correct: in `FigView`'s focus
handler (or `workspace::register_project_item` wrapper — find where FigView
gets focused, e.g. its `Focusable` impl / `focus_in`) update the registry's
active weak item. Fallback when no focus recorded: none → tools error with
"open a .fig/fanta project first".

Verified APIs to build on (research anchors):
- `FigItem::apply(operation: Operation, cx) -> Result<()>` — document.rs:743
  (re-solves layout, dirties, emits Edited).
- `clipboard::apply_transaction(doc: &mut Doc, label: &str, operations:
  Vec<Operation>) -> Result<bool>` — clipboard.rs:395 (grouped undo +
  rollback). Prefer this for multi-op batches via
  `FigItem::with_document(cx, |d| ...)` (document.rs:778) then emit change.
  CHECK how clipboard.rs paste path triggers layout re-solve + Edited —
  mirror it (may need `item.apply`-equivalent post-steps; if with_document's
  DocChange handles it, use that).
- Reads: `FigItem::doc() -> Option<&Doc>`; `Doc.scene` HashMap<NodeId,
  CanvasNode>; `Scene::{children_of, roots, world_bounds, get}`;
  `Doc::to_json_string()` exists (canonical projection) — for get_nodes,
  serialize selected CanvasNodes via serde (they are serde types) and add
  `world_bounds` when include_geometry.
- Creation: `CanvasNode::new(data: NodeData)` (fresh ULID);
  `Scene::next_child_index(parent)`; `Operation::create_node(node)`.
  NodeData variants: Group (frame when `clip_size` set; has background_fills,
  corner_radius), Vector (PathData — for rectangle/ellipse look at how
  `fanta-tools/src/rect.rs` and `ellipse.rs` build nodes; reuse their
  builders if exported, else replicate), Text (content, TextStyle).
  Fills: look at `properties_ops::add_fill/set_fill_color` and the
  fnxColor/solid fill shape in fanta-doc color.rs.
- Prop edits: use `properties_ops::{resize_operations, field_operations,
  set_corner_radius, set_fill_color, ...}` (properties_ops.rs) — they build
  Vec<Operation> with correct old-values.
- Selection: `d.doc.selection.replace_with(ids)` + DocChange::Selection.
- Screenshot: follow export.rs — `fanta_render::RasterRenderer::
  render_page_with(scene, viewport, page_root, RenderInputs)` on a CLONED
  doc + resolver in a background task (export.rs:126/255 pattern). Encode
  PNG. Downscale so max dimension ≤ target (default 1024).
- FNX: project_root via `FigItem::project_root()`; list
  `pages/*/page.fnx`, `components/*/master.fnx`; read file text. (Write lane
  stays with the normal edit_file tool — the doc hot-reloads, document.rs
  RELOAD_DEBOUNCE.)
- Respect `FigItem::is_editable()` — refuse writes with the reason when the
  FNX buffer is dirty (source_edit_locked).

## 3. Three native agent tools in `crates/agent/src/tools/`

Pattern-match `read_file_tool.rs` (impl AgentTool :208) exactly. Tools depend
only on `design_surface`.

- `design_state_tool.rs` — NAME "design_state", kind Read. Input: NodeQuery-ish
  { nodes: Option<Vec<String>>, page: Option<usize>, include_geometry: bool }.
  No input → surface.state(); with nodes/page → get_nodes. Output: struct
  wrapping serde_json::Value → Into<LanguageModelToolResultContent> (JSON
  string).
- `design_edit_tool.rs` — NAME "design_edit", kind Edit. Input { ops:
  Vec<DesignOp>, label: Option<String> }. Calls surface.apply. Errors → the
  Output type with per-op failure so the model can repair (trait returns
  Task<Result<Output, Output>>).
- `design_screenshot_tool.rs` — NAME "design_screenshot", kind Read. Input =
  ScreenshotTarget. Emit the PNG into the thread via the event stream as an
  image content block (see ToolCallContent::ContentBlock image —
  acp_thread.rs:1801; EditFileTool's event_stream usage in edit_session.rs:251
  shows the update pattern; use `update_fields` with content) AND return a
  small text summary. This is what makes generated/edited designs visible in
  the thread.

Registration gates (ALL required — tools.rs:173-188 comment):
1. `tools!` macro list (tools.rs:190-214) + `mod` lines in tools.rs header.
2. `Thread::add_default_tools` (thread.rs:2095): `self.add_tool(DesignStateTool::new())` etc. (providers resolved per-call from the registry global — tools can be unit structs).
3. `assets/settings/default.json` write/ask profiles (:1170-1199): add
   `"design_state": true, "design_edit": true, "design_screenshot": true`
   (write profile); read-only ones true in ask profile, design_edit false.
4. `crates/settings_ui/src/pages/tool_permissions_setup.rs` TOOLS list.

## 4. Built-in skills (`crates/agent_skills/builtin/`)

Follow `builtin/create-skill/` layout (SKILL.md with name/description
frontmatter; registered in `builtin_skills()` agent_skills.rs:701).
- `builtin/fanta-design/SKILL.md` — adapt the OLD skill (READ it at
  path/to/fanta/plugins/fanta-design/skills/fanta-design/SKILL.md):
  the loop (state → screenshot → edit → screenshot), build mechanics, depth &
  material, typography, icons; rewrite tool names to design_state/
  design_edit/design_screenshot and fnx-source editing via edit_file.
- `builtin/fanta-media/SKILL.md` — teaches: generate via the `fanta` MCP
  server tools (generate_image/edit_image/animate_image/plan_compose from
  https://api.fantaisa.net/mcp), poll with get_generation, then place the
  result: download url via fetch is NOT needed — design_edit gets a
  `CreateNode{node_type:image}`? Phase 1: instruct placing via fnx source
  (Image node with asset) OR leave placement to design_edit once image op
  exists. Keep honest about current capability: if image placement is not
  implemented in phase 1, the skill says to hand the URL to the user +
  create an AiArtifact node stub via design_edit if supported. Mark clearly.

## 5. MCP parity (same session if it fits, else next)

`crates/fig_viewer/src/live_mcp.rs`: use `context_server::listener::McpServer`
(UDS, McpServerTool trait, AsyncApp) registering tools that call the SAME
DesignSurfaceRegistry: get_editor_state, batch_get, batch_design,
get_screenshot, read_fnx_source. Socket path under the app's support dir;
enabled via setting `fanta_live_mcp: { "enabled": true }`. TCP/3846
Streamable-HTTP shim is phase 2 (old codex config compatibility).

## Verification

- `cargo check -p design_surface -p fig_viewer -p agent -p settings_ui` clean.
- `cargo test -p agent` existing tests stay green (add none this phase unless
  cheap).
- Grep-proof the 4 registration gates all updated.
- Do NOT touch anything outside: new crate, fig_viewer (agent_surface.rs +
  init wiring + Cargo.toml), agent (3 tool files + tools.rs + thread.rs),
  assets/settings/default.json, settings_ui permissions page, agent_skills
  builtin. No pushes; commit locally on feat/live-mcp-v2 in logical chunks.
