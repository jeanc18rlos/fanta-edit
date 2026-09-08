//! A local MCP server exposing the focused design canvas to EXTERNAL agents
//! (codex, Claude Code, etc.), sharing the same [`design_surface`] provider
//! the built-in agent tools use. On by default; disabled with
//! `"fanta_live_mcp": { "enabled": false }`.
//!
//! Transport is a Unix socket (`context_server::listener::McpServer`). The
//! socket lives in a private temp dir, so the path is advertised in
//! `<data_dir>/fanta_live_mcp.json` for clients to discover:
//! `{ "socket": "/…/mcp.sock", "pid": 1234 }`.
//!
//! Because it is on by default, every launch — including each dev
//! `cargo run` — binds a new socket and overwrites that discovery file, and
//! nothing removes the file when the app quits (only turning the setting off
//! does). So a reader can find a file naming a dead process: the
//! `--mcp-stdio` bridge has to check the recorded pid before trusting the
//! socket path next to it.

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use context_server::listener::{McpServer, McpServerTool, ToolResponse};
use context_server::types::{
    Implementation, InitializeResponse, LATEST_PROTOCOL_VERSION, ProtocolVersion,
    ServerCapabilities, ToolAnnotations, ToolsCapabilities, VERSION_2024_11_05, VERSION_2025_03_26,
    VERSION_2025_06_18, requests,
};
use design_surface::{DesignOp, NodeQuery, ScreenshotTarget};
use gpui::{App, AppContext as _, AsyncApp, ClipboardItem, Task};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::{RegisterSetting, Settings, SettingsStore};
use std::path::PathBuf;
use util::ResultExt as _;
use workspace::Workspace;

#[derive(Debug, RegisterSetting)]
pub struct FantaLiveMcpSettings {
    pub enabled: bool,
}

impl Settings for FantaLiveMcpSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            enabled: content
                .fanta_live_mcp
                .as_ref()
                .and_then(|content| content.enabled)
                .unwrap_or(true),
        }
    }
}

/// The running server (if any). Dropping it closes the socket and removes
/// its temp dir; we also remove the discovery file.
#[derive(Default)]
struct LiveMcpState {
    server: Option<McpServer>,
    /// Guards against a stale startup finishing after the setting flipped
    /// off (or on/off/on): only the latest epoch may install its server.
    epoch: usize,
}

impl gpui::Global for LiveMcpState {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(LiveMcpState::default());
    apply_setting(cx);
    cx.observe_global::<SettingsStore>(apply_setting).detach();
}

fn apply_setting(cx: &mut App) {
    let enabled = FantaLiveMcpSettings::get_global(cx).enabled;
    let state = cx.global_mut::<LiveMcpState>();
    if enabled == state.server.is_some() {
        return;
    }
    state.epoch += 1;
    let epoch = state.epoch;
    if !enabled {
        state.server.take();
        cx.background_spawn(async move {
            std::fs::remove_file(discovery_path()).ok();
        })
        .detach();
        log::info!("fanta live MCP server stopped");
        return;
    }
    cx.spawn(async move |cx| {
        let server = async {
            let mut server = McpServer::new(cx).await?;
            server.add_tool(GetEditorStateTool);
            server.add_tool(BatchGetTool);
            server.add_tool(BatchDesignTool);
            server.add_tool(GetScreenshotTool);
            server.add_tool(ReadFnxSourceTool);
            server.handle_request::<requests::Initialize>(|params, cx| {
                let client_name = params.client_info.name;
                // The handler only holds `&App`, and the connecting agent is
                // typically in a terminal, so the platform-focused window is
                // usually None: notice every window instead.
                cx.spawn(async move |cx| {
                    cx.update(|cx| {
                        for window in cx.windows() {
                            window
                                .update(cx, |_, window, cx| {
                                    crate::view::show_canvas_notice(
                                        format!("Agent connected: {client_name}"),
                                        window,
                                        cx,
                                    );
                                })
                                .log_err();
                        }
                    });
                })
                .detach();

                let requested = params.protocol_version.0;
                let protocol_version = if matches!(
                    requested.as_str(),
                    VERSION_2024_11_05
                        | VERSION_2025_03_26
                        | VERSION_2025_06_18
                        | LATEST_PROTOCOL_VERSION
                ) {
                    requested
                } else {
                    LATEST_PROTOCOL_VERSION.to_string()
                };

                Task::ready(Ok(InitializeResponse {
                    protocol_version: ProtocolVersion(protocol_version),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapabilities {
                            list_changed: Some(false),
                        }),
                        ..Default::default()
                    },
                    server_info: Implementation {
                        name: "fanta".into(),
                        title: Some("Fanta".into()),
                        version: env!("CARGO_PKG_VERSION").into(),
                        description: None,
                    },
                    meta: None,
                }))
            });
            server.handle_request::<Ping>(|_, _| Task::ready(Ok(Default::default())));
            anyhow::Ok(server)
        }
        .await
        .context("starting the fanta live MCP server")
        .log_err();
        let Some(server) = server else { return };

        let discovery = serde_json::json!({
            "socket": server.socket_path(),
            "pid": std::process::id(),
        });
        cx.background_spawn(async move {
            std::fs::write(
                discovery_path(),
                serde_json::to_string_pretty(&discovery).unwrap_or_default(),
            )
            .context("writing the live MCP discovery file")
            .log_err();
        })
        .detach();

        cx.update(|cx| {
            let state = cx.global_mut::<LiveMcpState>();
            // The setting may have flipped again while we were binding the
            // socket; only the newest activation installs itself.
            if state.epoch == epoch {
                log::info!(
                    "fanta live MCP server listening on {}",
                    server.socket_path().display()
                );
                state.server = Some(server);
            }
        });
    })
    .detach();
}

fn discovery_path() -> PathBuf {
    paths::data_dir().join("fanta_live_mcp.json")
}

/// `ping`, declared locally rather than reusing
/// [`context_server::types::requests::Ping`]: that one's response is `()`,
/// which serialises to `null`, and clients reject a ping result that is not
/// an object.
struct Ping;

impl context_server::types::Request for Ping {
    type Params = Option<serde_json::Value>;
    type Response = serde_json::Map<String, serde_json::Value>;
    const METHOD: &'static str = "ping";
}

/// Hook `Connect External Agent` up to a freshly created workspace. Called
/// from [`crate::workspace_hooks::init`]'s `observe_new` so every window gets
/// it.
pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(
        |_workspace, _: &zed_actions::fanta::ConnectExternalAgent, window, cx| {
            let executable = match std::env::current_exe() {
                Ok(executable) => executable,
                Err(error) => {
                    log::error!("locating the Fanta executable failed: {error:#}");
                    crate::view::show_canvas_notice(
                        format!("Fanta could not locate its own executable: {error}"),
                        window,
                        cx,
                    );
                    return;
                }
            };
            let executable = executable.display().to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(format!(
                "claude mcp add -s user fanta -- {executable} --mcp-stdio"
            )));
            crate::view::show_canvas_notice(
                format!(
                    "Claude Code command copied. For Codex, add to ~/.codex/config.toml: \
                     [mcp_servers.fanta] command = \"{executable}\" args = [\"--mcp-stdio\"]"
                ),
                window,
                cx,
            );
        },
    );
}

/// Deserialize helper: treat omitted/`null` MCP `arguments` as the default
/// input instead of erroring, while keeping the inner type's schema.
#[derive(Debug, Clone)]
struct OrDefault<T>(T);

impl<'de, T: Deserialize<'de> + Default> Deserialize<'de> for OrDefault<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(
            Option::<T>::deserialize(deserializer)?.unwrap_or_default(),
        ))
    }
}

impl<T: JsonSchema> JsonSchema for OrDefault<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        T::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        T::json_schema(generator)
    }
}

fn text_response(value: serde_json::Value) -> ToolResponse<()> {
    ToolResponse {
        content: vec![context_server::types::ToolResponseContent::Text {
            text: serde_json::to_string(&value).unwrap_or_default(),
        }],
        structured_content: (),
    }
}

/// Cap on one serialized JSON tool result. Whatever an MCP client can carry,
/// a model cannot use megabytes of JSON, and a client that drops the response
/// leaves the agent with nothing at all — so a query whose answer is this big
/// is refused with instructions for narrowing it, never silently cut.
const MAX_JSON_RESPONSE_BYTES: usize = 256 * 1024;

/// Like [`text_response`], but refuses a result too large to be carried or
/// read. `narrowing_hint` must tell the model how to ask a smaller question.
fn bounded_text_response(
    value: serde_json::Value,
    narrowing_hint: &str,
) -> Result<ToolResponse<()>> {
    let text = serde_json::to_string(&value).context("serializing the tool result")?;
    if text.len() > MAX_JSON_RESPONSE_BYTES {
        bail!(
            "this result would be {} bytes, over the {MAX_JSON_RESPONSE_BYTES}-byte response \
             budget, and no part of it was returned. {narrowing_hint}",
            text.len()
        );
    }
    Ok(ToolResponse {
        content: vec![context_server::types::ToolResponseContent::Text { text }],
        structured_content: (),
    })
}

/// Resolve the design surface and run `f` against it on the main thread.
fn with_surface<R: 'static>(
    cx: &mut AsyncApp,
    f: impl FnOnce(std::rc::Rc<dyn design_surface::DesignSurface>, &mut App) -> Result<R> + 'static,
) -> Result<R> {
    cx.update(|cx| {
        let surface = design_surface::active(cx).context(
            "no design canvas is open; open a .fig file or Fanta project in Fanta first",
        )?;
        f(surface, cx)
    })
}

fn read_only() -> ToolAnnotations {
    ToolAnnotations {
        title: None,
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: None,
        open_world_hint: Some(false),
    }
}

// ---- get_editor_state --------------------------------------------------------

/// Overview of the open Fanta design document: project name and root, pages
/// (index, name, root id, node counts), the active page, current selection,
/// viewport, and whether the canvas is editable right now.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct GetEditorStateArgs {}

#[derive(Clone)]
struct GetEditorStateTool;

impl McpServerTool for GetEditorStateTool {
    type Input = OrDefault<GetEditorStateArgs>;
    type Output = ();

    const NAME: &'static str = "get_editor_state";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, _input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, |surface, cx| surface.state(cx))?;
        Ok(text_response(value))
    }
}

// ---- batch_get ---------------------------------------------------------------

/// Read nodes from the open design document: pass `ids` for full node detail,
/// or omit them to list a page's node tree in compact form (`page` defaults to
/// the active page; `depth` limits recursion; `include_geometry` adds world
/// bounding boxes). A listing is capped at 262144 bytes; if the tree is wider
/// than that the call is refused rather than truncated, so lower `depth` or
/// walk down through `ids`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct BatchGetArgs {
    /// Fetch these node ids in full detail.
    #[serde(default)]
    ids: Option<Vec<String>>,
    /// Page index to list when `ids` is omitted.
    #[serde(default)]
    page: Option<usize>,
    /// How many levels of children to include below each listed node.
    /// Defaults to 2 when listing a page, because an imported `.fig` page can
    /// hold tens of thousands of nodes. A node cut off by the limit reports
    /// `child_count` instead of `children`: re-request it by id to go deeper.
    #[serde(default)]
    depth: Option<u32>,
    /// Include world-space bounding boxes.
    #[serde(default)]
    include_geometry: bool,
}

#[derive(Clone)]
struct BatchGetTool;

impl McpServerTool for BatchGetTool {
    type Input = OrDefault<BatchGetArgs>;
    type Output = ();

    const NAME: &'static str = "batch_get";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let args = input.0;
        let listing = args.ids.is_none();
        let query = NodeQuery {
            ids: args.ids,
            page: args.page,
            // An unbounded page listing walks the whole scene; on a real
            // imported `.fig` that is tens of thousands of nodes.
            depth: args.depth.or(if listing { Some(2) } else { None }),
            include_geometry: args.include_geometry,
        };
        let value = with_surface(cx, move |surface, cx| surface.get_nodes(query, cx))?;
        bounded_text_response(
            value,
            "Narrow it: lower `depth` (try 1), set `include_geometry` to false, or pass the \
             `ids` of the specific nodes you need.",
        )
    }
}

// ---- batch_design ------------------------------------------------------------

/// Apply a batch of design ops to the open canvas as ONE undoable
/// transaction: create_node (frame/rectangle/ellipse/text), create_image
/// (base64 source), set_props, reparent, delete, select, set_viewport. If any
/// op fails the whole batch rolls back and the result names the failing op.
/// Strokes, gradients, shadows, auto-layout, fonts and components are not
/// batch_design properties: read the page .fnx with read_fnx_source, edit the
/// file, and the canvas reloads (the change is a git diff).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct BatchDesignArgs {
    /// The ops to apply, in order.
    ops: Vec<DesignOp>,
    /// Undo-history label for the batch (e.g. "Add login card").
    #[serde(default)]
    label: Option<String>,
}

#[derive(Clone)]
struct BatchDesignTool;

impl McpServerTool for BatchDesignTool {
    type Input = BatchDesignArgs;
    type Output = ();

    const NAME: &'static str = "batch_design";

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations {
            title: None,
            read_only_hint: Some(false),
            // Edits are undoable in-app, but destructive from the caller's
            // point of view (delete removes subtrees).
            destructive_hint: Some(true),
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
        }
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let label = input.label.unwrap_or_else(|| "MCP edit".to_string());
        let ops = input.ops;
        let value = with_surface(cx, move |surface, cx| surface.apply(ops, label, cx))?;
        Ok(text_response(value))
    }
}

// ---- get_screenshot ----------------------------------------------------------

/// Render a PNG screenshot of the open design canvas: the active page by
/// default, another page via `page`, or a single node's region via `node`.
/// Returned as MCP image content.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct GetScreenshotArgs {
    /// Page index to render (defaults to the active page).
    #[serde(default)]
    page: Option<usize>,
    /// Render only this node's region of its page.
    #[serde(default)]
    node: Option<String>,
    /// Cap on the longer output dimension in pixels (default 1024).
    #[serde(default)]
    max_dimension: Option<u32>,
}

#[derive(Clone)]
struct GetScreenshotTool;

impl McpServerTool for GetScreenshotTool {
    type Input = OrDefault<GetScreenshotArgs>;
    type Output = ();

    const NAME: &'static str = "get_screenshot";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let args = input.0;
        let target = ScreenshotTarget {
            page: args.page,
            node: args.node,
            max_dimension: args.max_dimension,
        };
        let render = with_surface(cx, move |surface, cx| Ok(surface.screenshot(target, cx)))?;
        let png = render.await?;
        Ok(ToolResponse {
            content: vec![context_server::types::ToolResponseContent::Image {
                data: base64::engine::general_purpose::STANDARD.encode(&png),
                mime_type: "image/png".to_string(),
            }],
            structured_content: (),
        })
    }
}

// ---- read_fnx_source ---------------------------------------------------------

/// List the open Fanta project's FNX source files (omit `path`), or return one
/// file's text (e.g. `pages/<id>/page.fnx`). Edit sources with your own file
/// tools; while the project is open in Fanta the canvas hot-reloads about
/// 300ms after a save, and otherwise the edit is picked up the next time it is
/// opened.
///
/// A page imported from a `.fig` runs to tens of megabytes, so a file is
/// returned in slices: at most 65536 bytes by default, starting at line
/// `offset`. The result always carries `total_bytes`, `total_lines` and
/// `truncated`, and a truncated result carries a `notice` naming the exact
/// arguments for the next slice. Never assume a slice is the whole file.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct ReadFnxSourceArgs {
    /// Project-relative source path; omit to list the source files.
    #[serde(default)]
    path: Option<String>,
    /// 1-based line to start at (default 1).
    #[serde(default)]
    offset: Option<usize>,
    /// Maximum number of lines to return.
    #[serde(default)]
    limit: Option<usize>,
    /// Maximum bytes of file text to return (default 65536, ceiling 1048576).
    #[serde(default)]
    max_bytes: Option<usize>,
}

/// Default cap on the file text one `read_fnx_source` call returns. One page
/// of a 9.6 MB `.fig` is over 43 million characters, which no MCP client will
/// carry and no model can read; 64 KiB is roughly 16k tokens, small enough to
/// sit in a tool result next to everything else an agent is holding, and large
/// enough for 31 lines of that page — a whole frame's worth of `.fnx`.
const DEFAULT_SOURCE_BYTES: usize = 64 * 1024;

/// Ceiling on `max_bytes`, so a caller cannot ask for a response that breaks
/// its own transport.
const MAX_SOURCE_BYTES: usize = 1024 * 1024;

/// One slice of a source file, with everything the model needs to know that it
/// is holding a slice and how to ask for the rest.
#[derive(Debug, PartialEq, Eq)]
struct SourceSlice {
    text: String,
    /// 1-based line number of the first returned line; 0 when nothing was
    /// returned.
    first_line: usize,
    /// 1-based, inclusive line number of the last returned line; 0 when
    /// nothing was returned.
    last_line: usize,
    /// Pass as `offset` to continue; `None` when the file ends here.
    next_offset: Option<usize>,
    /// The 1-based line the caller asked to start at, echoed so an empty
    /// slice can say what was asked for.
    requested_offset: usize,
    total_bytes: usize,
    total_lines: usize,
    /// The line the byte budget was hit inside of, when the budget ran out
    /// part way through a single line.
    partial_line: Option<usize>,
}

/// Take `limit` lines from line `offset` of `text`, within a byte budget.
fn slice_source(
    text: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    max_bytes: Option<usize>,
) -> SourceSlice {
    let total_bytes = text.len();
    // `split_inclusive` keeps the line terminators, so concatenating the
    // returned lines reproduces the file byte for byte.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let total_lines = lines.len();
    let budget = max_bytes
        .unwrap_or(DEFAULT_SOURCE_BYTES)
        .clamp(1, MAX_SOURCE_BYTES);
    let start = offset.unwrap_or(1).max(1) - 1;
    let limit = limit.unwrap_or(usize::MAX);

    let mut slice = String::new();
    let mut taken = 0usize;
    let mut partial_line = None;
    for (index, line) in lines.iter().enumerate().skip(start).take(limit) {
        if slice.len() + line.len() <= budget {
            slice.push_str(line);
            taken += 1;
            continue;
        }
        if slice.is_empty() {
            // A single line wider than the budget would otherwise return
            // nothing at all and stall the caller's paging.
            let end = floor_char_boundary(line, budget);
            slice.push_str(&line[..end]);
            taken = 1;
            partial_line = Some(index + 1);
        }
        break;
    }

    let (first_line, last_line) = if taken == 0 {
        (0, 0)
    } else {
        (start + 1, start + taken)
    };
    let next_offset = (taken > 0 && last_line < total_lines).then_some(last_line + 1);
    SourceSlice {
        text: slice,
        first_line,
        last_line,
        next_offset,
        requested_offset: start + 1,
        total_bytes,
        total_lines,
        partial_line,
    }
}

/// The largest `index` at or below the given one that splits `text` between
/// characters (`str::floor_char_boundary` is still unstable).
fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut boundary = index;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

impl SourceSlice {
    fn truncated(&self) -> bool {
        self.next_offset.is_some()
            || self.partial_line.is_some()
            || (self.first_line == 0 && self.total_lines > 0)
    }

    /// The warning a truncated slice carries. Deleting or rewriting a design
    /// on the belief that an unread tail is empty is the failure this exists
    /// to prevent, so the notice says outright that the file is not all here.
    fn notice(&self, path: &str) -> Option<String> {
        if !self.truncated() {
            return None;
        }
        if self.first_line == 0 {
            return Some(format!(
                "NOTHING WAS RETURNED: {path} has {} lines ({} bytes), and the slice requested \
                 from line {} is empty. Re-request with {{\"path\": \"{path}\", \"offset\": 1}} to \
                 start from the top.",
                self.total_lines, self.total_bytes, self.requested_offset
            ));
        }
        let mut notice = format!(
            "TRUNCATED — THIS IS NOT THE WHOLE FILE. Returned lines {}-{} of {} ({} of {} bytes) \
             of {path}. To continue, call read_fnx_source again with {{\"path\": \"{path}\", \
             \"offset\": {}}}; pass \"limit\" (lines) or \"max_bytes\" (up to {MAX_SOURCE_BYTES}) \
             for a different slice size. Do not edit, replace or delete anything on the \
             assumption that the lines you have not read are absent.",
            self.first_line,
            self.last_line,
            self.total_lines,
            self.text.len(),
            self.total_bytes,
            self.next_offset.unwrap_or(self.last_line + 1),
        );
        if let Some(line) = self.partial_line {
            notice.push_str(&format!(
                " Line {line} is longer than the byte budget and was cut mid-line, so the rest of \
                 line {line} is in NO later slice: re-request that line with a larger \
                 \"max_bytes\" to read it whole."
            ));
        }
        Some(notice)
    }

    fn into_response(self, path: &str) -> serde_json::Value {
        let notice = self.notice(path);
        serde_json::json!({
            "path": path,
            "text": self.text,
            "first_line": self.first_line,
            "last_line": self.last_line,
            "next_offset": self.next_offset,
            "total_lines": self.total_lines,
            "total_bytes": self.total_bytes,
            "truncated": self.truncated(),
            "notice": notice,
        })
    }
}

#[derive(Clone)]
struct ReadFnxSourceTool;

impl McpServerTool for ReadFnxSourceTool {
    type Input = OrDefault<ReadFnxSourceArgs>;
    type Output = ();

    const NAME: &'static str = "read_fnx_source";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let args = input.0;
        let requested = args.path.clone();
        let value = with_surface(cx, move |surface, cx| surface.read_source(requested, cx))?;
        let Some(path) = args.path else {
            return bounded_text_response(
                value,
                "This project has more source files than fit in one result; read them by name \
                 from `pages/` and `components/` instead.",
            );
        };
        let text = value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let slice = slice_source(text, args.offset, args.limit, args.max_bytes);
        Ok(text_response(slice.into_response(&path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_returns_a_whole_small_file_untruncated() {
        let slice = slice_source("a\nb\nc\n", None, None, None);
        assert_eq!(slice.text, "a\nb\nc\n");
        assert_eq!((slice.first_line, slice.last_line), (1, 3));
        assert_eq!(slice.next_offset, None);
        assert!(!slice.truncated());
        assert_eq!(slice.notice("pages/page-1/page.fnx"), None);
    }

    #[test]
    fn slice_stops_at_the_byte_budget_and_says_how_to_continue() {
        let source = "0123456789\n".repeat(100);
        let slice = slice_source(&source, None, None, Some(35));
        assert_eq!(slice.text, "0123456789\n0123456789\n0123456789\n");
        assert_eq!((slice.first_line, slice.last_line), (1, 3));
        assert_eq!(slice.next_offset, Some(4));
        assert_eq!(slice.total_bytes, 1100);
        assert_eq!(slice.total_lines, 100);
        assert!(slice.truncated());

        let notice = slice
            .notice("pages/page-1/page.fnx")
            .expect("a truncated slice must carry a notice");
        assert!(notice.contains("TRUNCATED"));
        // The true size and the exact next call are the two facts a model
        // needs to avoid acting on a partial file.
        assert!(notice.contains("1100 bytes"));
        assert!(notice.contains("\"offset\": 4"));
        assert!(notice.contains("pages/page-1/page.fnx"));
    }

    #[test]
    fn slice_honors_an_offset_and_a_line_limit() {
        let source = "a\nb\nc\nd\ne\n";
        let slice = slice_source(source, Some(2), Some(2), None);
        assert_eq!(slice.text, "b\nc\n");
        assert_eq!((slice.first_line, slice.last_line), (2, 3));
        assert_eq!(slice.next_offset, Some(4));
        assert!(slice.truncated());
    }

    #[test]
    fn slice_cuts_inside_an_oversized_line_and_warns_about_the_remainder() {
        let source = format!("{}\nsecond\n", "x".repeat(200));
        let slice = slice_source(&source, None, None, Some(50));
        assert_eq!(slice.text, "x".repeat(50));
        assert_eq!(slice.partial_line, Some(1));
        assert_eq!(slice.next_offset, Some(2));
        let notice = slice.notice("page.fnx").expect("a cut line must be reported");
        assert!(notice.contains("cut mid-line"));
        assert!(notice.contains("max_bytes"));
    }

    #[test]
    fn slice_never_splits_a_multibyte_character() {
        let source = "ééééé";
        let slice = slice_source(source, None, None, Some(5));
        assert_eq!(slice.text, "éé");
        assert_eq!(slice.partial_line, Some(1));
    }

    #[test]
    fn slice_past_the_end_returns_nothing_and_says_so() {
        let slice = slice_source("a\nb\n", Some(9), None, None);
        assert!(slice.text.is_empty());
        assert_eq!(slice.next_offset, None);
        assert!(slice.truncated());
        let notice = slice.notice("page.fnx").expect("an empty slice must be reported");
        assert!(notice.contains("NOTHING WAS RETURNED"));
        assert!(notice.contains("line 9"));
    }

    #[test]
    fn an_empty_file_is_not_reported_as_truncated() {
        let slice = slice_source("", None, None, None);
        assert!(!slice.truncated());
        assert_eq!(slice.notice("page.fnx"), None);
    }

    #[test]
    fn max_bytes_is_capped_so_a_caller_cannot_ask_for_the_whole_43mb_page() {
        let source = "x".repeat(4 * MAX_SOURCE_BYTES);
        let slice = slice_source(&source, None, None, Some(usize::MAX));
        assert_eq!(slice.text.len(), MAX_SOURCE_BYTES);
        assert!(slice.truncated());
    }

    #[test]
    fn the_response_carries_the_notice_and_the_true_total() {
        let source = "0123456789\n".repeat(100);
        let response = slice_source(&source, None, None, Some(35)).into_response("page.fnx");
        assert_eq!(response["truncated"], serde_json::json!(true));
        assert_eq!(response["total_bytes"], serde_json::json!(1100));
        assert_eq!(response["next_offset"], serde_json::json!(4));
        assert!(
            response["notice"]
                .as_str()
                .is_some_and(|notice| notice.contains("TRUNCATED"))
        );
    }

    #[test]
    fn bounded_response_refuses_an_oversized_result_with_instructions() {
        let value = serde_json::json!({ "nodes": "n".repeat(MAX_JSON_RESPONSE_BYTES + 1) });
        let error = bounded_text_response(value, "Lower `depth`.")
            .expect_err("an oversized result must be refused, not carried");
        let message = error.to_string();
        assert!(message.contains("response budget"));
        assert!(message.contains("Lower `depth`."));
    }
}
