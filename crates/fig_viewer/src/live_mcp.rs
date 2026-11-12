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
use design_surface::{
    BatchDesignArgs, BatchGetArgs, GetEditorStateArgs, GetGuidelinesArgs, GetScreenshotArgs,
    ImportImageArgs, ListCommentsArgs, MAX_JSON_RESPONSE_BYTES, NodeQuery, ReadFnxSourceArgs,
    ReplyCommentArgs, ScreenshotTarget, ValidateFnxSourceArgs, narrowing_hint, slice_source,
};
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
            CheckProjectTool,
            ImportImageTool,
            ListCommentsTool,
            ReplyCommentTool,
            AddCommentTool,
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
        |workspace, _: &fanta_actions::fanta::ConnectExternalAgent, _, cx| {
            let executable = match std::env::current_exe() {
                Ok(executable) => executable,
                Err(error) => {
                    log::error!("locating the Fanta executable failed: {error:#}");
                    crate::view::show_workspace_canvas_notice(
                        workspace,
                        format!("Fanta could not locate its own executable: {error}"),
                        cx,
                    );
                    return;
                }
            };
            if !FantaLiveMcpSettings::get_global(cx).enabled {
                crate::view::show_workspace_canvas_notice(workspace, "Enable fanta_live_mcp.enabled to connect an external agent".into(), cx);
                return;
            }
            let instructions = connection_instructions(&executable, paths::data_dir());
            cx.write_to_clipboard(ClipboardItem::new_string(instructions));
            crate::view::show_workspace_canvas_notice(
                workspace,
                "Connection instructions copied for Codex and Claude Code. Keep this project open in Fanta.".into(),
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
         Verify with /mcp and call get_editor_state. Edit the returned source files with file tools \
         (validate legacy .fnx candidates with validate_fnx_source), inspect the canvas with batch_get \
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
            cx.background_spawn(async move { fanta_headless::assets::decode_image_source(&source) })
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

/// Place a new canvas comment as an agent: a review finding, a question or a
/// note. Anchor it to a layer with `node` (prefixed id; `offset` in the node's
/// local units, its centre by default) so the pin follows the layer, or give a
/// `world` position. Review findings carry `severity` (info, minor, major,
/// blocker), `category` (layout, typography, color, accessibility,
/// consistency, content, interaction, motion, other), the run's
/// `review_run_id`, and optionally a `suggested_fix` (`summary` plus the
/// content-editing batch_design `ops` that would address it, excluding select
/// and set_viewport) the user can apply.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AddCommentArgs {
    #[serde(flatten)]
    draft: design_surface::CommentDraft,
}

#[derive(Clone)]
struct AddCommentTool;

impl McpServerTool for AddCommentTool {
    type Input = AddCommentArgs;
    type Output = ();
    const NAME: &'static str = "add_comment";
    fn annotations(&self) -> ToolAnnotations {
        non_destructive_write()
    }
    async fn run(&self, input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let value = with_surface(cx, move |surface, cx| surface.add_comment(input.draft, cx))?;
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

// ---- check_project -----------------------------------------------------------

#[derive(Clone)]
struct CheckProjectTool;

impl McpServerTool for CheckProjectTool {
    type Input = OrDefault<design_surface::CheckProjectArgs>;
    type Output = ();

    const NAME: &'static str = "check_project";

    fn annotations(&self) -> ToolAnnotations {
        read_only()
    }

    async fn run(&self, _input: Self::Input, cx: &mut AsyncApp) -> Result<ToolResponse<()>> {
        let root = with_surface(cx, |surface, cx| surface.project_root(cx))?
            .context("the open design is not saved as a project yet; save it first")?;
        // Spec 0009 R5: the same JSON `fanta-check --json` prints.
        let text = cx
            .background_spawn(async move {
                fanta_format::findings_json(&fanta_format::check_project(&root)?)
            })
            .await?;
        Ok(ToolResponse {
            content: vec![context_server::types::ToolResponseContent::Text { text }],
            structured_content: (),
        })
    }
}

/// Inspect reusable component properties/variant sets, variable collections, modes,
/// paginated components, variant sets and typed token values, and optional
/// selected-node bindings. Use offset and limit (default 100, maximum 200),
/// advancing offset while the corresponding more_* flag is true.
/// Read before creating a design system.
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
            "Lower limit; use offset for subsequent component, variant-set and variable pages. Choose a collection to filter variables.",
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
        assert_eq!(tools.len(), 15);
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
