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
//! The discovery file is published atomically only after startup succeeds.
//! The stdio bridge checks its recorded pid in case the app crashed.

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use context_server::listener::{McpServer, McpServerTool, ToolResponse};
use context_server::types::{
    Implementation, InitializeResponse, LATEST_PROTOCOL_VERSION, ProtocolVersion,
    ServerCapabilities, ToolAnnotations, ToolsCapabilities, VERSION_2024_11_05, VERSION_2025_03_26,
    VERSION_2025_06_18, requests,
};
use design_surface::{DesignOp, MAX_JSON_RESPONSE_BYTES, NodeQuery, ScreenshotTarget};
use futures::{AsyncReadExt as _, FutureExt as _};
use gpui::{App, AppContext as _, AsyncApp, ClipboardItem, Task};
use http_client::HttpClient as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::{RegisterSetting, Settings, SettingsStore};
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};
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
    enabled: bool,
    _quit_subscription: Option<gpui::Subscription>,
    /// Guards against a stale startup finishing after the setting flipped
    /// off (or on/off/on): only the latest epoch may install its server.
    epoch: usize,
}

impl gpui::Global for LiveMcpState {}

/// The live server's tools, listed once: registration expands it with
/// `add_tools!` and the tests with a description check, so a tool can't be
/// registered without being checked.
macro_rules! live_tools {
    ($apply:ident!($($arg:tt)*)) => {
        $apply!(
            $($arg)*
            GetEditorStateTool,
            GetDesignSystemTool,
            PrepareDesignAssetTool,
            BatchGetTool,
            BatchDesignTool,
            GetScreenshotTool,
            ReadFnxSourceTool,
            ValidateFnxSourceTool,
            GetGuidelinesTool,
            ImportImageTool,
            ListCommentsTool,
            ReplyCommentTool,
            ReportActivityTool
        )
    };
}

macro_rules! add_tools {
    ($server:ident, $($tool:ident),+) => {
        $($server.add_tool($tool);)+
    };
}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(LiveMcpState::default());

    let quit_subscription = cx.on_app_quit(|cx| {
        let state = cx.global_mut::<LiveMcpState>();
        state.epoch += 1;
        state.enabled = false;
        state.server.take();
        design_surface::set_live_mcp_command(None, cx);
        remove_discovery().log_err();
        Task::ready(())
    });
    cx.global_mut::<LiveMcpState>()._quit_subscription = Some(quit_subscription);
    apply_setting(cx);
    cx.observe_global::<SettingsStore>(apply_setting).detach();
}

fn apply_setting(cx: &mut App) {
    let enabled = FantaLiveMcpSettings::get_global(cx).enabled;
    let state = cx.global_mut::<LiveMcpState>();
    if enabled == state.enabled {
        return;
    }
    state.enabled = enabled;
    state.epoch += 1;
    let epoch = state.epoch;
    if !enabled {
        state.server.take();
        design_surface::set_live_mcp_command(None, cx);
        remove_discovery().log_err();
        log::info!("fanta live MCP server stopped");
        return;
    }
    #[cfg(not(test))]
    match std::env::current_exe() {
        Ok(executable) => design_surface::set_live_mcp_command(
            Some(design_surface::LiveMcpCommand {
                executable: executable.display().to_string(),
                args: vec![
                    "--mcp-stdio".into(),
                    "--user-data-dir".into(),
                    paths::data_dir().display().to_string(),
                ],
            }),
            cx,
        ),
        Err(error) => log::error!("locating the Fanta MCP bridge executable failed: {error:#}"),
    }
    cx.spawn(async move |cx| {
        let server = async {
            let mut server = McpServer::new(cx).await?;
            live_tools!(add_tools!(server,));
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
        .context("starting the fanta live MCP server");
        let server = match server {
            Ok(server) => server,
            Err(error) => {
                log::error!("{error:#}");
                cx.update(|cx| {
                    let state = cx.global_mut::<LiveMcpState>();
                    if state.epoch == epoch {
                        state.enabled = false;
                        design_surface::set_live_mcp_command(None, cx);
                        show_connection_error(
                            format!("Fanta live tools could not start: {error:#}"),
                            cx,
                        );
                    }
                });
                return;
            }
        };

        cx.update(|cx| {
            let state = cx.global_mut::<LiveMcpState>();
            // The setting may have flipped again while we were binding the
            // socket; only the newest activation installs itself.
            if state.epoch == epoch && state.enabled {
                if let Err(error) = write_discovery(server.socket_path()) {
                    log::error!("publishing the Fanta MCP connection failed: {error:#}");
                    state.enabled = false;
                    design_surface::set_live_mcp_command(None, cx);
                    show_connection_error(
                        format!("Fanta live tools could not publish the connection: {error:#}"),
                        cx,
                    );
                    return;
                }
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

fn show_connection_error(message: String, cx: &mut App) {
    for window in cx.windows() {
        window
            .update(cx, |_, window, cx| {
                crate::view::show_canvas_notice(message.clone(), window, cx);
            })
            .log_err();
    }
}

fn discovery_path() -> PathBuf {
    paths::data_dir().join("fanta_live_mcp.json")
}

fn write_discovery(socket: &Path) -> Result<()> {
    std::fs::create_dir_all(paths::data_dir())?;
    let mut file = tempfile::NamedTempFile::new_in(paths::data_dir())?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "socket": socket, "pid": std::process::id(),
        }),
    )?;
    file.flush()?;
    file.persist(discovery_path())
        .context("publishing the live MCP discovery file")?;
    Ok(())
}

fn remove_discovery() -> Result<()> {
    let path = discovery_path();
    let contents = match std::fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let discovery: serde_json::Value = serde_json::from_slice(&contents)?;
    if discovery.get("pid").and_then(serde_json::Value::as_u64)
        == Some(u64::from(std::process::id()))
    {
        std::fs::remove_file(path).context("removing the live MCP discovery file")?;
    }
    Ok(())
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
            if !FantaLiveMcpSettings::get_global(cx).enabled {
                crate::view::show_canvas_notice("Enable fanta_live_mcp.enabled to connect an external agent".into(), window, cx);
                return;
            }
            let instructions = connection_instructions(&executable, paths::data_dir());
            cx.write_to_clipboard(ClipboardItem::new_string(instructions));
            crate::view::show_canvas_notice(
                "Connection instructions copied for Codex and Claude Code. Keep this project open in Fanta.".into(),
                window,
                cx,
            );
        },
    );
}

fn connection_instructions(executable: &Path, data_directory: &Path) -> String {
    let command = design_surface::LiveMcpCommand {
        executable: executable.display().to_string(),
        args: vec![
            "--mcp-stdio".into(),
            "--user-data-dir".into(),
            data_directory.display().to_string(),
        ],
    };
    format!(
        "Fanta live canvas connection\n\nKeep Fanta running with this project open.\n\n\
         Codex: add the following to this project's .codex/config.toml, then restart the agent:\n\n\
         {}\n\
         Claude Code: run this command from the project directory, then restart the agent:\n\n\
         {}\n\n\
         Verify with /mcp and call get_editor_state. Edit the returned .fnx files with file tools, \
         validate complete candidates with validate_fnx_source, inspect the canvas with batch_get \
         and get_screenshot, and register generated images with import_image.\n",
        command.codex_config(),
        command.claude_code_command(),
    )
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

fn non_destructive_write() -> ToolAnnotations {
    ToolAnnotations {
        title: None,
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    }
}

/// Import a generated image into the active project's Assets panel and persist
/// its bytes under assets/images/. Supply exactly one of path (local file,
/// relative to the project or absolute), url (HTTP/HTTPS), or source (base64 or
/// a base64 data URI). Returns the asset id and saved path for FNX file edits.
/// Use this after image generation, including images made by another MCP
/// server. Importing an asset does not place a new layer on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ImportImageArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Clone)]
struct ImportImageTool;

impl McpServerTool for ImportImageTool {
    type Input = ImportImageArgs;
    type Output = ();
    const NAME: &'static str = "import_image";

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations {
            open_world_hint: Some(true),
            ..non_destructive_write()
        }
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        if [
            input.path.is_some(),
            input.url.is_some(),
            input.source.is_some(),
        ]
        .into_iter()
        .filter(|provided| *provided)
        .count()
            != 1
        {
            bail!("supply exactly one of path, url or source");
        }
        let initial_state = with_surface(cx, |surface, cx| surface.state(cx))?;
        let document_id = initial_state.get("document_id").cloned();
        let bytes = if let Some(path) = input.path {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                let root = initial_state
                    .get("project_root")
                    .and_then(serde_json::Value::as_str)
                    .context(
                        "relative image paths require a saved project; use an absolute path",
                    )?;
                Path::new(root).join(path)
            };
            cx.background_spawn(async move {
                use std::io::Read as _;
                let file = std::fs::File::open(&path)
                    .with_context(|| format!("opening image {}", path.display()))?;
                if !file.metadata()?.is_file() {
                    bail!("the image path must be a regular file");
                }
                let mut bytes = Vec::new();
                file.take(crate::document::MAX_IMAGE_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > crate::document::MAX_IMAGE_SOURCE_BYTES {
                    bail!("the image exceeds the project asset limit");
                }
                Ok::<_, anyhow::Error>(bytes)
            })
            .await?
        } else if let Some(url) = input.url {
            let parsed = url::Url::parse(&url).context("the image URL is invalid")?;
            if !matches!(parsed.scheme(), "http" | "https") {
                bail!("the image URL must use HTTP or HTTPS");
            }
            let client = cx.update(|cx| client::Client::global(cx).http_client());
            let executor = cx.background_executor().clone();
            cx.background_spawn(async move {
                let download = async {
                    let response = client.get(&url, http_client::AsyncBody::empty(), true).await?;
                    if !response.status().is_success() { bail!("the image download failed: {}", response.status()); }
                    let mut bytes = Vec::new();
                    response.into_body().take(crate::document::MAX_IMAGE_SOURCE_BYTES as u64 + 1).read_to_end(&mut bytes).await?;
                    if bytes.len() > crate::document::MAX_IMAGE_SOURCE_BYTES { bail!("the image exceeds the project asset limit"); }
                    Ok::<_, anyhow::Error>(bytes)
                }.fuse();
                let timeout = executor.timer(Duration::from_secs(60)).fuse();
                futures::pin_mut!(download, timeout);
                futures::select! {
                    result = download => result,
                    _ = timeout => Err(anyhow::anyhow!("the image download timed out; retry importing the image")),
                }
            }).await?
        } else {
            let source = input.source.context("the image source is missing")?;
            cx.background_spawn(async move { crate::agent_surface::decode_image_source(&source) })
                .await?
        };
        let name = input.name.unwrap_or_else(|| "Generated image".into());
        let import = with_surface(cx, move |surface, cx| {
            let state = surface.state(cx)?;
            if state.get("document_id") != document_id.as_ref() {
                bail!(
                    "the active design changed during the image import; select the intended project and retry"
                );
            }
            Ok(surface.import_image(bytes, name, cx))
        })?;
        Ok(text_response(import.await?))
    }
}

/// Read canvas comment threads for design reviews. By default returns unresolved
/// threads on the active page, including the pin position and existing replies.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct ListCommentsArgs {
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    include_resolved: bool,
}

#[derive(Clone)]
struct ListCommentsTool;

impl McpServerTool for ListCommentsTool {
    type Input = OrDefault<ListCommentsArgs>;
    type Output = ();
    const NAME: &'static str = "list_comments";
    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let args = input.0;
        let value = with_surface(cx, move |surface, cx| {
            surface.comments(args.page, args.include_resolved, cx)
        })?;
        bounded_text_response(value, "Read one page's comments at a time.")
    }
}

/// Reply to a canvas comment as an agent. Use the exact comment id returned by
/// list_comments, and set resolve only after addressing and verifying the note.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ReplyCommentArgs {
    #[serde(default)]
    page: Option<usize>,
    id: String,
    body: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    resolve: bool,
}

#[derive(Clone)]
struct ReplyCommentTool;

impl McpServerTool for ReplyCommentTool {
    type Input = ReplyCommentArgs;
    type Output = ();
    const NAME: &'static str = "reply_comment";
    fn annotations(&self) -> ToolAnnotations {
        non_destructive_write()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, move |surface, cx| {
            surface.reply_comment(
                input.page,
                input.id,
                input.body,
                input.author.unwrap_or_else(|| "Fanta Agent".into()),
                input.resolve,
                cx,
            )
        })?;
        Ok(text_response(value))
    }
}

/// Report each visible editing milestone with its page, node, coordinates,
/// source_path and workspace (canvas, variables or code). Choose a stable
/// mage name and agent_id so the user can follow you across files and tabs.
/// Reuse that identity in batch_design.activity for streamed canvas changes.
/// Set active false when finished.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ReportActivityArgs {
    #[serde(flatten)]
    activity: design_surface::AgentActivity,
}

#[derive(Clone)]
struct ReportActivityTool;

impl McpServerTool for ReportActivityTool {
    type Input = ReportActivityArgs;
    type Output = ();
    const NAME: &'static str = "report_agent_activity";
    fn annotations(&self) -> ToolAnnotations {
        non_destructive_write()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, move |surface, cx| {
            match input.activity.source_path.clone() {
                Some(path) => surface.report_source_activity(path, input.activity, cx),
                None => surface.report_activity(input.activity, cx),
            }
        })?;
        Ok(text_response(value))
    }
}

// ---- get_editor_state --------------------------------------------------------

/// Overview of the open Fanta design document: project name and root, pages
/// (index, name, root id, node counts, `.fnx` source path), the active page
/// and its content bounds, components, current selection and its bounds,
/// viewport, whether the canvas is editable right now, and short `hints`.
/// Pass `empty_space: [width, height]` to also get a free `{x, y}` on the
/// page for a new top-level frame of that size.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct GetEditorStateArgs {
    /// `[width, height]` of a box to find room for; the result then carries
    /// `empty_space: {page, x, y, width, height}`.
    #[serde(default)]
    empty_space: Option<[f64; 2]>,
    /// Page index `empty_space` searches (defaults to the active page).
    #[serde(default)]
    page: Option<usize>,
}

#[derive(Clone)]
struct GetEditorStateTool;

impl McpServerTool for GetEditorStateTool {
    type Input = OrDefault<GetEditorStateArgs>;
    type Output = ();

    const NAME: &'static str = "get_editor_state";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let args = input.0;
        let value = with_surface(cx, move |surface, cx| {
            let mut state = surface.state(cx)?;
            if let Some([width, height]) = args.empty_space {
                let spot = surface.find_empty_space(width, height, args.page, cx)?;
                if let Some(state) = state.as_object_mut() {
                    state.insert("empty_space".into(), spot);
                }
            }
            Ok(state)
        })?;
        Ok(text_response(value))
    }
}

// ---- get_guidelines ----------------------------------------------------------

/// Fanta's design guidelines for agents: coordinate system, frames vs groups,
/// auto layout, spacing and type scales, naming, components, the working
/// method (read state, batch ops with a label, screenshot to verify), and when
/// to edit `.fnx` source instead of the canvas. Read once per session before
/// designing.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
struct GetGuidelinesArgs {}

#[derive(Clone)]
struct GetGuidelinesTool;

impl McpServerTool for GetGuidelinesTool {
    type Input = OrDefault<GetGuidelinesArgs>;
    type Output = ();

    const NAME: &'static str = "get_guidelines";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, _input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let spec = with_surface(cx, |surface, cx| surface.read_design_spec(cx))?;
        let mut text = design_surface::DESIGN_GUIDELINES.to_string();
        if let Some(spec) = spec {
            text.push_str(&format!("\n\n{}", spec.prompt_context()));
        }
        Ok(ToolResponse {
            content: vec![context_server::types::ToolResponseContent::Text { text }],
            structured_content: (),
        })
    }
}

/// Inspect reusable component properties/variant sets, variable collections, modes,
/// paginated typed token values, and optional selected-node bindings. Read before creating a design system.
#[derive(Clone)]
struct GetDesignSystemTool;

impl McpServerTool for GetDesignSystemTool {
    type Input = OrDefault<design_surface::DesignSystemQuery>;
    type Output = ();
    const NAME: &'static str = "get_design_system";
    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, move |surface, cx| surface.design_system(input.0, cx))?;
        bounded_text_response(
            value,
            "Choose a collection and lower limit; use offset for subsequent variable pages.",
        )
    }
}

/// Open the existing image or SVG generation composer with a prefilled prompt
/// after user interest. No generation is submitted; the user reviews available catalog models and submits there.
#[derive(Clone)]
struct PrepareDesignAssetTool;

impl McpServerTool for PrepareDesignAssetTool {
    type Input = design_surface::DesignAssetRequest;
    type Output = ();
    const NAME: &'static str = "prepare_design_asset";
    fn annotations(&self) -> ToolAnnotations {
        non_destructive_write()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, move |surface, cx| surface.prepare_asset(input, cx))?;
        Ok(text_response(value))
    }
}

// ---- batch_get ---------------------------------------------------------------

/// Read nodes from the open design document: pass `ids` for full node detail,
/// or omit them to list a page's node tree in compact form (`page` defaults to
/// the active page; `depth` limits recursion; `include_geometry` adds world
/// bounding boxes). A listing returns at most `limit` (default 200) of the
/// page's direct children starting at `offset`, and reports `child_count`,
/// `children_offset`, `children_limit` and `more_children`: page through a
/// wide page by calling again with `offset` advanced. A result over 262144
/// bytes is refused rather than truncated.
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
    /// How many of the page's direct children to skip before listing any
    /// (default 0). Applies to the listed page's own children only; nested
    /// levels are governed by `depth`. Ignored when `ids` is given.
    #[serde(default)]
    offset: Option<usize>,
    /// How many of the page's direct children to list, starting at `offset`
    /// (default 200). Applies to the listed page's own children only; nested
    /// levels are governed by `depth`. When the result says `more_children`,
    /// call again with `offset` advanced by this limit. Ignored when `ids` is
    /// given.
    #[serde(default)]
    limit: Option<usize>,
    /// `summary`, `style` or `raw`. With `ids` the default is `style`: the
    /// node's editable style in the ops' own vocabulary (send a value back in
    /// an op unchanged); `raw` is the internal record. A page listing defaults
    /// to `summary`; `style` merges each listed node's style into it.
    #[serde(default)]
    detail: Option<design_surface::NodeDetail>,
}

/// How a caller narrows a result that overran [`MAX_JSON_RESPONSE_BYTES`].
/// A listing must be told about `offset`/`limit` first: the caller may already
/// be at `depth: 1` with no geometry, and cannot pass `ids` it has not been
/// able to list.
fn narrowing_hint(listing: bool) -> &'static str {
    if listing {
        "Narrow it: page the children with `limit` and `offset` (e.g. `limit: 50`, then \
         `offset: 50` for the next window), and if that is still too large lower `depth` \
         (try 1) or set `include_geometry` to false."
    } else {
        "Narrow it: ask for fewer `ids` per call, or set `include_geometry` to false."
    }
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
            offset: args.offset,
            limit: args.limit,
            detail: args.detail,
        };
        let value = with_surface(cx, move |surface, cx| surface.get_nodes(query, cx))?;
        bounded_text_response(value, narrowing_hint(listing))
    }
}

// ---- batch_design ------------------------------------------------------------

/// Apply a batch of design ops to the open canvas as ONE undoable
/// transaction. Ops: create_node (frame/rectangle/ellipse/text), create_image
/// (base64 source), create_instance (of a component, by id or unique name),
/// set_props (name/position/size/opacity/fill/corner_radius/text/hidden/
/// locked), set_stroke, set_shadow, set_text_style, set_auto_layout,
/// set_index (z-order), rotate, align, distribute, group, frame_selection,
/// ungroup, duplicate, create_component, reparent, delete, select,
/// set_viewport, set_layout_child, create_variable_collection, add_variable_mode,
/// create_variable, set_variable_value, set_variable_mode, bind_variable,
/// unbind_variable, combine_variants, create_component_property,
/// bind_component_property, set_instance_property. Read get_design_system first.
/// Coordinates are world px, y down, x/y = top-left; ids are
/// exact node ids. If any op fails the whole batch rolls back and the result
/// names the failing op; created ids come back in `created`. Gradients,
/// per-run rich text are not ops yet: for those, read the page
/// .fnx with read_fnx_source, edit the file, and the canvas reloads (the change
/// is a git diff). Pass activity with the same stable mage name and agent_id
/// used in report_agent_activity so the user can follow each streamed step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct BatchDesignArgs {
    /// The ops to apply, in order.
    ops: Vec<DesignOp>,
    /// Undo-history label for the batch (e.g. "Add login card").
    #[serde(default)]
    label: Option<String>,
    /// Agent identity for the streamed steps. Reuse agent_id and agent_name
    /// from report_agent_activity; action and location update per operation.
    #[serde(default)]
    activity: Option<design_surface::AgentActivity>,
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
        let activity = input.activity;
        let apply = with_surface(cx, move |surface, cx| {
            Ok(surface.apply_streamed(ops, label, activity, cx))
        })?;
        let value = apply.await?;
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
    /// Exact animation clip id from get_editor_state.motion_clips.
    #[serde(default)]
    motion_clip: Option<String>,
    /// Time in milliseconds to sample (defaults to zero when a clip is given).
    #[serde(default)]
    playhead_ms: Option<u32>,
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
            motion_clip: args.motion_clip,
            playhead_ms: args.playhead_ms,
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

/// Validate a complete FNX or managed design-system JSON candidate without changing the project. If source
/// is omitted, validate the saved file. Errors include parser diagnostics;
/// repair them before writing a candidate with external file tools.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ValidateFnxSourceArgs {
    path: String,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Clone)]
struct ValidateFnxSourceTool;

impl McpServerTool for ValidateFnxSourceTool {
    type Input = ValidateFnxSourceArgs;
    type Output = ();

    const NAME: &'static str = "validate_fnx_source";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let validation = with_surface(cx, move |surface, cx| {
            let source = match input.source {
                Some(source) => source,
                None => surface
                    .read_source(Some(input.path.clone()), cx)?
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .context("the requested FNX source text is unavailable")?
                    .to_owned(),
            };
            Ok(surface.validate_source_edit(input.path, source, cx))
        })?;
        bounded_text_response(validation.await?, "Additional FNX diagnostics omitted.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent learns what a tool does only from its description; release
    /// builds would otherwise advertise an undescribed tool silently.
    #[test]
    fn every_live_tool_is_described() {
        macro_rules! descriptions {
            ($($tool:ident),+) => {
                vec![$((
                    <$tool as McpServerTool>::NAME,
                    context_server::listener::tool_description::<$tool>(),
                )),+]
            };
        }
        let tools = live_tools!(descriptions!());
        assert_eq!(tools.len(), 13);
        for (name, description) in tools {
            let description = description.unwrap_or_default();
            assert!(
                description.len() >= 20,
                "`{name}` needs a doc comment on its input type describing the tool"
            );
        }
    }

    #[test]
    fn batch_design_activity_is_optional_and_preserves_agent_identity() {
        let legacy: BatchDesignArgs = serde_json::from_value(serde_json::json!({
            "ops": [],
            "label": "Add login card"
        }))
        .expect("existing batch input remains valid");
        assert!(legacy.activity.is_none());

        let identified: BatchDesignArgs = serde_json::from_value(serde_json::json!({
            "ops": [],
            "activity": {
                "agent_id": "cornelius-design",
                "agent_name": "Cornelius",
                "action": "Add login card"
            }
        }))
        .expect("batch input accepts a stable agent identity");
        let activity = identified.activity.expect("the identity is retained");
        assert_eq!(activity.agent_id, "cornelius-design");
        assert_eq!(activity.agent_name, "Cornelius");

        let schema = serde_json::to_value(schemars::schema_for!(BatchDesignArgs))
            .expect("the input schema serializes");
        assert!(schema["properties"]["activity"].is_object());
        assert!(
            !schema["required"]
                .as_array()
                .expect("batch ops remain required")
                .iter()
                .any(|field| field == "activity")
        );
    }

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
        let notice = slice
            .notice("page.fnx")
            .expect("a cut line must be reported");
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
        let notice = slice
            .notice("page.fnx")
            .expect("an empty slice must be reported");
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

    /// A listing refused for size must be told to page. The caller that found
    /// this was already at `depth: 1` without geometry and had no ids to ask
    /// by, because listing is how ids are discovered.
    #[test]
    fn a_refused_listing_is_told_to_page_with_offset_and_limit() {
        let value = serde_json::json!({ "root": "n".repeat(MAX_JSON_RESPONSE_BYTES + 1) });
        let error = bounded_text_response(value, narrowing_hint(true))
            .expect_err("an oversized listing must be refused, not carried");
        let message = error.to_string();
        assert!(message.contains("`limit`"), "{message}");
        assert!(message.contains("`offset`"), "{message}");
        let pagination = message.find("`limit`").unwrap_or(usize::MAX);
        let depth = message.find("`depth`").unwrap_or(usize::MAX);
        assert!(pagination < depth, "pagination must come first: {message}");

        assert!(!narrowing_hint(false).contains("`offset`"));
    }
}
