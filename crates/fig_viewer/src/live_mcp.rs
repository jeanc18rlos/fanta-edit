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

use anyhow::{Context as _, Result};
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
/// bounding boxes).
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct BatchGetArgs {
    /// Fetch these node ids in full detail.
    #[serde(default)]
    ids: Option<Vec<String>>,
    /// Page index to list when `ids` is omitted.
    #[serde(default)]
    page: Option<usize>,
    /// How many levels of children to include below each listed node.
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
        let query = NodeQuery {
            ids: args.ids,
            page: args.page,
            depth: args.depth,
            include_geometry: args.include_geometry,
        };
        let value = with_surface(cx, move |surface, cx| surface.get_nodes(query, cx))?;
        Ok(text_response(value))
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
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct ReadFnxSourceArgs {
    /// Project-relative source path; omit to list the source files.
    #[serde(default)]
    path: Option<String>,
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
        let path = input.0.path;
        let value = with_surface(cx, move |surface, cx| surface.read_source(path, cx))?;
        Ok(text_response(value))
    }
}
