use std::{io::Cursor, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use client::{Client, ClientSettings};
use design_surface::{DesignOp, ScreenshotTarget};
use futures::{AsyncReadExt as _, StreamExt as _};
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, Image,
    ImageFormat, MouseButton, ObjectFit, PathPromptOptions, Pixels, Render, SharedString, Task,
    WeakEntity, Window, actions, canvas, img,
};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde::Deserialize;
use serde_json::{Value, json};
use settings::Settings as _;
use sha2::{Digest as _, Sha256};
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, IconPosition, prelude::*};
use ui_input::InputField;
use util::ResultExt as _;
use workspace::{Item, MultiWorkspace, Workspace, item::ItemEvent};

use crate::{FigItem, FigView, agent_surface, generation_media};

const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_VECTOR_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_VECTOR_SVG_BYTES: usize = 128 * 1024;
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const PREVIEW_SIZE: u32 = 1200;
const HISTORY_LIMIT: usize = 12;

actions!(
    fanta,
    [
        GenerateImage,
        GenerateVideo,
        GenerateVector,
        GenerateDesign,
        GenerateMasks
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenerationMode {
    Image,
    Video,
    Vector,
    Design,
    Masks,
}

impl GenerationMode {
    const ALL: [Self; 5] = [
        Self::Image,
        Self::Video,
        Self::Vector,
        Self::Design,
        Self::Masks,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Vector => "Vector",
            Self::Design => "Design",
            Self::Masks => "Masks",
        }
    }

    fn accepts(self, model: &GenerationModel) -> bool {
        match self {
            Self::Image => model.kind == "image",
            Self::Video => model.kind == "video",
            Self::Vector => matches!(model.kind.as_str(), "chat" | "svg" | "vectorize"),
            Self::Masks => model.kind == "segment",
            Self::Design => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorOperation {
    Create,
    Trace,
}

impl VectorOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create from prompt",
            Self::Trace => "Trace an image",
        }
    }

    fn accepts(self, model: &GenerationModel) -> bool {
        match self {
            Self::Create => model.kind == "chat" && model.id.starts_with("claude-"),
            Self::Trace => matches!(model.kind.as_str(), "svg" | "vectorize"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct GenerationModel {
    id: String,
    kind: String,
    #[serde(default)]
    credits_per_output: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    capabilities: Value,
}

impl GenerationModel {
    fn label(&self) -> String {
        match self.id.as_str() {
            "claude-sonnet-5" => "Claude Sonnet 5".into(),
            "claude-haiku-4-5" => "Claude Haiku 4.5".into(),
            "claude-opus-4-8" => "Claude Opus 4.8".into(),
            "claude-fable-5" => "Claude Fable 5".into(),
            _ => self.id.clone(),
        }
    }

    fn sizes(&self) -> Vec<String> {
        self.capabilities["size"]["values"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|value| parse_size(value).is_ok())
            .map(str::to_owned)
            .collect()
    }

    fn supports_inpaint(&self) -> bool {
        self.capabilities["operations"]
            .as_array()
            .is_some_and(|operations| operations.iter().any(|operation| operation == "inpaint"))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct GenerationResponse {
    id: String,
    status: String,
    #[serde(default)]
    output: Vec<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    billed_credits: Option<f64>,
    #[serde(default)]
    error: Option<Value>,
}

impl GenerationResponse {
    fn pending(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "processing" | "warming")
    }
}

#[derive(Clone)]
struct MediaOutput {
    label: String,
    mime: String,
    location: MediaLocation,
    mask: bool,
}

#[derive(Clone)]
enum MediaLocation {
    Inline(Arc<[u8]>),
    Url(String),
}

#[derive(Clone)]
struct Preview {
    image: Arc<Image>,
    width: u32,
    height: u32,
}

#[derive(Clone)]
struct SourceImage {
    reference: Value,
    name: String,
    preview: Preview,
}

#[derive(Clone, Debug)]
struct MaskPoint {
    x: f64,
    y: f64,
    positive: bool,
}

#[derive(Clone)]
struct RunSummary {
    result: RunResult,
    model: String,
    prompt: String,
    source: Option<SourceImage>,
}

#[derive(Clone)]
enum RunResult {
    Generation {
        id: String,
    },
    VectorMessage {
        message_id: Option<String>,
        svg: Arc<[u8]>,
    },
}

impl RunSummary {
    fn generation_id(&self) -> Option<&str> {
        match &self.result {
            RunResult::Generation { id } => Some(id),
            RunResult::VectorMessage { .. } => None,
        }
    }

    fn provenance(&self) -> Value {
        let mut value = json!({"model": self.model, "prompt": self.prompt});
        match &self.result {
            RunResult::Generation { id } => value["generation_id"] = json!(id),
            RunResult::VectorMessage { message_id, .. } => {
                value["operation"] = json!("generate_vectors");
                if let Some(id) = message_id {
                    value["message_id"] = json!(id);
                }
            }
        }
        value
    }
}

#[derive(Clone)]
struct Submission {
    key: String,
    request: Value,
    model: String,
    prompt: String,
    source: Option<SourceImage>,
    account: Option<Arc<str>>,
}

#[derive(Debug)]
struct ApiRejected {
    status: u16,
    message: String,
}
impl std::fmt::Display for ApiRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for ApiRejected {}

enum PreparedPlacement {
    Image(String, u32, u32),
    Vector(generation_media::VectorArtwork),
    Video(Arc<[u8]>, generation_media::VideoMetadata),
}

struct GenerationWorkspace {
    workspace: WeakEntity<Workspace>,
    canvas_item: Option<WeakEntity<FigItem>>,
    client: Arc<Client>,
    base_url: String,
    mode: GenerationMode,
    vector_operation: VectorOperation,
    models: Vec<GenerationModel>,
    selected_model: Option<String>,
    size: String,
    prompt: Entity<InputField>,
    negative: Entity<InputField>,
    seed: Entity<InputField>,
    steps: Entity<InputField>,
    guidance: Entity<InputField>,
    duration: Entity<InputField>,
    strength: Entity<InputField>,
    source: Option<SourceImage>,
    mask: Option<Arc<[u8]>>,
    points: Vec<MaskPoint>,
    exclude_points: bool,
    source_bounds: Option<Bounds<Pixels>>,
    outputs: Vec<MediaOutput>,
    selected_output: usize,
    preview: Option<Preview>,
    history: Vec<RunSummary>,
    active_run: Option<RunSummary>,
    pending: bool,
    playback_file: Option<tempfile::TempPath>,
    unresolved_submission: Option<Submission>,
    _account_task: Task<()>,
    account: Option<Arc<str>>,
    status: SharedString,
    error: Option<SharedString>,
    catalog_task: Option<Task<()>>,
    catalog_request: Option<uuid::Uuid>,
    task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
}

pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &GenerateImage, window, cx| {
        open(workspace, GenerationMode::Image, None, window, cx);
    });
    workspace.register_action(|workspace, _: &GenerateVideo, window, cx| {
        open(workspace, GenerationMode::Video, None, window, cx);
    });
    workspace.register_action(|workspace, _: &GenerateVector, window, cx| {
        open(workspace, GenerationMode::Vector, None, window, cx);
    });
    workspace.register_action(|workspace, _: &GenerateDesign, window, cx| {
        open(workspace, GenerationMode::Design, None, window, cx);
    });
    workspace.register_action(|workspace, _: &GenerateMasks, window, cx| {
        open(workspace, GenerationMode::Masks, None, window, cx);
    });
}

fn open(
    workspace: &mut Workspace,
    mode: GenerationMode,
    item: Option<WeakEntity<FigItem>>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let item = item.or_else(|| {
        workspace
            .active_item_as::<FigView>(cx)
            .map(|view| view.read(cx).item().downgrade())
    });
    let workspace_handle = cx.entity().downgrade();
    let view = cx.new(|cx| GenerationWorkspace::new(workspace_handle, item, mode, window, cx));
    workspace.add_item_to_active_pane(Box::new(view), None, true, window, cx);
}

pub(crate) fn open_from_canvas(
    mode: GenerationMode,
    item: WeakEntity<FigItem>,
    window: &mut Window,
    cx: &mut App,
) {
    let workspace = window
        .root::<MultiWorkspace>()
        .flatten()
        .map(|root| root.read(cx).workspace().clone());
    if let Some(workspace) = workspace {
        window.defer(cx, move |window, cx| {
            workspace.update(cx, |workspace, cx| {
                open(workspace, mode, Some(item), window, cx)
            });
        });
    }
}

impl GenerationWorkspace {
    fn new(
        workspace: WeakEntity<Workspace>,
        canvas_item: Option<WeakEntity<FigItem>>,
        mode: GenerationMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prompt = cx.new(|cx| InputField::new(window, cx, "Describe what you want to create"));
        prompt
            .read(cx)
            .editor()
            .clone()
            .set_multiline(Some(6), window, cx);
        let client = Client::global(cx);
        let mut account_status = client.status();
        let account_task = cx.spawn(async move |this, cx| {
            while account_status.next().await.is_some() {
                if this
                    .update(cx, |this, cx| {
                        this.sync_account(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = Self {
            workspace,
            canvas_item,
            client: Client::global(cx),
            base_url: ClientSettings::get_global(cx)
                .server_url
                .trim_end_matches('/')
                .to_owned(),
            mode,
            vector_operation: VectorOperation::Create,
            models: Vec::new(),
            selected_model: None,
            size: "1024x1024".into(),
            prompt,
            negative: cx.new(|cx| InputField::new(window, cx, "What to avoid (optional)")),
            seed: cx.new(|cx| InputField::new(window, cx, "Random")),
            steps: cx.new(|cx| InputField::new(window, cx, "Model default")),
            guidance: cx.new(|cx| InputField::new(window, cx, "Model default")),
            duration: cx.new(|cx| InputField::new(window, cx, "5")),
            strength: cx.new(|cx| InputField::new(window, cx, "0.8")),
            source: None,
            mask: None,
            points: Vec::new(),
            exclude_points: false,
            source_bounds: None,
            outputs: Vec::new(),
            selected_output: 0,
            preview: None,
            history: Vec::new(),
            active_run: None,
            pending: false,
            playback_file: None,
            unresolved_submission: None,
            _account_task: account_task,
            account: client.account_access_token(),
            status: "Choose a model and describe your idea.".into(),
            error: None,
            catalog_task: None,
            catalog_request: None,
            task: None,
            preview_task: None,
        };
        this.refresh_catalog(cx);
        this
    }

    fn sync_account(&mut self, cx: &mut Context<Self>) {
        let account = self.client.account_access_token();
        if account == self.account {
            return;
        }
        self.account = account;
        self.catalog_task = None;
        self.catalog_request = None;
        self.models.clear();
        self.selected_model = None;
        self.task = None;
        self.preview_task = None;
        self.unresolved_submission = None;
        self.source = None;
        self.mask = None;
        self.points.clear();
        self.outputs.clear();
        self.preview = None;
        self.history.clear();
        self.active_run = None;
        self.pending = false;
        self.playback_file = None;
        self.status = "Account changed. Choose a source and start a new experiment.".into();
        self.error = None;
        self.refresh_catalog(cx);
        cx.notify();
    }

    fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        let account = self.client.account_access_token();
        if account != self.account {
            self.sync_account(cx);
            return;
        }
        let Some(account) = account else {
            self.status =
                "Sign in to Fanta to discover models and use your account credits.".into();
            self.error = None;
            cx.notify();
            return;
        };
        if self.catalog_task.is_some() {
            return;
        }
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let request_id = uuid::Uuid::new_v4();
        self.catalog_request = Some(request_id);
        self.catalog_task = Some(cx.spawn(async move |this, cx| {
            let result = fetch_catalog(&client, &base_url, Some(&account)).await;
            this.update(cx, |this, cx| {
                if this.catalog_request != Some(request_id) {
                    return;
                }
                if this.client.account_access_token().as_deref() != Some(account.as_ref()) {
                    this.sync_account(cx);
                    return;
                }
                this.catalog_task = None;
                this.catalog_request = None;
                match result {
                    Ok(models) => {
                        this.models = models;
                        this.choose_default_model();
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn accepts_model(&self, model: &GenerationModel) -> bool {
        self.mode.accepts(model)
            && (self.mode != GenerationMode::Vector || self.vector_operation.accepts(model))
    }

    fn choose_default_model(&mut self) {
        if !self.models.iter().any(|model| {
            self.accepts_model(model) && self.selected_model.as_deref() == Some(&model.id)
        }) {
            self.selected_model = self
                .models
                .iter()
                .filter(|model| self.accepts_model(model))
                .min_by_key(|model| {
                    (
                        if self.mode == GenerationMode::Vector
                            && self.vector_operation == VectorOperation::Create
                        {
                            match model.id.as_str() {
                                "claude-sonnet-5" => 0,
                                "claude-haiku-4-5" => 1,
                                "claude-opus-4-8" => 2,
                                _ => 3,
                            }
                        } else {
                            0
                        },
                        !model.id.starts_with("fanta-"),
                        model.supports_inpaint(),
                    )
                })
                .map(|model| model.id.clone());
        }
        if let Some(model) = self.model() {
            let sizes = model.sizes();
            if !sizes.is_empty() && !sizes.contains(&self.size) {
                self.size = model.capabilities["size"]["default"]
                    .as_str()
                    .filter(|size| sizes.iter().any(|candidate| candidate == size))
                    .map(str::to_owned)
                    .unwrap_or_else(|| sizes[0].clone());
            }
        }
    }

    fn model(&self) -> Option<&GenerationModel> {
        self.models
            .iter()
            .find(|model| Some(model.id.as_str()) == self.selected_model.as_deref())
    }

    fn set_mode(&mut self, mode: GenerationMode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode != GenerationMode::Image {
            self.mask = None;
        }
        self.choose_default_model();
        self.error = None;
        cx.emit(ItemEvent::UpdateTab);
        cx.notify();
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        log::warn!("Fanta generation workspace: {error:#}");
        self.error = Some(error.to_string().into());
        self.task = None;
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let client = self.client.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = client.sign_in_with_optional_connect(true, cx).await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(()) => {
                        this.error = None;
                        this.status = "Your Fanta account is connected.".into();
                        this.refresh_catalog(cx);
                    }
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn request(&self, cx: &App) -> Result<Value> {
        let model = self
            .model()
            .context("No model is available. Refresh the model list.")?;
        build_request(
            model,
            &self.prompt.read(cx).text(cx),
            &self.negative.read(cx).text(cx),
            &self.size,
            &self.seed.read(cx).text(cx),
            &self.steps.read(cx).text(cx),
            &self.guidance.read(cx).text(cx),
            &self.duration.read(cx).text(cx),
            &self.strength.read(cx).text(cx),
            self.source.as_ref().map(|source| source.reference.clone()),
            self.mask.as_deref(),
            &self.points,
        )
    }

    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        if self.mode == GenerationMode::Design {
            self.generate_design(window, cx);
            return;
        }
        if self.mode == GenerationMode::Vector && self.vector_operation == VectorOperation::Create {
            self.generate_vectors(cx);
            return;
        }
        let request = match self.request(cx) {
            Ok(request) => request,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        if self.unresolved_submission.is_some() {
            self.fail(anyhow!("Check the previous submission using Retry same request before starting another generation."), cx);
            return;
        }
        self.submit(
            Submission {
                key: uuid::Uuid::new_v4().to_string(),
                request,
                prompt: self.prompt.read(cx).text(cx),
                model: self.selected_model.clone().unwrap_or_default(),
                source: self.source.clone(),
                account: self.client.account_access_token(),
            },
            cx,
        );
    }

    fn generate_vectors(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(model) = self.model().cloned() else {
            self.fail(
                anyhow!("No vector creation model is available. Refresh the model list."),
                cx,
            );
            return;
        };
        let prompt = self.prompt.read(cx).text(cx);
        let request = match build_vector_message_request(&model, &prompt, &self.size) {
            Ok(request) => request,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = client.account_access_token();
        self.pending = false;
        self.outputs.clear();
        self.preview = None;
        self.preview_task = None;
        self.active_run = None;
        self.error = None;
        self.status = "Creating vector artwork…".into();
        self.task = Some(cx.spawn(async move |this, cx| {
            let request = api_json(&client, &base_url, Method::POST, "/v1/messages", Some(request), None, account.as_deref());
            let deadline = cx.background_executor().timer(Duration::from_secs(125));
            futures::pin_mut!(request, deadline);
            let response = match futures::future::select(request, deadline).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(anyhow!("The vector request timed out.")),
            };
            let result = response.and_then(|response| parse_vector_message(&response));
            this.update(cx, |this, cx| {
                this.task = None;
                if this.client.account_access_token() != account { this.sync_account(cx); return; }
                match result {
                    Ok((message_id, svg)) => {
                        let run = RunSummary {
                            result: RunResult::VectorMessage { message_id, svg: svg.clone() },
                            model: model.id, prompt, source: None,
                        };
                        this.active_run = Some(run.clone());
                        this.history.insert(0, run);
                        this.history.truncate(HISTORY_LIMIT);
                        this.outputs = vec![vector_output(svg)];
                        this.selected_output = 0;
                        this.status = "Vector artwork is ready. AI usage is billed to your Fanta credits.".into();
                        this.load_preview(cx);
                    }
                    Err(error) => {
                        let rejected = error.downcast_ref::<ApiRejected>().is_some_and(|error| error.status < 500);
                        this.status = "Vector creation finished without a usable result.".into();
                        let message = if rejected { error.to_string() } else {
                            format!("{error} Some AI usage may already be billed. Generating again starts a new request.")
                        };
                        this.error = Some(message.into());
                    }
                }
                cx.notify();
            }).log_err();
        }));
        cx.notify();
    }

    fn submit(&mut self, submission: Submission, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let retrying = self.unresolved_submission.is_some();
        self.unresolved_submission = Some(submission.clone());
        let Submission {
            key,
            request,
            model,
            prompt,
            source,
            account,
        } = submission;
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        self.pending = false;
        self.error = None;
        self.status = "Sending your request…".into();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = api_json(
                &client,
                &base_url,
                Method::POST,
                "/v1/generations",
                Some(request),
                Some(&key),
                account.as_deref(),
            )
            .await
            .and_then(|value| {
                serde_json::from_value::<GenerationResponse>(value).map_err(Into::into)
            });
            match result {
                Ok(response) => {
                    let run = RunSummary {
                        result: RunResult::Generation {
                            id: response.id.clone(),
                        },
                        model,
                        prompt,
                        source,
                    };
                    this.update(cx, |this, cx| {
                        this.unresolved_submission = None;
                        this.active_run = Some(run.clone());
                        this.history.insert(0, run);
                        this.history.truncate(HISTORY_LIMIT);
                        this.outputs.clear();
                        this.preview = None;
                        this.preview_task = None;
                        this.accept_response(&response, cx);
                    })
                    .log_err();
                    if response.pending() {
                        Self::poll(
                            this.clone(),
                            client,
                            base_url,
                            response.id,
                            account.clone(),
                            cx,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        if error.downcast_ref::<ApiRejected>().is_some_and(|error| {
                            error.status < 500 && error.status != 409 && !retrying
                        }) {
                            this.unresolved_submission = None;
                        }
                        this.fail(error, cx);
                    })
                    .log_err();
                }
            }
            this.update(cx, |this, cx| {
                this.task = None;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    async fn poll(
        this: WeakEntity<Self>,
        client: Arc<Client>,
        base_url: String,
        id: String,
        account: Option<Arc<str>>,
        cx: &mut gpui::AsyncApp,
    ) {
        for _ in 0..300 {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let result = api_json(
                &client,
                &base_url,
                Method::GET,
                &format!("/v1/generations/{id}"),
                None,
                None,
                account.as_deref(),
            )
            .await
            .and_then(|value| {
                serde_json::from_value::<GenerationResponse>(value).map_err(Into::into)
            });
            match result {
                Ok(response) => {
                    let pending = response.pending();
                    if this
                        .update(cx, |this, cx| this.accept_response(&response, cx))
                        .is_err()
                    {
                        return;
                    }
                    if !pending {
                        return;
                    }
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(
                            format!(
                                "Status check failed: {error}. Use Check status to resume this job."
                            )
                            .into(),
                        );
                        cx.notify();
                    })
                    .log_err();
                    return;
                }
            }
        }
        this.update(cx, |this, cx| {
            this.status = "This job is still running. Check its status again in a moment.".into();
            cx.notify();
        })
        .log_err();
    }

    fn check_status(&mut self, run: RunSummary, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = self.client.account_access_token();
        self.outputs.clear();
        self.preview = None;
        self.preview_task = None;
        self.pending = false;
        self.active_run = Some(run.clone());
        if let RunResult::VectorMessage { svg, .. } = &run.result {
            self.outputs = vec![vector_output(svg.clone())];
            self.selected_output = 0;
            self.error = None;
            self.status = "Vector artwork restored from this tab's history.".into();
            self.load_preview(cx);
            cx.notify();
            return;
        }
        let Some(generation_id) = run.generation_id().map(str::to_owned) else {
            return;
        };
        self.status = "Checking this generation…".into();
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = api_json(
                &client,
                &base_url,
                Method::GET,
                &format!("/v1/generations/{generation_id}"),
                None,
                None,
                account.as_deref(),
            )
            .await
            .and_then(|value| {
                serde_json::from_value::<GenerationResponse>(value).map_err(Into::into)
            });
            match result {
                Ok(response) => {
                    this.update(cx, |this, cx| this.accept_response(&response, cx))
                        .log_err();
                    if response.pending() {
                        Self::poll(
                            this.clone(),
                            client,
                            base_url,
                            response.id,
                            account.clone(),
                            cx,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    this.update(cx, |this, cx| this.fail(error, cx)).log_err();
                }
            }
            this.update(cx, |this, cx| {
                this.task = None;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn accept_response(&mut self, response: &GenerationResponse, cx: &mut Context<Self>) {
        self.pending = response.pending();
        if self.pending {
            self.status = if response.status == "warming" {
                "The model is warming up…"
            } else {
                "Creating your result…"
            }
            .into();
        } else if response.status == "succeeded" {
            match normalize_outputs(&response.output) {
                Ok(outputs) if !outputs.is_empty() => {
                    self.outputs = outputs;
                    self.selected_output = 0;
                    self.error = None;
                    let mut status =
                        format!("Ready · {}", response.model.as_deref().unwrap_or("Fanta"));
                    if let Some(credits) = response.billed_credits {
                        status.push_str(&format!(" · {credits:.2} credits"));
                    }
                    if let Some(seed) = response.seed {
                        status.push_str(&format!(" · Seed {seed}"));
                    }
                    self.status = status.into();
                    self.load_preview(cx);
                }
                Ok(_) => {
                    self.error = Some(
                        "The model returned no supported media. Try another model or prompt."
                            .into(),
                    )
                }
                Err(error) => self.error = Some(error.to_string().into()),
            }
        } else {
            self.error = Some(
                response
                    .error
                    .as_ref()
                    .map(error_message)
                    .unwrap_or_else(|| {
                        format!(
                            "Generation {}. Try a different prompt or model.",
                            response.status
                        )
                    })
                    .into(),
            );
            self.status = "Generation finished without a result.".into();
        }
        cx.notify();
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = None;
        self.preview_task = None;
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        if !output.mime.starts_with("image/") {
            return;
        }
        let client = self.client.clone();
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes = media_bytes(&client, &output.location).await?;
                cx.background_spawn(async move { make_preview(&bytes, &output.mime) })
                    .await
            }
            .await;
            this.update(cx, |this, cx| {
                this.preview_task = None;
                match result {
                    Ok(preview) => this.preview = Some(preview),
                    Err(error) => {
                        this.error = Some(
                            format!("Preview unavailable: {error}. You can still save the result.")
                                .into(),
                        )
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn choose_source(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose source image".into()),
        });
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<Option<SourceImage>> = async {
                let Some(path) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                let (bytes, name, mime, preview) = cx
                    .background_spawn(async move {
                        ensure!(
                            std::fs::metadata(&path)?.len() <= MAX_MEDIA_BYTES as u64,
                            "Choose an image smaller than 100 MB."
                        );
                        let bytes = std::fs::read(&path)?;
                        let format = image::guess_format(&bytes)
                            .context("Choose a PNG, JPEG, or WebP image.")?;
                        let mime = match format {
                            image::ImageFormat::Png => "image/png",
                            image::ImageFormat::Jpeg => "image/jpeg",
                            image::ImageFormat::WebP => "image/webp",
                            _ => bail!("Choose a PNG, JPEG, or WebP image."),
                        };
                        let preview = make_preview(&bytes, mime)?;
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Source image".into());
                        Ok::<_, anyhow::Error>((bytes, name, mime, preview))
                    })
                    .await?;
                this.update(cx, |this, cx| {
                    this.status = "Uploading source image…".into();
                    cx.notify();
                })?;
                let asset = upload_image(&client, &base_url, bytes, &name, mime, &preview).await?;
                Ok(Some(SourceImage {
                    reference: json!({"asset_id": asset}),
                    name,
                    preview,
                }))
            }
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(Some(source)) => this.set_source(source, cx),
                    Ok(None) => {}
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn capture_canvas(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let result = (|| {
            let item = self
                .canvas_item
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .context("Open this tool from a design canvas first.")?;
            agent_surface::set_active_item(item.downgrade(), cx);
            let surface = design_surface::active(cx).context("No design canvas is available.")?;
            let node = item
                .read(cx)
                .doc()
                .and_then(|doc| doc.selection.iter().next().map(|id| id.to_string()));
            Ok::<_, anyhow::Error>(surface.screenshot(
                ScreenshotTarget {
                    page: None,
                    node,
                    max_dimension: Some(2048),
                },
                cx,
            ))
        })();
        let screenshot = match result {
            Ok(task) => task,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        self.status = "Capturing the design…".into();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes = screenshot.await?;
                let (bytes, preview) = cx
                    .background_spawn(async move {
                        let preview = make_preview(&bytes, "image/png")?;
                        Ok::<_, anyhow::Error>((bytes, preview))
                    })
                    .await?;
                let asset = upload_image(
                    &client,
                    &base_url,
                    bytes,
                    "Canvas reference.png",
                    "image/png",
                    &preview,
                )
                .await?;
                Ok::<_, anyhow::Error>(SourceImage {
                    reference: json!({"asset_id":asset}),
                    name: "Canvas reference".into(),
                    preview,
                })
            }
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(source) => this.set_source(source, cx),
                    Err(error) => this.fail(error, cx),
                }
            })
            .log_err();
        }));
        cx.notify();
    }

    fn set_source(&mut self, source: SourceImage, cx: &mut Context<Self>) {
        self.source = Some(source);
        self.mask = None;
        self.points.clear();
        self.source_bounds = None;
        self.error = None;
        self.status = "Source ready. Describe your changes or select mask points.".into();
        cx.notify();
    }

    fn use_result(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self.outputs.get(self.selected_output) else {
            return;
        };
        if output.mask {
            if let MediaLocation::Inline(bytes) = &output.location {
                let Some(source) = self.active_run.as_ref().and_then(|run| run.source.clone())
                else {
                    self.fail(anyhow!("The source image for this mask is no longer available. Generate masks from a new source."), cx);
                    return;
                };
                self.source = Some(source);
                self.mask = Some(bytes.clone());
                self.mode = GenerationMode::Image;
                self.selected_model = self
                    .models
                    .iter()
                    .find(|model| model.kind == "image" && model.supports_inpaint())
                    .map(|model| model.id.clone());
                self.status =
                    "Mask selected. Describe the change; white areas will be edited.".into();
                cx.emit(ItemEvent::UpdateTab);
                cx.notify();
            }
            return;
        }
        if self.selected_output > 0 {
            let Some(preview) = self.preview.clone() else {
                return;
            };
            let output = output.clone();
            let client = self.client.clone();
            let base_url = self.base_url.clone();
            self.task = Some(cx.spawn(async move |this, cx| {
                let result = async {
                    let bytes = media_bytes(&client, &output.location).await?;
                    let asset = upload_image(
                        &client,
                        &base_url,
                        bytes.to_vec(),
                        "Generated source",
                        &output.mime,
                        &preview,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(SourceImage {
                        reference: json!({"asset_id":asset}),
                        name: "Generated image".into(),
                        preview,
                    })
                }
                .await;
                this.update(cx, |this, cx| {
                    this.task = None;
                    match result {
                        Ok(source) => this.set_source(source, cx),
                        Err(error) => this.fail(error, cx),
                    }
                })
                .log_err();
            }));
            cx.notify();
            return;
        }
        if let (Some(preview), Some(run)) = (self.preview.clone(), self.active_run.clone()) {
            let Some(generation_id) = run.generation_id() else {
                return;
            };
            self.set_source(
                SourceImage {
                    reference: json!({"generation_id": generation_id}),
                    name: "Generated image".into(),
                    preview,
                },
                cx,
            );
        }
    }

    fn remove_background(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self
            .outputs
            .get(self.selected_output)
            .filter(|output| output.mask)
            .cloned()
        else {
            return;
        };
        let Some(source) = self.active_run.as_ref().and_then(|run| run.source.clone()) else {
            return;
        };
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = client.account_access_token();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let location = if let Some(id) = source.reference["asset_id"].as_str() {
                    let asset = api_json(&client, &base_url, Method::GET, &format!("/v1/assets/{id}"), None, None, account.as_deref()).await?;
                    MediaLocation::Url(asset["url"].as_str().context("The source image is no longer available.")?.to_owned())
                } else if let Some(id) = source.reference["generation_id"].as_str() {
                    let generation = api_json(&client, &base_url, Method::GET, &format!("/v1/generations/{id}"), None, None, account.as_deref()).await?;
                    let outputs: Vec<Value> = serde_json::from_value(generation["output"].clone())?;
                    normalize_outputs(&outputs)?.into_iter().next().context("The original image is unavailable.")?.location
                } else { bail!("The original source image is unavailable."); };
                let image = media_bytes(&client, &location).await?;
                let mask = media_bytes(&client, &output.location).await?;
                cx.background_spawn(async move { cutout_image(&image, &mask) }).await
            }.await;
            this.update(cx, |this, cx| {
                this.task = None;
                if this.client.account_access_token() != account { this.sync_account(cx); return; }
                match result {
                    Ok(bytes) => {
                        this.outputs.push(MediaOutput { label: "Background removed".into(), mime: "image/png".into(), location: MediaLocation::Inline(bytes.into()), mask: false });
                        this.selected_output = this.outputs.len() - 1;
                        this.status = "Background removed. Save the transparent image or add it to your design.".into();
                        this.load_preview(cx);
                    }
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            }).log_err();
        }));
        cx.notify();
    }

    fn save_output(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        let extension = match output.mime.as_str() {
            "image/svg+xml" => "svg",
            "video/mp4" => "mp4",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "png",
        };
        let name = format!("fanta-generation.{extension}");
        let path = cx.prompt_for_new_path(&PathBuf::from(paths::home_dir().as_path()), Some(&name));
        let client = self.client.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<bool> = async {
                let Some(path) = path.await?? else {
                    return Ok(false);
                };
                let bytes = media_bytes(&client, &output.location).await?;
                cx.background_spawn(async move {
                    std::fs::write(path, bytes).context("The result could not be saved")
                })
                .await?;
                Ok(true)
            }
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(true) => this.status = "Result saved.".into(),
                    Ok(false) => {}
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn play_output(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        let client = self.client.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes = media_bytes(&client, &output.location).await?;
                cx.background_spawn(async move {
                    generation_media::mp4_metadata(&bytes)?;
                    let file = tempfile::Builder::new()
                        .prefix("fanta-generation-")
                        .suffix(".mp4")
                        .tempfile()?;
                    std::fs::write(file.path(), &bytes)?;
                    Ok::<_, anyhow::Error>(file.into_temp_path())
                })
                .await
            }
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(file) => {
                        cx.open_with_system(&file);
                        this.playback_file = Some(file);
                        this.status = "Opened in your video player.".into();
                    }
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn place_output(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        let Some(item) = self.canvas_item.clone() else {
            self.fail(
                anyhow!("Open this tool from a design canvas to place the result."),
                cx,
            );
            return;
        };
        let client = self.client.clone();
        let run = self.active_run.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes = media_bytes(&client, &output.location).await?;
                let media = cx.background_spawn(async move {
                    if output.mime == "image/svg+xml" {
                        Ok::<_, anyhow::Error>(PreparedPlacement::Vector(generation_media::parse_svg(&bytes)?))
                    } else if output.mime.starts_with("video/") {
                        let metadata = generation_media::mp4_metadata(&bytes)?;
                        Ok(PreparedPlacement::Video(bytes, metadata))
                    } else {
                        let preview = make_preview(&bytes, &output.mime)?;
                        Ok(PreparedPlacement::Image(STANDARD.encode(bytes), preview.width, preview.height))
                    }
                }).await?;
                cx.update(|cx| {
                    let item = item.upgrade().context("The source design was closed. Save the result and open a design to place it.")?;
                    agent_surface::set_active_item(item.downgrade(), cx);
                    let surface = design_surface::active(cx).context("The design canvas is unavailable.")?;
                    let (width, height) = match &media {
                        PreparedPlacement::Image(_, width, height) => (*width as f64, *height as f64),
                        PreparedPlacement::Vector(artwork) => (artwork.width, artwork.height),
                        PreparedPlacement::Video(_, metadata) => (metadata.width as f64, metadata.height as f64),
                    };
                    let spot = surface.find_empty_space(width, height, None, cx)?;
                    let x = spot["x"].as_f64().context("No placement position was returned.")?;
                    let y = spot["y"].as_f64().context("No placement position was returned.")?;
                    let meta = run.map(|run| run.provenance());
                    match media {
                        PreparedPlacement::Image(source, _, _) => {
                            let value = surface.apply(vec![DesignOp::CreateImage {
                                source, parent: None, name: Some("Generated image".into()), x, y,
                                width: Some(width), height: Some(height), meta,
                            }], "Place AI generation".into(), cx)?;
                            ensure!(value["applied"].as_bool() == Some(true), "The result could not be placed: {value}");
                            Ok(())
                        }
                        media => item.update(cx, |item, cx| {
                            ensure!(item.is_editable(), "Save or discard source edits before placing media on the canvas.");
                            item.with_document(cx, |document| match media {
                                PreparedPlacement::Vector(artwork) => generation_media::place_svg(document, artwork, x, y, meta),
                                PreparedPlacement::Video(bytes, metadata) => generation_media::place_video(document, bytes, metadata, x, y, meta),
                                PreparedPlacement::Image(_, _, _) => (Err(anyhow!("The media type changed before placement.")), crate::document::DocChange::None),
                            }).context("The design is still loading.")?
                        }),
                    }
                })
            }.await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result { Ok(()) => this.status = "Added to your design. Save the design to keep it.".into(), Err(error) => this.fail(error, cx) }
                cx.notify();
            }).log_err();
        }));
        cx.notify();
    }

    fn generate_design(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| {
            let prompt = self.prompt.read(cx).text(cx);
            ensure!(
                !prompt.trim().is_empty(),
                "Describe the design you want to create."
            );
            let item = self
                .canvas_item
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .context("Open a design canvas, then choose Generate a design from its AI menu.")?;
            agent_surface::set_active_item(item.downgrade(), cx);
            let workspace = self
                .workspace
                .upgrade()
                .context("This workspace was closed.")?;
            let prompt = format!(
                "Create this design in the active Fanta canvas using editable native layers: {}\n\nUse design_state and design_get_guidelines first, then design_batch with frames, text, shapes, and auto layout. Keep existing work and create in empty space. Finish by checking design_screenshot.",
                prompt.trim()
            );
            agent_ui::open_external_prompt_for_review(workspace, &prompt, window, cx)
        })();
        match result {
            Ok(()) => {
                self.error = None;
                self.status =
                    "Your editable design brief is ready in the Agent panel. Send it to begin."
                        .into();
            }
            Err(error) => self.fail(error, cx),
        }
        cx.notify();
    }

    fn model_dropdown(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let models: Vec<_> = self
            .models
            .iter()
            .filter(|model| self.accepts_model(model))
            .cloned()
            .collect();
        let selected = self.selected_model.clone();
        let this = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for model in models {
                let this = this.clone();
                let id = model.id.clone();
                menu.push_item(
                    ContextMenuEntry::new(model.label())
                        .toggleable(IconPosition::End, selected.as_deref() == Some(&model.id))
                        .handler(move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.selected_model = Some(id.clone());
                                this.choose_default_model();
                                this.error = None;
                                cx.notify();
                            })
                            .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(
            "generation-model",
            self.model()
                .map(GenerationModel::label)
                .unwrap_or_else(|| "No models available".into()),
            menu,
        )
        .style(DropdownStyle::Outlined)
        .full_width(true)
        .into_any_element()
    }

    fn size_dropdown(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sizes = self
            .model()
            .map(GenerationModel::sizes)
            .filter(|sizes| !sizes.is_empty())
            .unwrap_or_else(|| vec!["1024x1024".into(), "1280x768".into(), "768x1280".into()]);
        let selected = self.size.clone();
        let this = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for size in sizes {
                let this = this.clone();
                menu.push_item(
                    ContextMenuEntry::new(size.clone())
                        .toggleable(IconPosition::End, size == selected)
                        .handler(move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.size = size.clone();
                                cx.notify();
                            })
                            .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new("generation-size", self.size.clone(), menu)
            .style(DropdownStyle::Outlined)
            .full_width(true)
            .into_any_element()
    }

    fn render_source(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(source) = self.source.clone() else {
            return Label::new("Add an image to animate, vectorize, or edit a selected area.")
                .color(Color::Muted)
                .into_any_element();
        };
        let weak = cx.weak_entity();
        let preview = source.preview.clone();
        let points = self.points.clone();
        let bounds = self.source_bounds;
        let mask_mode = self.mode == GenerationMode::Masks;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(source.name).truncate())
                    .child(
                        Button::new("remove-source", "Remove")
                            .disabled(self.task.is_some())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.source = None;
                                this.mask = None;
                                this.points.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("source-preview")
                    .relative()
                    .w_full()
                    .h(px(200.))
                    .overflow_hidden()
                    .bg(cx.theme().colors().editor_background)
                    .rounded_md()
                    .child(
                        img(preview.image)
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                weak.update(cx, |this, _| this.source_bounds = Some(bounds))
                                    .log_err();
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .when(mask_mode, |element| {
                        element
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                    if this.task.is_some() {
                                        return;
                                    }
                                    if let Some(bounds) = this.source_bounds {
                                        let local = event.position - bounds.origin;
                                        if let Some((x, y)) = image_point(
                                            f32::from(local.x),
                                            f32::from(local.y),
                                            f32::from(bounds.size.width),
                                            f32::from(bounds.size.height),
                                            preview.width,
                                            preview.height,
                                        ) {
                                            if this.points.len() < 64 {
                                                this.points.push(MaskPoint {
                                                    x,
                                                    y,
                                                    positive: !this.exclude_points,
                                                });
                                                cx.notify();
                                            }
                                        }
                                    }
                                }),
                            )
                            .children(points.into_iter().filter_map(move |point| {
                                let bounds = bounds?;
                                let (x, y) = preview_point(
                                    point.x,
                                    point.y,
                                    f32::from(bounds.size.width),
                                    f32::from(bounds.size.height),
                                    preview.width,
                                    preview.height,
                                );
                                Some(
                                    div()
                                        .absolute()
                                        .left(px(x - 4.))
                                        .top(px(y - 4.))
                                        .size(px(8.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(gpui::white())
                                        .bg(if point.positive {
                                            gpui::rgb(0x22c55e)
                                        } else {
                                            gpui::rgb(0xef4444)
                                        }),
                                )
                            }))
                    }),
            )
            .when(mask_mode, |element| {
                element
                    .child(
                        Label::new("Click the image to mark an area. Then generate masks.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Button::new("include-points", "Include")
                                    .toggle_state(!self.exclude_points)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.exclude_points = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("exclude-points", "Exclude")
                                    .toggle_state(self.exclude_points)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.exclude_points = true;
                                        cx.notify();
                                    })),
                            )
                            .child(Button::new("clear-points", "Clear points").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.points.clear();
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Label::new(format!("{} points", self.points.len()))
                                    .size(LabelSize::Small),
                            ),
                    )
            })
            .into_any_element()
    }

    fn render_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let output = self.outputs.get(self.selected_output).cloned();
        let is_raster = output.as_ref().is_some_and(|output| {
            output.mime.starts_with("image/") && output.mime != "image/svg+xml" && !output.mask
        });
        let is_mask = output.as_ref().is_some_and(|output| output.mask);
        v_flex()
            .flex_1()
            .min_w(px(280.))
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(Label::new("Results").weight(gpui::FontWeight::SEMIBOLD))
                    .when(self.pending, |element| {
                        element.child(
                            Button::new("check-generation", "Check status")
                                .disabled(self.task.is_some())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(run) = this.active_run.clone() {
                                        this.check_status(run, cx);
                                    }
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(440.))
                    .min_h(px(240.))
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().editor_background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .when_some(self.preview.clone(), |element, preview| {
                        element.child(
                            img(preview.image)
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .when(self.preview.is_none(), |element| {
                        element.child(
                            Label::new(if self.preview_task.is_some() {
                                "Loading preview…"
                            } else if output
                                .as_ref()
                                .is_some_and(|output| output.mime.starts_with("video/"))
                            {
                                "Your video is ready. Play it or add it as a video layer in your design."
                            } else if self.pending {
                                "Your result will appear here."
                            } else {
                                "Create something, compare results, and bring it into your design."
                            })
                            .color(Color::Muted),
                        )
                    }),
            )
            .when(!self.outputs.is_empty(), |element| {
                element
                    .child(h_flex().flex_wrap().gap_1().children(
                        self.outputs.iter().enumerate().map(|(index, output)| {
                            Button::new(("generation-output", index), output.label.clone())
                                .toggle_state(index == self.selected_output)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected_output = index;
                                    this.load_preview(cx);
                                    cx.notify();
                                }))
                        }),
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("save-generation", "Save result")
                                    .disabled(self.task.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.save_output(cx))),
                            )
                            .when(output.as_ref().is_some_and(|output| output.mime.starts_with("video/")), |element| element.child(
                                Button::new("play-generation", "Play video").disabled(self.task.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.play_output(cx)))))
                            .when(is_mask, |element| element.child(
                                Button::new("remove-generated-background", "Remove background").disabled(self.task.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.remove_background(cx)))))
                            .when(is_raster || is_mask, |element| {
                                element.child(
                                    Button::new(
                                        "reuse-generation",
                                        if is_mask {
                                            "Edit this mask"
                                        } else {
                                            "Use as source"
                                        },
                                    )
                                    .disabled(
                                        self.task.is_some() || self.preview.is_none()
                                            || (is_mask && self.active_run.as_ref().is_none_or(|run| run.source.is_none())),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.use_result(cx))),
                                )
                            })
                            .when(output.as_ref().is_some_and(|output| !output.mask) && self.canvas_item.is_some(), |element| {
                                element.child(
                                    Button::new("place-generation", "Add to design")
                                        .style(ButtonStyle::Filled)
                                        .disabled(self.task.is_some())
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.place_output(cx)),
                                        ),
                                )
                            }),
                    )
            })
            .when(!self.history.is_empty(), |element| {
                element
                    .child(Label::new("Recent experiments").color(Color::Muted))
                    .children(self.history.iter().enumerate().map(|(index, run)| {
                        let run = run.clone();
                        let label = format!(
                            "{} · {}",
                            self.models.iter().find(|model| model.id == run.model).map(GenerationModel::label).unwrap_or_else(|| "Fanta".into()),
                            run.prompt.chars().take(72).collect::<String>()
                        );
                        Button::new(("generation-history", index), label)
                            .full_width()
                            .disabled(self.task.is_some())
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.check_status(run.clone(), cx)
                                }),
                            )
                    }))
            })
            .into_any_element()
    }
}

impl Render for GenerationWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_account(cx);
        let signed_in = self.client.account_access_token().is_some();
        let is_design = self.mode == GenerationMode::Design;
        let prompt_vectors =
            self.mode == GenerationMode::Vector && self.vector_operation == VectorOperation::Create;
        let trace_vectors =
            self.mode == GenerationMode::Vector && self.vector_operation == VectorOperation::Trace;
        let capabilities = self
            .model()
            .map(|model| model.capabilities.clone())
            .unwrap_or_default();
        let model_dropdown = self.model_dropdown(window, cx);
        let size_dropdown = self.size_dropdown(window, cx);
        let source = self.render_source(cx);
        let results = self.render_results(cx);
        let price = self
            .model()
            .and_then(|model| model.credits_per_output)
            .map(|credits| format!("From {credits:.2} credits per output"));
        v_flex().id("fanta-generation-workspace").size_full().min_h_0().p_5().gap_4().overflow_y_scroll()
            .bg(cx.theme().colors().panel_background)
            .child(h_flex().gap_2().flex_wrap().justify_between()
                .child(Label::new("Create with Fanta").size(LabelSize::Large).weight(gpui::FontWeight::SEMIBOLD))
                .child(h_flex().gap_2()
                    .child(Label::new(if signed_in { "Fanta account connected" } else { "Connect your account to generate" }).color(Color::Muted))
                    .when(!signed_in || self.error.is_some(), |element| element.child(Button::new("generation-sign-in", "Sign in")
                        .disabled(self.task.is_some()).on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))))
                    .child(Button::new("generation-billing", "Credits & billing").on_click(|_, _, cx| {
                        cx.open_url(&client::zed_urls::account_url(cx));
                    }))))
            .child(h_flex().gap_1().flex_wrap().children(GenerationMode::ALL.into_iter().map(|mode| {
                Button::new(("generation-mode", mode as usize), mode.label()).toggle_state(self.mode == mode)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
            })))
            .child(h_flex().items_start().gap_5().flex_wrap()
                .child(v_flex().w(px(330.)).max_w_full().flex_shrink_0().gap_3()
                    .when(self.mode == GenerationMode::Vector, |element| element
                        .child(Label::new("Vector tool").color(Color::Muted))
                        .child(h_flex().gap_1().flex_wrap().children([VectorOperation::Create, VectorOperation::Trace].into_iter().map(|operation| {
                            Button::new(("vector-operation", operation as usize), operation.label()).toggle_state(self.vector_operation == operation)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.vector_operation = operation;
                                    this.choose_default_model();
                                    this.error = None;
                                    cx.emit(ItemEvent::UpdateTab);
                                    cx.notify();
                                }))
                        })))
                        .child(Label::new(if prompt_vectors { "Describe artwork to create editable vectors with Fanta AI." } else { "Upload an image or capture the canvas to turn it into vectors." }).color(Color::Muted)))
                    .when(!is_design, |element| element
                        .child(Label::new("Model").color(Color::Muted)).child(model_dropdown)
                        .child(Button::new("refresh-generation-models", "Refresh models").disabled(self.catalog_task.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_catalog(cx)))))
                    .child(Label::new(if self.mode == GenerationMode::Masks { "What to select (optional)" } else if trace_vectors { "Guidance (optional)" } else { "Your idea" }).color(Color::Muted))
                    .child(self.prompt.clone())
                    .when(is_design, |element| element.child(Label::new("Create editable frames, text, and shapes with the Fanta Agent. Review and send your brief in the Agent panel.").color(Color::Muted)))
                    .when(!is_design, |element| element
                        .when(self.mode != GenerationMode::Masks && !trace_vectors, |element| element
                            .child(Label::new("Size").color(Color::Muted)).child(size_dropdown))
                        .when(self.mode != GenerationMode::Masks && self.mode != GenerationMode::Vector, |element| element
                            .child(Label::new("Seed").color(Color::Muted)).child(self.seed.clone()))
                        .when(self.mode == GenerationMode::Video, |element| element
                            .child(Label::new("Duration in seconds (1–5)").color(Color::Muted)).child(self.duration.clone())
                            .child(Label::new("For an image source, choose fanta-animate-1.").size(LabelSize::Small).color(Color::Muted)))
                        .when(capabilities["steps"].is_object(), |element| element.child(Label::new("Steps").color(Color::Muted)).child(self.steps.clone()))
                        .when(capabilities["guidance"].is_object(), |element| element.child(Label::new("Prompt strength").color(Color::Muted)).child(self.guidance.clone()))
                        .when(capabilities["negative_prompt"]["supported"].as_bool() == Some(true), |element| element.child(self.negative.clone()))
                        .when(!prompt_vectors, |element| element.child(h_flex().gap_2().flex_wrap()
                            .child(Button::new("upload-generation-source", "Upload image").disabled(self.task.is_some() || !signed_in)
                                .on_click(cx.listener(|this, _, _, cx| this.choose_source(cx))))
                            .when(self.canvas_item.is_some(), |element| element.child(Button::new("capture-generation-source", "Use canvas selection")
                                .disabled(self.task.is_some() || !signed_in).on_click(cx.listener(|this, _, _, cx| this.capture_canvas(cx))))))
                        .child(source))
                        .when(self.mask.is_some(), |element| element
                            .child(Label::new("A mask is selected. White areas will be changed.").color(Color::Accent))
                            .child(Label::new("Edit strength (0–1)").color(Color::Muted)).child(self.strength.clone())
                            .child(Button::new("clear-generation-mask", "Clear mask").on_click(cx.listener(|this, _, _, cx| { this.mask = None; cx.notify(); })))))
                    .when(prompt_vectors, |element| element.child(Label::new("Uses your Fanta AI credits. The final charge depends on AI usage.").size(LabelSize::Small).color(Color::Muted)))
                    .when_some(price.filter(|_| !is_design && !prompt_vectors), |element, price| element.child(Label::new(price).size(LabelSize::Small).color(Color::Muted)))
                    .child(Button::new("submit-generation", if is_design { "Prepare design brief" } else if self.mode == GenerationMode::Masks { "Generate masks" } else if prompt_vectors { "Create vectors" } else if trace_vectors { "Trace image" } else { "Generate" })
                        .style(ButtonStyle::Filled).full_width()
                        .disabled(self.task.is_some() || !signed_in || (!is_design && (self.model().is_none() || self.unresolved_submission.is_some())))
                        .on_click(cx.listener(|this, _, window, cx| this.generate(window, cx))))
                    .when(self.unresolved_submission.is_some() && self.task.is_none(), |element| element
                        .child(Label::new("The previous submission was not confirmed. Retry it with the same request to avoid a duplicate charge.").color(Color::Muted))
                        .child(Button::new("retry-generation-submission", "Retry same request")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(submission) = this.unresolved_submission.clone() { this.submit(submission, cx); }
                            }))))
                    .when(self.task.is_some() && self.pending, |element| element
                        .child(Button::new("pause-generation-poll", "Stop waiting")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.task = None;
                                this.status = "Status checks paused. Your generation continues and may use credits.".into();
                                cx.notify();
                            }))))
                    .child(Label::new(self.status.clone()).color(Color::Muted))
                    .when_some(self.error.clone(), |element, error| element.child(Label::new(error).color(Color::Error))))
                .when(!is_design, |element| element.child(results)))
    }
}

impl EventEmitter<ItemEvent> for GenerationWorkspace {}
impl Focusable for GenerationWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.prompt.read(cx).focus_handle(cx)
    }
}
impl Item for GenerationWorkspace {
    type Event = ItemEvent;
    fn tab_content_text(&self, _: usize, _: &App) -> SharedString {
        let label = if self.mode == GenerationMode::Vector {
            self.vector_operation.label()
        } else {
            self.mode.label()
        };
        format!("AI · {label}").into()
    }
    fn show_toolbar(&self) -> bool {
        false
    }
    fn to_item_events(event: &ItemEvent, callback: &mut dyn FnMut(ItemEvent)) {
        callback(*event);
    }
}

fn build_vector_message_request(
    model: &GenerationModel,
    prompt: &str,
    size: &str,
) -> Result<Value> {
    ensure!(
        VectorOperation::Create.accepts(model),
        "Choose a Fanta Claude model to create vectors from a prompt."
    );
    ensure!(
        !prompt.trim().is_empty(),
        "Describe the vector artwork you want to create."
    );
    ensure!(
        prompt.chars().count() <= 1200,
        "Keep the prompt under 1,200 characters."
    );
    let (width, height) = parse_size(size)?;
    let max_tokens = model.max_output_tokens.unwrap_or(8192).min(8192);
    ensure!(
        max_tokens >= 256,
        "This model's output limit is too small for vector artwork."
    );
    let system = format!(
        "Create production-quality vector artwork as a single standalone SVG document. Return only SVG, without markdown or explanations. Use xmlns=\"http://www.w3.org/2000/svg\", width=\"{width}\", height=\"{height}\" and a matching viewBox. Build editable paths and basic shapes in named groups. Use solid colors or non-repeating linear/radial gradients, standard strokes, and transforms. Convert lettering into paths. Do not use text elements, images, embedded raster data, external references, scripts, animation, use/symbol elements, masks, clip paths, patterns, filters, CSS stylesheets, or foreignObject. Keep the artwork concise and finish the SVG within the output limit."
    );
    Ok(json!({
        "model": model.id,
        "max_tokens": max_tokens,
        "stream": false,
        "system": system,
        "messages": [{"role":"user", "content": prompt.trim()}],
    }))
}

fn parse_vector_message(response: &Value) -> Result<(Option<String>, Arc<[u8]>)> {
    ensure!(
        response["stop_reason"] != "max_tokens",
        "The model ran out of space before finishing the vectors. Try a simpler design."
    );
    let blocks = response["content"]
        .as_array()
        .context("The model did not return vector artwork.")?;
    let mut text = String::new();
    for block in blocks {
        if block["type"] == "text" {
            let part = block["text"]
                .as_str()
                .context("The vector response contained unreadable text.")?;
            ensure!(
                text.len().saturating_add(part.len()) <= MAX_VECTOR_SVG_BYTES,
                "The vector artwork is too large. Try a simpler design."
            );
            text.push_str(part);
        }
    }
    let text = text.trim();
    let svg = if let Some(fenced) = text.strip_prefix("```") {
        let (language, body) = fenced
            .split_once('\n')
            .context("The model returned an incomplete vector document.")?;
        ensure!(
            matches!(language.trim(), "" | "svg" | "xml"),
            "The model did not return SVG artwork."
        );
        body.trim()
            .strip_suffix("```")
            .context("The model returned an incomplete vector document.")?
            .trim()
    } else {
        text
    };
    let document = usvg::roxmltree::Document::parse(svg)
        .context("The model returned incomplete vector artwork. Try a simpler design.")?;
    ensure!(
        document.root_element().tag_name().name() == "svg",
        "The model did not return SVG artwork."
    );
    ensure!(
        !document
            .descendants()
            .any(|node| matches!(node.tag_name().name(), "use" | "symbol")),
        "The model returned reused symbols. Ask for artwork made from individual paths."
    );
    generation_media::parse_svg(svg.as_bytes())
        .context("The artwork could not be converted to editable paths")?;
    let message_id = response["id"]
        .as_str()
        .filter(|id| id.len() <= 200)
        .map(str::to_owned);
    Ok((message_id, Arc::from(svg.as_bytes())))
}

fn vector_output(svg: Arc<[u8]>) -> MediaOutput {
    MediaOutput {
        label: "Vector artwork".into(),
        mime: "image/svg+xml".into(),
        location: MediaLocation::Inline(svg),
        mask: false,
    }
}

async fn fetch_catalog(
    client: &Arc<Client>,
    base_url: &str,
    expected_account: Option<&str>,
) -> Result<Vec<GenerationModel>> {
    let value = api_json(
        client,
        base_url,
        Method::GET,
        "/v1/models",
        None,
        None,
        expected_account,
    )
    .await?;
    serde_json::from_value(value["models"].clone()).context("The model catalog could not be read")
}

async fn api_json(
    client: &Arc<Client>,
    base_url: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    idempotency_key: Option<&str>,
    expected_account: Option<&str>,
) -> Result<Value> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("{base_url}{path}"))
        .header("Accept", "application/json");
    let token = expected_account.context("Sign in to Fanta to use your account credits.")?;
    ensure!(
        client.account_access_token().as_deref() == Some(token),
        "Your Fanta account changed. Start a new experiment."
    );
    request = request.header("Authorization", format!("Bearer {token}"));
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    let body = if let Some(body) = body {
        request = request.header("Content-Type", "application/json");
        AsyncBody::from(serde_json::to_vec(&body)?)
    } else {
        AsyncBody::empty()
    };
    let response = client
        .http_client()
        .send(request.body(body)?)
        .await
        .context("Could not reach Fanta. Check your connection and try again.")?;
    let status = response.status();
    let limit = if path == "/v1/messages" {
        MAX_VECTOR_RESPONSE_BYTES
    } else {
        MAX_JSON_BYTES
    };
    let bytes = bounded_body(response.into_body(), limit).await?;
    ensure!(
        client.account_access_token().as_deref() == expected_account,
        "Your Fanta account changed. Start a new experiment."
    );
    let value: Value = serde_json::from_slice(&bytes)
        .context("Fanta returned an unreadable response. Please try again.")?;
    if !status.is_success() {
        let message = match status.as_u16() {
            401 => "Your Fanta session expired. Sign in again to continue.".into(),
            402 => "You need more Fanta credits. Open Credits & billing to top up.".into(),
            429 => {
                "Fanta is busy with your other requests. Wait a moment and check the job status."
                    .into()
            }
            _ => error_message(&value),
        };
        return Err(ApiRejected {
            status: status.as_u16(),
            message,
        }
        .into());
    }
    Ok(value)
}

fn error_message(value: &Value) -> String {
    value["error"]["message"]
        .as_str()
        .or_else(|| value["message"].as_str())
        .or_else(|| value["error"].as_str())
        .or_else(|| value.as_str())
        .unwrap_or("The generation service could not complete this request.")
        .chars()
        .take(600)
        .collect()
}

async fn bounded_body(body: AsyncBody, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    body.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() <= limit,
        "This result is too large to open safely."
    );
    Ok(bytes)
}

async fn media_bytes(client: &Arc<Client>, location: &MediaLocation) -> Result<Arc<[u8]>> {
    match location {
        MediaLocation::Inline(bytes) => Ok(bytes.clone()),
        MediaLocation::Url(url) => {
            let url = url::Url::parse(url)?;
            ensure!(url.scheme() == "https", "The result URL must use HTTPS.");
            let response = client
                .http_client()
                .get(url.as_str(), AsyncBody::empty(), true)
                .await?;
            ensure!(
                response.status().is_success(),
                "This result link expired. Select the experiment again to refresh it."
            );
            Ok(bounded_body(response.into_body(), MAX_MEDIA_BYTES)
                .await?
                .into())
        }
    }
}

async fn upload_image(
    client: &Arc<Client>,
    base_url: &str,
    bytes: Vec<u8>,
    name: &str,
    mime: &str,
    preview: &Preview,
) -> Result<String> {
    let account = client.account_access_token();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let value = api_json(
        client,
        base_url,
        Method::POST,
        "/v1/assets/uploads",
        Some(json!({
            "mime":mime,"size_bytes":bytes.len(),"sha256":hash,"filename":name,
            "width":preview.width,"height":preview.height,
        })),
        None,
        account.as_deref(),
    )
    .await?;
    if value["reused"].as_bool() == Some(true) {
        return value["asset"]["id"]
            .as_str()
            .map(str::to_owned)
            .context("The uploaded asset was not returned.");
    }
    let id = value["asset_id"]
        .as_str()
        .context("Fanta did not create an upload.")?;
    let url = value["upload_url"]
        .as_str()
        .context("Fanta did not provide an upload link.")?;
    ensure!(
        url::Url::parse(url)?.scheme() == "https",
        "The upload link must use HTTPS."
    );
    // Upload links authorize the object themselves; sending the account token would leak it to storage.
    let request = Request::builder()
        .method(Method::PUT)
        .uri(url)
        .header("Content-Type", mime)
        .body(AsyncBody::from(bytes))?;
    let response = client.http_client().send(request).await?;
    ensure!(
        response.status().is_success(),
        "The image upload failed. Choose the image again to retry."
    );
    api_json(
        client,
        base_url,
        Method::POST,
        &format!("/v1/assets/uploads/{id}/complete"),
        Some(json!({})),
        None,
        account.as_deref(),
    )
    .await?;
    Ok(id.to_owned())
}

fn parse_size(size: &str) -> Result<(u32, u32)> {
    let (width, height) = size.split_once('x').context("Choose a valid image size.")?;
    let width = width.parse::<u32>()?;
    let height = height.parse::<u32>()?;
    ensure!(
        (1..=4096).contains(&width) && (1..=4096).contains(&height),
        "Size must be between 1 and 4096 pixels."
    );
    Ok((width, height))
}

#[allow(clippy::too_many_arguments)]
fn build_request(
    model: &GenerationModel,
    prompt: &str,
    negative: &str,
    size: &str,
    seed: &str,
    steps: &str,
    guidance: &str,
    duration: &str,
    strength: &str,
    source: Option<Value>,
    mask: Option<&[u8]>,
    points: &[MaskPoint],
) -> Result<Value> {
    ensure!(
        prompt.chars().count() <= 1200,
        "Keep the prompt under 1,200 characters."
    );
    ensure!(
        model.kind != "chat",
        "Use Create from prompt to generate vector artwork with a chat model."
    );
    let mut request = json!({"model":model.id});
    let mut input = json!({});
    if model.kind == "segment" {
        ensure!(
            source.is_some(),
            "Choose a source image before generating masks."
        );
        if !prompt.trim().is_empty() {
            input["text"] = json!(prompt.trim());
        }
        if !points.is_empty() {
            input["points"] = json!(
                points
                    .iter()
                    .map(|point| json!({"x":point.x,"y":point.y,"positive":point.positive}))
                    .collect::<Vec<_>>()
            );
        }
    } else {
        ensure!(
            !prompt.trim().is_empty() || matches!(model.kind.as_str(), "svg" | "vectorize"),
            "Describe what you want to create."
        );
        if !prompt.trim().is_empty() {
            request["prompt"] = json!(prompt.trim());
        }
        if !negative.trim().is_empty() {
            request["negative"] = json!(negative.trim());
        }
        let (width, height) = parse_size(size)?;
        request["width"] = json!(width);
        request["height"] = json!(height);
        if !seed.trim().is_empty() {
            request["seed"] = json!(
                seed.trim()
                    .parse::<u64>()
                    .context("Seed must be a positive whole number.")?
            );
        }
    }
    if matches!(model.kind.as_str(), "svg" | "vectorize") {
        ensure!(
            source.is_some(),
            "Choose a source image to turn into vectors."
        );
    }
    if model.kind == "video" {
        if matches!(model.id.as_str(), "fanta-animate-1" | "wan-2.2-i2v-a14b") {
            ensure!(source.is_some(), "Choose a source image to animate.");
        }
        if source.is_some() {
            ensure!(
                matches!(model.id.as_str(), "fanta-animate-1" | "wan-2.2-i2v-a14b"),
                "Choose fanta-animate-1 to animate a source image."
            );
        }
        let duration = if duration.trim().is_empty() {
            5.
        } else {
            duration
                .trim()
                .parse::<f64>()
                .context("Duration must be a number of seconds.")?
        };
        ensure!(
            duration.is_finite() && (1.0..=5.0).contains(&duration),
            "Choose a duration between 1 and 5 seconds."
        );
        let frames = (((duration * 16.).round() as u32) / 4 * 4 + 1).clamp(17, 81);
        input["frames"] = json!(frames);
        input["fps"] = json!(16);
    }
    for (key, text) in [("steps", steps), ("guidance", guidance)] {
        if !text.trim().is_empty() && model.capabilities[key].is_object() {
            let value = text
                .trim()
                .parse::<f64>()
                .with_context(|| format!("Enter a valid value for {key}."))?;
            let min = model.capabilities[key]["min"].as_f64().unwrap_or(0.);
            let max = model.capabilities[key]["max"].as_f64().unwrap_or(100.);
            ensure!(
                value.is_finite() && (min..=max).contains(&value),
                "{key} must be between {min} and {max}."
            );
            ensure!(
                key != "steps" || value.fract() == 0.,
                "Steps must be a whole number."
            );
            input[key] = json!(value);
        }
    }
    if let Some(source) = source {
        input["source"] = source;
    }
    if let Some(mask) = mask {
        ensure!(
            model.kind == "image" && model.supports_inpaint(),
            "Choose an image editing model to use the selected mask."
        );
        ensure!(
            input.get("source").is_some(),
            "Choose the original source image for this mask."
        );
        let strength = if strength.trim().is_empty() {
            0.8
        } else {
            strength
                .trim()
                .parse::<f64>()
                .context("Edit strength must be a number between 0 and 1.")?
        };
        ensure!(
            strength.is_finite() && (0.0..=1.0).contains(&strength),
            "Edit strength must be between 0 and 1."
        );
        input["mask"] = json!(STANDARD.encode(mask));
        input["mask_polarity"] = json!("edit_white");
        input["operation"] = json!("inpaint");
        input["strength"] = json!(strength);
    }
    if input.as_object().is_some_and(|input| !input.is_empty()) {
        request["input"] = input;
    }
    Ok(request)
}

fn normalize_outputs(outputs: &[Value]) -> Result<Vec<MediaOutput>> {
    let mut result = Vec::new();
    for output in outputs.iter().take(16) {
        if let Some(masks) = output["masks"].as_array() {
            for mask in masks.iter().take(64) {
                if let Some(data) = mask["data_url"].as_str() {
                    let (mime, bytes) = decode_data_url(data)?;
                    ensure!(
                        mime == "image/png",
                        "The model returned an unsupported mask format."
                    );
                    result.push(MediaOutput {
                        label: format!("Mask {}", result.len() + 1),
                        mime,
                        location: MediaLocation::Inline(bytes.into()),
                        mask: true,
                    });
                }
            }
        } else if let Some(svg) = output["svg"].as_str().filter(|svg| !svg.trim().is_empty()) {
            result.push(MediaOutput {
                label: format!("Vector {}", result.len() + 1),
                mime: "image/svg+xml".into(),
                location: MediaLocation::Inline(Arc::from(svg.as_bytes())),
                mask: false,
            });
        } else if let Some(data) = output["data_url"].as_str() {
            let (mime, bytes) = decode_data_url(data)?;
            result.push(MediaOutput {
                label: format!("Result {}", result.len() + 1),
                mime,
                location: MediaLocation::Inline(bytes.into()),
                mask: false,
            });
        } else if let Some(url) = output["url"].as_str() {
            let mime = output["mime"]
                .as_str()
                .or_else(|| output["r2_key_mime"].as_str())
                .unwrap_or(if output.get("duration_s").is_some() {
                    "video/mp4"
                } else {
                    "image/png"
                });
            result.push(MediaOutput {
                label: format!("Result {}", result.len() + 1),
                mime: mime.into(),
                location: MediaLocation::Url(url.to_owned()),
                mask: false,
            });
        }
    }
    Ok(result)
}

fn decode_data_url(data: &str) -> Result<(String, Vec<u8>)> {
    let (header, encoded) = data
        .split_once(',')
        .context("The result contains an invalid image.")?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|header| header.strip_suffix(";base64"))
        .context("The result uses an unsupported encoding.")?;
    ensure!(
        encoded.len() <= MAX_MEDIA_BYTES * 4 / 3 + 4,
        "This result is too large to open safely."
    );
    let bytes = STANDARD
        .decode(encoded)
        .context("The image could not be decoded.")?;
    Ok((mime.to_owned(), bytes))
}

fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    reader.limits(limits);
    let image = reader.decode().context("The image could not be opened.")?;
    ensure!(
        image.width() as u64 * image.height() as u64 <= MAX_IMAGE_PIXELS,
        "Choose an image smaller than 32 megapixels."
    );
    Ok(image)
}

fn cutout_image(image: &[u8], mask: &[u8]) -> Result<Vec<u8>> {
    let mut image = decode_image(image)?.to_rgba8();
    let mask = decode_image(mask)?.to_luma8();
    ensure!(
        image.dimensions() == mask.dimensions(),
        "The mask size does not match its source. Generate masks from the original image again."
    );
    for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
        pixel.0[3] = (pixel.0[3] as u16 * mask.0[0] as u16 / 255) as u8;
    }
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn make_preview(bytes: &[u8], mime: &str) -> Result<Preview> {
    if mime == "image/svg+xml" {
        let text = std::str::from_utf8(bytes).context("The SVG is not valid text.")?;
        let xml = usvg::roxmltree::Document::parse(text).context("The SVG could not be read.")?;
        ensure!(
            !xml.descendants().any(|node| {
                node.attributes().any(|attribute| {
                    attribute.name() == "href" && !attribute.value().starts_with('#')
                })
            }),
            "The SVG references external content and cannot be previewed safely. Save it for inspection."
        );
        ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "The vector is too large to preview."
        );
        return Ok(Preview {
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, bytes.to_vec())),
            width: 1024,
            height: 1024,
        });
    }
    let image = decode_image(bytes)?;
    let (width, height) = (image.width(), image.height());
    let image = image.thumbnail(PREVIEW_SIZE, PREVIEW_SIZE);
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(Preview {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, png.into_inner())),
        width,
        height,
    })
}

fn image_point(
    x: f32,
    y: f32,
    box_width: f32,
    box_height: f32,
    width: u32,
    height: u32,
) -> Option<(f64, f64)> {
    if width == 0 || height == 0 || box_width <= 0. || box_height <= 0. {
        return None;
    }
    let scale = (box_width / width as f32).min(box_height / height as f32);
    let x = (x - (box_width - width as f32 * scale) / 2.) / scale;
    let y = (y - (box_height - height as f32 * scale) / 2.) / scale;
    (x >= 0. && y >= 0. && x < width as f32 && y < height as f32).then_some((x as f64, y as f64))
}

fn preview_point(
    x: f64,
    y: f64,
    box_width: f32,
    box_height: f32,
    width: u32,
    height: u32,
) -> (f32, f32) {
    let scale = (box_width / width.max(1) as f32).min(box_height / height.max(1) as f32);
    (
        (box_width - width as f32 * scale) / 2. + x as f32 * scale,
        (box_height - height as f32 * scale) / 2. + y as f32 * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_client(
        cx: &mut gpui::TestAppContext,
        http: Arc<http_client::HttpClientWithUrl>,
    ) -> Arc<Client> {
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
            release_channel::init_test(
                semver::Version::new(0, 0, 0),
                release_channel::ReleaseChannel::Stable,
                cx,
            );
            cx.set_http_client(http);
            Client::production(cx)
        })
    }

    async fn sign_in_catalog_client(client: &Arc<Client>, cx: &mut gpui::TestAppContext) {
        client.override_authenticate(|_| {
            Task::ready(Ok(client::Credentials {
                user_id: 1,
                access_token: "fnt_live_catalog_test".into(),
            }))
        });
        client
            .sign_in(false, &cx.to_async())
            .await
            .expect("sign in");
    }

    #[gpui::test]
    async fn catalog_request_sends_the_signed_in_account_authorization(
        cx: &mut gpui::TestAppContext,
    ) {
        let http = http_client::FakeHttpClient::create(|request| async move {
            assert_eq!(request.method(), Method::GET);
            assert_eq!(request.uri().path(), "/v1/models");
            assert_eq!(
                request
                    .headers()
                    .get("Authorization")
                    .and_then(|header| header.to_str().ok()),
                Some("Bearer fnt_live_catalog_test")
            );
            Ok(http_client::Response::builder()
                .status(200)
                .body(r#"{"models":[{"id":"fanta-image-1","kind":"image"}]}"#.into())?)
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let models = fetch_catalog(
            &client,
            "https://api.fantaisa.net",
            client.account_access_token().as_deref(),
        )
        .await
        .expect("authenticated model catalog");
        assert_eq!(
            models.first().map(|model| model.id.as_str()),
            Some("fanta-image-1")
        );
    }

    #[gpui::test]
    async fn catalog_without_an_account_requires_sign_in_without_sending_a_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let http = http_client::FakeHttpClient::create(|_| async move {
            panic!("A signed-out catalog must not make an unauthorized network request")
        });
        let client = catalog_client(cx, http);
        let error = fetch_catalog(&client, "https://api.fantaisa.net", None)
            .await
            .err()
            .expect("sign-in required");
        assert!(error.to_string().contains("Sign in to Fanta"));
        assert!(!error.to_string().contains("expired"));
    }

    #[gpui::test]
    async fn catalog_rejects_an_in_flight_response_after_sign_out(cx: &mut gpui::TestAppContext) {
        let (response_sender, response_receiver) = futures::channel::oneshot::channel::<()>();
        let response_receiver = Arc::new(std::sync::Mutex::new(Some(response_receiver)));
        let http = http_client::FakeHttpClient::create(move |request| {
            assert_eq!(request.uri().path(), "/v1/models");
            let receiver = response_receiver
                .lock()
                .expect("response lock")
                .take()
                .expect("one catalog request");
            async move {
                receiver.await?;
                Ok(http_client::Response::builder()
                    .status(200)
                    .body(r#"{"models":[{"id":"old-account-model","kind":"image"}]}"#.into())?)
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let request = fetch_catalog(&client, "https://api.fantaisa.net", account.as_deref());
        futures::pin_mut!(request);
        assert!(futures::poll!(&mut request).is_pending());
        client.sign_out(&cx.to_async()).await;
        response_sender.send(()).expect("release response");
        let error = request.await.err().expect("discard old account response");
        assert!(error.to_string().contains("account changed"));
    }

    fn model(kind: &str) -> GenerationModel {
        GenerationModel {
            id: format!("fanta-{kind}-1"),
            kind: kind.into(),
            credits_per_output: None,
            max_output_tokens: None,
            capabilities: json!({"operations":["inpaint"],"steps":{"min":10,"max":50}}),
        }
    }

    #[test]
    fn vector_prompt_request_uses_metered_messages_and_bounded_output() {
        let mut model = model("chat");
        model.id = "claude-sonnet-5".into();
        model.max_output_tokens = Some(4096);
        let request = build_vector_message_request(&model, "a friendly fox icon", "1280x768")
            .expect("valid vector request");
        assert_eq!(request["model"], "claude-sonnet-5");
        assert_eq!(request["max_tokens"], 4096);
        assert_eq!(request["stream"], false);
        assert_eq!(request["messages"][0]["content"], "a friendly fox icon");
        assert!(
            request["system"]
                .as_str()
                .expect("system instruction")
                .contains("1280")
        );
        assert!(request.get("input").is_none());
        assert!(request.get("seed").is_none());
        assert!(build_vector_message_request(&model, "", "1280x768").is_err());
    }

    #[test]
    fn vector_image_workers_require_a_source_before_submission() {
        for kind in ["svg", "vectorize"] {
            let model = model(kind);
            assert!(
                build_request(
                    &model,
                    "a fox",
                    "",
                    "1024x1024",
                    "",
                    "",
                    "",
                    "",
                    "",
                    None,
                    None,
                    &[]
                )
                .is_err()
            );
            let request = build_request(
                &model,
                "",
                "",
                "1024x1024",
                "",
                "",
                "",
                "",
                "",
                Some(json!({"asset_id":"source"})),
                None,
                &[],
            )
            .expect("source tracing is valid without a prompt");
            assert_eq!(request["input"]["source"]["asset_id"], "source");
        }
    }

    #[test]
    fn vector_operations_offer_chat_for_creation_and_image_workers_for_tracing() {
        let mut chat = model("chat");
        chat.id = "claude-sonnet-5".into();
        assert!(VectorOperation::Create.accepts(&chat));
        assert!(!VectorOperation::Trace.accepts(&chat));
        assert!(!VectorOperation::Create.accepts(&model("svg")));
        assert!(VectorOperation::Trace.accepts(&model("svg")));
        assert!(VectorOperation::Trace.accepts(&model("vectorize")));
    }

    #[test]
    fn vector_messages_parse_fences_and_keep_message_provenance_out_of_generation_ids() {
        let response = json!({"id":"msg_vector", "stop_reason":"end_turn", "content":[
            {"type":"thinking","thinking":"not SVG"},
            {"type":"text","text":"```svg\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><path d=\"M0 0L20 0L20 20Z\"/></svg>\n```"}
        ]});
        let (message_id, svg) = parse_vector_message(&response).expect("valid editable SVG");
        let run = RunSummary {
            result: RunResult::VectorMessage {
                message_id,
                svg: svg.clone(),
            },
            model: "claude-sonnet-5".into(),
            prompt: "triangle".into(),
            source: None,
        };
        assert!(run.generation_id().is_none());
        assert_eq!(run.provenance()["message_id"], "msg_vector");
        assert!(run.provenance().get("generation_id").is_none());
        let output = vector_output(svg.clone());
        assert_eq!(output.mime, "image/svg+xml");
        assert!(!output.mask);
        let RunResult::VectorMessage { svg: retained, .. } = run.result else {
            panic!("local vector history")
        };
        assert_eq!(retained, svg);
    }

    #[test]
    fn vector_messages_reject_incomplete_unsafe_and_oversized_artwork() {
        let response =
            |text: &str| json!({"stop_reason":"end_turn", "content":[{"type":"text","text":text}]});
        assert!(parse_vector_message(&response("<svg>")).is_err());
        assert!(parse_vector_message(&response("<html/>")).is_err());
        assert!(parse_vector_message(&response("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><image href=\"file:///private/image.png\"/></svg>")).is_err());
        assert!(parse_vector_message(&response(&"x".repeat(MAX_VECTOR_SVG_BYTES + 1))).is_err());
        let mut truncated = response("<svg/>");
        truncated["stop_reason"] = json!("max_tokens");
        assert!(parse_vector_message(&truncated).is_err());
    }

    #[test]
    fn mask_request_preserves_source_and_pixel_coordinates() {
        let request = build_request(
            &model("segment"),
            "person",
            "",
            "1024x1024",
            "",
            "",
            "",
            "",
            "",
            Some(json!({"asset_id":"source"})),
            None,
            &[MaskPoint {
                x: 120.,
                y: 80.,
                positive: false,
            }],
        )
        .expect("valid segmentation");
        assert_eq!(
            request["input"],
            json!({"source":{"asset_id":"source"},"text":"person","points":[{"x":120.,"y":80.,"positive":false}]})
        );
        assert!(request.get("prompt").is_none());
    }

    #[test]
    fn mask_edit_uses_original_source_and_white_edit_polarity() {
        let request = build_request(
            &model("image"),
            "blue jacket",
            "",
            "1024x1024",
            "42",
            "20",
            "",
            "",
            "0.7",
            Some(json!({"asset_id":"original"})),
            Some(b"mask"),
            &[],
        )
        .expect("valid inpaint");
        assert_eq!(request["input"]["source"]["asset_id"], "original");
        assert_eq!(request["input"]["mask"], STANDARD.encode(b"mask"));
        assert_eq!(request["input"]["mask_polarity"], "edit_white");
        assert_eq!(request["input"]["operation"], "inpaint");
        assert_eq!(request["seed"], 42);
    }

    #[test]
    fn invalid_generation_controls_fail_before_charging() {
        assert!(
            build_request(
                &model("image"),
                "image",
                "",
                "1024x1024",
                "",
                "2",
                "",
                "",
                "",
                None,
                None,
                &[]
            )
            .is_err()
        );
        assert!(
            build_request(
                &model("vectorize"),
                "",
                "",
                "1024x1024",
                "",
                "",
                "",
                "",
                "",
                None,
                None,
                &[]
            )
            .is_err()
        );
        assert!(
            build_request(
                &model("video"),
                "animate",
                "",
                "1024x1024",
                "",
                "",
                "",
                "5",
                "",
                Some(json!({"asset_id":"source"})),
                None,
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn video_requests_use_worker_frames_and_require_an_animation_source() {
        let mut model = model("video");
        model.id = "fanta-animate-1".into();
        assert!(
            build_request(
                &model,
                "animate",
                "",
                "1280x768",
                "",
                "",
                "",
                "5",
                "",
                None,
                None,
                &[]
            )
            .is_err()
        );
        let request = build_request(
            &model,
            "animate",
            "",
            "1280x768",
            "",
            "",
            "",
            "5",
            "",
            Some(json!({"asset_id":"image"})),
            None,
            &[],
        )
        .expect("valid animation");
        assert_eq!(request["input"]["frames"], 81);
        assert_eq!(request["input"]["fps"], 16);
        assert!(request["input"].get("duration_s").is_none());
    }

    #[test]
    fn segmentation_nested_masks_are_normalized_without_losing_bytes() {
        let masks = normalize_outputs(&[json!({"masks":[{"data_url":"data:image/png;base64,bWFzaw==","score":0.9,"bbox":[0,0,4,4]}]})]).expect("valid output");
        assert_eq!(masks.len(), 1);
        assert!(masks[0].mask);
        match &masks[0].location {
            MediaLocation::Inline(bytes) => assert_eq!(bytes.as_ref(), b"mask"),
            _ => panic!("mask must remain inline"),
        }
    }

    #[test]
    fn background_removal_preserves_foreground_and_existing_alpha() {
        fn png(image: image::RgbaImage) -> Vec<u8> {
            let mut bytes = Cursor::new(Vec::new());
            image
                .write_to(&mut bytes, image::ImageFormat::Png)
                .expect("png");
            bytes.into_inner()
        }
        let source = image::RgbaImage::from_fn(2, 1, |_, _| image::Rgba([200, 100, 50, 128]));
        let mask = image::RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 255])
            }
        });
        let result = cutout_image(&png(source), &png(mask)).expect("transparent cutout");
        let image = image::load_from_memory(&result)
            .expect("png result")
            .to_rgba8();
        assert_eq!(image.get_pixel(0, 0).0, [200, 100, 50, 128]);
        assert_eq!(image.get_pixel(1, 0).0, [200, 100, 50, 0]);
    }

    #[test]
    fn point_selection_rejects_letterboxing_and_uses_source_resolution() {
        assert_eq!(image_point(100., 10., 200., 200., 1000, 500), None);
        assert_eq!(
            image_point(100., 100., 200., 200., 1000, 500),
            Some((500., 250.))
        );
        assert_eq!(
            preview_point(500., 250., 200., 200., 1000, 500),
            (100., 100.)
        );
        assert_eq!(image_point(200., 100., 200., 200., 1000, 500), None);
    }
}
