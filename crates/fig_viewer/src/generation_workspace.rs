use std::{cell::Cell, collections::HashMap, io::Cursor, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use client::{Client, ClientSettings};
use design_surface::{DesignOp, ScreenshotTarget};
use fanta_gpui::generation::{
    GenerationAction, GenerationChoice, GenerationDraft, GenerationKind,
    GenerationModel as ScreenModel, GenerationOptionGroup, GenerationOutput,
    GenerationOutputStatus, GenerationRecipe, GenerationScreen, GenerationSource,
    GenerationSubmission, GenerationTemplate, GenerationViewData, GenerationVoiceReference,
};
use futures::{AsyncReadExt as _, StreamExt as _};
#[cfg(feature = "mac_app_store")]
use gpui::Action as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, Image,
    ImageFormat, MouseButton, ObjectFit, PathPromptOptions, Pixels, Render, SharedString,
    Subscription, Task, WeakEntity, Window, actions, canvas, img,
};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
#[cfg(target_os = "macos")]
use rodio::Source as _;
use serde::Deserialize;
use serde_json::{Value, json};
use settings::Settings as _;
use sha2::{Digest as _, Sha256};
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, IconPosition, prelude::*};
use ui_input::InputField;
use util::ResultExt as _;
use workspace::{Item, MultiWorkspace, Workspace, item::ItemEvent};

use crate::{
    FigItem, FigView, agent_surface,
    generation_journal::{
        GenerationJournal, JournalRecord, JournalScope, JournalSnapshot, SavedDispatch,
        SavedRunResult, SavedSource, SavedSubmission, normalize_endpoint,
    },
    generation_media,
};

const MAX_SOURCE_BYTES: usize = 100 * 1024 * 1024;
const MAX_MEDIA_BYTES: usize = 128 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_VECTOR_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_VECTOR_SVG_BYTES: usize = 128 * 1024;
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const PREVIEW_SIZE: u32 = 1200;
const HISTORY_LIMIT: usize = 12;
// Generation submissions can run for 120 seconds before returning a job.
const API_TIMEOUT: Duration = Duration::from_secs(125);
const MEDIA_TRANSFER_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const VIDEO_PREVIEW_TIMEOUT: Duration = Duration::from_secs(20);
const QWEN_LOGO: &[u8] = include_bytes!("../assets/generation/logos/qwen.png");
const TONGYI_LOGO: &[u8] = include_bytes!("../assets/generation/logos/tongyi-mai.jpg");
const BFL_LOGO: &[u8] = include_bytes!("../assets/generation/logos/black-forest-labs-white.png");
const LTX_LOGO: &[u8] = include_bytes!("../assets/generation/logos/ltx.svg");
const WAN_LOGO: &[u8] = include_bytes!("../assets/generation/logos/wan.png");
const RESEMBLE_LOGO: &[u8] = include_bytes!("../assets/generation/logos/resemble-ai.png");
const HEXGRAD_LOGO: &[u8] = include_bytes!("../assets/generation/logos/hexgrad.png");
const ACE_LOGO: &[u8] = include_bytes!("../assets/generation/logos/ace-step.jpg");
const STARVECTOR_LOGO: &[u8] = include_bytes!("../assets/generation/logos/starvector.jpg");
const CLAUDE_LOGO: &[u8] = include_bytes!("../assets/generation/logos/claude.png");
const RECRAFT_LOGO: &[u8] = include_bytes!("../assets/generation/logos/recraft.png");
const MINIMAX_LOGO: &[u8] = include_bytes!("../assets/generation/logos/minimax.png");
const BYTEDANCE_LOGO: &[u8] = include_bytes!("../assets/generation/logos/bytedance-seed.png");
const ELEVENLABS_LOGO: &[u8] = include_bytes!("../assets/generation/logos/elevenlabs.png");
const GOOGLE_LOGO: &[u8] = include_bytes!("../assets/generation/logos/google.png");
const META_LOGO: &[u8] = include_bytes!("../assets/generation/logos/meta.png");
const XAI_LOGO: &[u8] = include_bytes!("../assets/generation/logos/xai.png");
const KLING_LOGO: &[u8] = include_bytes!("../assets/generation/logos/kling.png");
const OPENAI_LOGO: &[u8] = include_bytes!("../assets/generation/logos/openai.svg");
const QUIVER_LOGO: &[u8] = include_bytes!("../assets/generation/logos/quiverai.png");

actions!(
    fanta,
    [GenerateImage, GenerateVideo, GenerateAudio, GenerateVector]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenerationMode {
    Image,
    Video,
    Audio,
    Vector,
    Design,
    Masks,
}

impl GenerationMode {
    const ALL: [Self; 6] = [
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Vector,
        Self::Design,
        Self::Masks,
    ];
    const CREATION_MODES: [Self; 4] = [Self::Image, Self::Video, Self::Audio, Self::Vector];

    fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Vector => "Vector",
            Self::Design => "Design",
            Self::Masks => "Masks",
        }
    }

    fn from_label(label: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.label() == label)
            .context("The saved experiment uses an unsupported tool.")
    }

    fn accepts(self, model: &GenerationModel) -> bool {
        match self {
            Self::Image => model.kind == "image",
            Self::Video => model.kind == "video",
            Self::Audio => model.kind == "audio",
            Self::Vector => matches!(model.kind.as_str(), "chat" | "svg" | "vectorize"),
            Self::Masks => model.kind == "segment",
            Self::Design => false,
        }
    }

    fn generation_kind(self) -> Option<GenerationKind> {
        match self {
            Self::Image => Some(GenerationKind::Image),
            Self::Video => Some(GenerationKind::Video),
            Self::Audio => Some(GenerationKind::Audio),
            Self::Vector => Some(GenerationKind::Svg),
            Self::Design | Self::Masks => None,
        }
    }

    fn default_recipe(self) -> GenerationRecipe {
        match self {
            Self::Image | Self::Design | Self::Masks => GenerationRecipe::TextImage,
            Self::Video => GenerationRecipe::TextVideo,
            Self::Audio => GenerationRecipe::Speech,
            Self::Vector => GenerationRecipe::PromptSvg,
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
    display_name: Option<String>,
    #[serde(default)]
    credits_per_output: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    capabilities: Value,
    #[serde(default)]
    pricing: Value,
}

impl GenerationModel {
    fn label(&self) -> String {
        if let Some(name) = self
            .display_name
            .as_deref()
            .or_else(|| self.capabilities["display_name"].as_str())
            .filter(|name| !name.trim().is_empty())
        {
            return name.to_owned();
        }
        match self.id.as_str() {
            "claude-sonnet-5" => "Claude Sonnet 5".into(),
            "claude-haiku-4-5" => "Claude Haiku 4.5".into(),
            "claude-opus-4-8" => "Claude Opus 4.8".into(),
            "claude-fable-5" => "Claude Fable 5".into(),
            _ => self
                .id
                .split('-')
                .map(|part| match part {
                    "svg" => "SVG".to_owned(),
                    "gpt" => "GPT".to_owned(),
                    "i2v" => "I2V".to_owned(),
                    "t2v" => "T2V".to_owned(),
                    "ai" => "AI".to_owned(),
                    _ => {
                        let mut chars = part.chars();
                        chars
                            .next()
                            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                            .unwrap_or_default()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    fn gateway(&self) -> bool {
        self.capabilities["gateway"].is_object()
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

    fn recipes(&self) -> Vec<GenerationRecipe> {
        let declared = self.capabilities["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|operation| match operation {
                "text_to_image" => Some(GenerationRecipe::TextImage),
                "text_to_video" => Some(GenerationRecipe::TextVideo),
                "image_to_video" => Some(GenerationRecipe::ImageVideo),
                "speech" => Some(GenerationRecipe::Speech),
                "music" => Some(GenerationRecipe::Music),
                "prompt_to_svg" => Some(GenerationRecipe::PromptSvg),
                "image_to_svg" => Some(GenerationRecipe::ImageSvg),
                "vectorize" => Some(GenerationRecipe::Vectorize),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !declared.is_empty()
            || self.capabilities["operations"]
                .as_array()
                .is_some_and(|operations| !operations.is_empty())
        {
            return declared;
        }
        match self.kind.as_str() {
            "image" if !self.id.contains("edit") => vec![GenerationRecipe::TextImage],
            "video" if self.id.contains("i2v") || self.id.contains("animate") => {
                vec![GenerationRecipe::ImageVideo]
            }
            "video" => vec![GenerationRecipe::TextVideo],
            "audio" if self.id.contains("music") || self.id.contains("ace-step") => {
                vec![GenerationRecipe::Music]
            }
            "audio" => vec![GenerationRecipe::Speech],
            "chat" if self.id.starts_with("claude-") => vec![GenerationRecipe::PromptSvg],
            "svg" => vec![GenerationRecipe::ImageSvg],
            "vectorize" => vec![GenerationRecipe::Vectorize],
            _ => Vec::new(),
        }
    }

    fn hosted_creation(&self) -> bool {
        !self.id.starts_with("fanta-")
            && !self.recipes().is_empty()
            && (self.capabilities["hosted"] == true
                || self.gateway()
                || (self.kind == "image" && self.id == "flux-schnell")
                || (self.kind == "chat"
                    && matches!(
                        self.id.as_str(),
                        "claude-fable-5-1"
                            | "claude-opus-5-5"
                            | "claude-fable-5"
                            | "claude-sonnet-5"
                            | "claude-opus-4-8"
                            | "claude-haiku-4-5"
                    )))
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
    #[serde(default, deserialize_with = "deserialize_generation_seed")]
    seed: Option<String>,
    #[serde(default)]
    billed_credits: Option<f64>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    dispatch_protocol: Option<String>,
    #[serde(default)]
    dispatch_state: Option<String>,
    #[serde(default)]
    retry_requires_input: bool,
}

fn deserialize_generation_seed<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seed {
        Text(String),
        Number(u64),
    }

    Ok(
        Option::<Seed>::deserialize(deserializer)?.map(|seed| match seed {
            Seed::Text(seed) => seed,
            Seed::Number(seed) => seed.to_string(),
        }),
    )
}

impl GenerationResponse {
    fn pending(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "processing" | "warming")
    }

    fn saved_result(&self) -> SavedRunResult {
        let dispatch = if self.dispatch_protocol.as_deref() == Some("fanta-dispatch-v1") {
            match self.dispatch_state.as_deref() {
                Some("awaiting_input" | "delivering") => SavedDispatch::Unclaimed {
                    retry_requires_input: self.retry_requires_input,
                },
                Some("claimed" | "terminal") if !self.retry_requires_input => {
                    SavedDispatch::Claimed
                }
                _ => SavedDispatch::Unknown,
            }
        } else {
            SavedDispatch::Unknown
        };
        SavedRunResult::Generation {
            id: self.id.clone(),
            finished: !self.pending(),
            dispatch,
        }
    }

    fn honor_saved_claim(&mut self, result: Option<&SavedRunResult>) {
        if matches!(result, Some(SavedRunResult::Generation { id, dispatch: SavedDispatch::Claimed, .. }) if *id == self.id)
        {
            self.dispatch_protocol = Some("fanta-dispatch-v1".into());
            self.dispatch_state = Some("claimed".into());
            self.retry_requires_input = false;
        }
    }

    fn requires_input(&self) -> bool {
        self.saved_result().requires_input()
    }

    fn awaiting_input(&self) -> bool {
        self.requires_input() && self.dispatch_state.as_deref() == Some("awaiting_input")
    }

    fn should_poll(&self) -> bool {
        self.pending() && !self.awaiting_input()
    }
}

fn parse_generation_response(value: Value) -> Result<GenerationResponse> {
    let response: GenerationResponse = serde_json::from_value(value)?;
    ensure!(
        !response.id.is_empty()
            && response.id.len() <= 256
            && response
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "The server returned an invalid generation ID."
    );
    ensure!(
        matches!(
            response.status.as_str(),
            "queued" | "warming" | "processing" | "succeeded" | "failed" | "canceled"
        ),
        "The server returned an unknown generation status. Check the saved request again."
    );
    Ok(response)
}

#[derive(Clone)]
struct MediaOutput {
    label: String,
    mime: String,
    location: MediaLocation,
    mask: bool,
}

#[derive(Clone)]
struct GalleryRecord {
    id: String,
    kind: GenerationKind,
    model: String,
    status: GenerationOutputStatus,
    created_at: String,
    outputs: Vec<MediaOutput>,
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

#[derive(Clone)]
struct VoiceSource {
    name: String,
    bytes: Arc<[u8]>,
}

#[cfg(target_os = "macos")]
struct AudioPlaybackView {
    bytes: Arc<[u8]>,
    output: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    duration: Option<Duration>,
    playing: bool,
    error: Option<SharedString>,
    progress_task: Option<Task<()>>,
}

#[cfg(target_os = "macos")]
impl AudioPlaybackView {
    fn new(bytes: Arc<[u8]>) -> Self {
        Self {
            bytes,
            output: None,
            player: None,
            duration: None,
            playing: false,
            error: None,
            progress_task: None,
        }
    }

    fn play(&mut self, cx: &mut Context<Self>) {
        let result = (|| -> Result<()> {
            if self.player.as_ref().is_none_or(rodio::Player::empty) {
                self.player = None;
                self.output = None;
                let decoder = rodio::Decoder::new(Cursor::new(self.bytes.to_vec()))
                    .context("The generated audio could not be decoded.")?;
                self.duration = decoder.total_duration();
                let output = rodio::DeviceSinkBuilder::open_default_sink()
                    .context("No audio output device is available.")?;
                let player = rodio::Player::connect_new(output.mixer());
                player.append(decoder);
                self.output = Some(output);
                self.player = Some(player);
            } else if let Some(player) = self.player.as_ref() {
                player.play();
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.playing = true;
                self.error = None;
                if self.progress_task.is_none() {
                    self.progress_task = Some(cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(250))
                                .await;
                            let Ok(finished) = this.update(cx, |this, cx| {
                                let finished =
                                    this.player.as_ref().is_none_or(rodio::Player::empty);
                                if finished {
                                    this.playing = false;
                                    this.progress_task = None;
                                }
                                cx.notify();
                                finished
                            }) else {
                                break;
                            };
                            if finished {
                                break;
                            }
                        }
                    }));
                }
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
        self.playing = false;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.progress_task = None;
        if let Some(player) = self.player.take() {
            player.stop();
        }
        self.output = None;
        self.playing = false;
        cx.notify();
    }
}

#[cfg(target_os = "macos")]
impl Render for AudioPlaybackView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let position = self
            .player
            .as_ref()
            .map(rodio::Player::get_pos)
            .unwrap_or_default();
        let duration = self.duration.unwrap_or_default();
        let label = format!(
            "{:02}:{:02} / {:02}:{:02}",
            position.as_secs() / 60,
            position.as_secs() % 60,
            duration.as_secs() / 60,
            duration.as_secs() % 60,
        );
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Label::new("Audio preview").size(LabelSize::Large))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new(
                            "audio-playback-toggle",
                            if self.playing { "Pause" } else { "Play" },
                        )
                        .style(ButtonStyle::Filled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.playing {
                                this.pause(cx)
                            } else {
                                this.play(cx)
                            }
                        })),
                    )
                    .child(Label::new(label).color(Color::Muted)),
            )
            .when_some(self.error.clone(), |element, error| {
                element.child(Label::new(error).color(Color::Error))
            })
    }
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
    mode: GenerationMode,
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
    mode: GenerationMode,
}

impl Submission {
    fn saved(&self) -> Result<SavedSubmission> {
        let source = self
            .source
            .as_ref()
            .map(|source| {
                ensure!(
                    source.preview.image.format == ImageFormat::Png,
                    "The source preview cannot be saved for recovery. Choose the image again."
                );
                anyhow::Ok(SavedSource {
                    reference: source.reference.clone(),
                    name: source.name.clone(),
                    preview_png: STANDARD.encode(&source.preview.image.bytes),
                    width: source.preview.width,
                    height: source.preview.height,
                })
            })
            .transpose()?;
        Ok(SavedSubmission {
            key: self.key.clone(),
            request: self.request.clone(),
            model: self.model.clone(),
            prompt: self.prompt.clone(),
            source,
            mode: self.mode.label().into(),
        })
    }
}

fn restore_source(source: &SavedSource) -> Result<SourceImage> {
    ensure!(
        source.width > 0
            && source.height > 0
            && u64::from(source.width) * u64::from(source.height) <= MAX_IMAGE_PIXELS,
        "The saved source dimensions are invalid."
    );
    ensure!(
        source.preview_png.len() <= 12 * 1024 * 1024,
        "The saved source preview is too large."
    );
    let bytes = STANDARD
        .decode(&source.preview_png)
        .context("The saved source preview is unreadable.")?;
    let mut preview = make_preview(&bytes, "image/png")?;
    preview.width = source.width;
    preview.height = source.height;
    Ok(SourceImage {
        reference: source.reference.clone(),
        name: source.name.clone(),
        preview,
    })
}

fn run_from_record(record: &JournalRecord, mode: GenerationMode) -> Result<RunSummary> {
    let result = match record
        .result
        .as_ref()
        .context("The saved experiment has no result.")?
    {
        SavedRunResult::Generation { id, .. } => RunResult::Generation { id: id.clone() },
        SavedRunResult::VectorMessage { message_id, svg } => {
            ensure!(
                svg.len() <= MAX_VECTOR_SVG_BYTES,
                "The saved vector artwork is too large."
            );
            RunResult::VectorMessage {
                message_id: message_id.clone(),
                svg: svg.as_bytes().into(),
            }
        }
    };
    Ok(RunSummary {
        result,
        model: record.model.clone(),
        prompt: record.prompt.clone(),
        source: record.source.as_ref().map(restore_source).transpose()?,
        mode,
    })
}

fn restore_journal(
    snapshot: JournalSnapshot,
    account: Arc<str>,
) -> Result<(Vec<Submission>, Vec<RunSummary>)> {
    let mut submissions = Vec::new();
    let mut history = Vec::new();
    for record in snapshot.records {
        let mode = GenerationMode::from_label(&record.mode)?;
        if record.result.is_some() {
            history.push(run_from_record(&record, mode)?);
        }
        if record.result.is_none()
            || (record.request.is_some()
                && record
                    .result
                    .as_ref()
                    .is_some_and(SavedRunResult::requires_input))
        {
            submissions.push(Submission {
                source: record.source.as_ref().map(restore_source).transpose()?,
                request: record.request.context("The saved request is missing.")?,
                key: record.key,
                model: record.model,
                prompt: record.prompt,
                mode,
                account: Some(account.clone()),
            });
        }
    }
    Ok((submissions, history))
}

async fn persist_generation_status(
    journal: &GenerationJournal,
    response: &mut GenerationResponse,
    account: Option<Arc<str>>,
    cx: &gpui::AsyncApp,
) -> Result<Option<(Vec<Submission>, Vec<RunSummary>)>> {
    let (changed, observed) = journal.observe_generation(response.saved_result()).await?;
    response.honor_saved_claim(observed.as_ref());
    if !changed {
        return Ok(None);
    }
    let snapshot = journal.load().await?;
    let account = account.context("Sign in to restore your experiments.")?;
    cx.background_spawn(async move { restore_journal(snapshot, account) })
        .await
        .map(Some)
}

#[derive(Debug)]
struct ApiRejected {
    status: u16,
    unreserved: bool,
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
    Video(Arc<generation_media::PreparedVideo>),
}

struct GenerationWorkspace {
    workspace: WeakEntity<Workspace>,
    canvas_item: Option<WeakEntity<FigItem>>,
    client: Arc<Client>,
    base_url: String,
    mode: GenerationMode,
    recipe: GenerationRecipe,
    generation_screen: Option<Entity<GenerationScreen>>,
    _generation_screen_subscription: Option<Subscription>,
    screen_options: HashMap<(String, String), String>,
    screen_drafts: HashMap<GenerationKind, GenerationDraft>,
    selected_template: Option<SharedString>,
    member_id: Option<String>,
    gallery: Vec<GalleryRecord>,
    gallery_before: Option<String>,
    gallery_task: Option<Task<()>>,
    gallery_previews: HashMap<String, Arc<Image>>,
    gallery_preview_task: Option<Task<()>>,
    play_after_preview: bool,
    vector_operation: VectorOperation,
    models: Vec<GenerationModel>,
    selected_model: Option<String>,
    size: String,
    prompt: Entity<InputField>,
    prompt_drafts: Vec<(GenerationMode, Entity<InputField>)>,
    negative: Entity<InputField>,
    seed: Entity<InputField>,
    steps: Entity<InputField>,
    guidance: Entity<InputField>,
    duration: Entity<InputField>,
    strength: Entity<InputField>,
    source: Option<SourceImage>,
    end_frame: Option<SourceImage>,
    voice_reference: Option<VoiceSource>,
    voice_consent_granted: bool,
    mask: Option<Arc<[u8]>>,
    points: Vec<MaskPoint>,
    exclude_points: bool,
    source_bounds: Option<Bounds<Pixels>>,
    outputs: Vec<MediaOutput>,
    selected_output: usize,
    preview: Option<Preview>,
    prepared_video: Option<Arc<generation_media::PreparedVideo>>,
    #[cfg(target_os = "macos")]
    playback: Option<Entity<crate::video_playback::VideoPlaybackView>>,
    #[cfg(target_os = "macos")]
    audio_playback: Option<Entity<AudioPlaybackView>>,
    #[cfg(all(test, target_os = "macos"))]
    playback_factory: Option<
        Box<dyn Fn(Arc<[u8]>, &mut App) -> Entity<crate::video_playback::VideoPlaybackView>>,
    >,
    playback_active: bool,
    playback_removed: Cell<bool>,
    preview_request: Option<uuid::Uuid>,
    history: Vec<RunSummary>,
    active_run: Option<RunSummary>,
    pending: bool,
    playback_file: Option<tempfile::TempPath>,
    unresolved_submission: Option<Submission>,
    recovered_submissions: Vec<Submission>,
    journal: Option<GenerationJournal>,
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
    workspace.register_action(|workspace, _: &GenerateAudio, window, cx| {
        open(workspace, GenerationMode::Audio, None, window, cx);
    });
    workspace.register_action(|workspace, _: &GenerateVector, window, cx| {
        open(workspace, GenerationMode::Vector, None, window, cx);
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
    fn mount_generation_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.generation_screen.take() {
            let kind = old.read(cx).kind();
            self.screen_drafts.insert(kind, old.read(cx).draft(cx));
        }
        self._generation_screen_subscription = None;
        let Some(kind) = self.mode.generation_kind() else {
            return;
        };
        let data = self.generation_view_data(cx);
        let screen = cx.new(|cx| GenerationScreen::new("fanta-generation", kind, data, window, cx));
        if let Some(draft) = self.screen_drafts.get(&kind).cloned() {
            screen.update(cx, |screen, cx| screen.set_draft(draft, window, cx));
        }
        let subscription = cx.subscribe_in(
            &screen,
            window,
            |this, _, action: &GenerationAction, window, cx| {
                this.handle_generation_action(action.clone(), window, cx);
            },
        );
        self.generation_screen = Some(screen);
        self._generation_screen_subscription = Some(subscription);
    }

    fn screen_model(
        &self,
        model: &GenerationModel,
        recipe: GenerationRecipe,
        prompt_units: usize,
    ) -> ScreenModel {
        let option_groups = self.screen_option_groups(model, recipe);
        let mut description = match recipe {
            GenerationRecipe::TextImage => "Create an image from text",
            GenerationRecipe::TextVideo => "Generate a scene from text",
            GenerationRecipe::ImageVideo => "Animate a start frame",
            GenerationRecipe::Speech => "Generate spoken audio",
            GenerationRecipe::Music => "Compose music from a prompt",
            GenerationRecipe::PromptSvg => "Create editable SVG artwork",
            GenerationRecipe::ImageSvg => "Trace an image to SVG",
            GenerationRecipe::Vectorize => "Convert an image to vectors",
        }
        .to_owned();
        if model.gateway()
            && matches!(
                recipe,
                GenerationRecipe::TextVideo | GenerationRecipe::ImageVideo
            )
        {
            let selected = |key: &str| {
                option_groups
                    .iter()
                    .find(|group| group.key.as_ref() == key)
                    .map(|group| group.selected.to_string())
                    .or_else(|| option_value(&model.capabilities["controls"][key]["default"]))
            };
            let mut details = Vec::new();
            if let Some(duration) = selected("duration")
                .or_else(|| option_value(&model.capabilities["gateway"]["duration"]))
            {
                details.push(format!("{duration}s"));
            }
            if let Some(resolution) = selected("resolution")
                .or_else(|| selected("aspect_ratio"))
                .or_else(|| {
                    model.capabilities["gateway"]["resolution"]
                        .as_str()
                        .map(str::to_owned)
                })
            {
                details.push(resolution);
            }
            if let Some(audio) = selected("generate_audio") {
                details.push(
                    if audio == "true" {
                        "With audio"
                    } else {
                        "Silent"
                    }
                    .into(),
                );
            }
            if !details.is_empty() {
                description.push_str(" · ");
                description.push_str(&details.join(" · "));
            }
        }
        let mut screen = ScreenModel::new(&model.id, model.label(), recipe)
            .max_prompt_chars(Some(
                model.capabilities["prompt_limit"]
                    .as_u64()
                    .and_then(|limit| usize::try_from(limit).ok())
                    .unwrap_or(1_200),
            ))
            .description(description)
            .supports_prompt(recipe != GenerationRecipe::ImageSvg)
            .supports_negative(model.capabilities["negative_prompt"]["supported"] == true)
            .supports_seed(
                model.capabilities["seed"]["supported"] == true
                    || model.capabilities["seed"]["optional"] == true,
            )
            .supports_voice_reference(matches!(
                model.id.as_str(),
                "fanta-voice-1" | "fanta-voice-turbo-1" | "chatterbox-tts" | "chatterbox-tts-turbo"
            ))
            .supports_end_frame(
                recipe == GenerationRecipe::ImageVideo
                    && model.capabilities["inputs"]["end_frame"]["request_field"]
                        .as_str()
                        .is_some_and(|field| !field.is_empty()),
            )
            .option_groups(option_groups);
        let provider_hint = model.capabilities["provider"].as_str();
        let (inferred_provider, logo) = provider_mark(&model.id, provider_hint);
        let provider = provider_hint
            .filter(|provider| {
                !matches!(
                    *provider,
                    "Replicate" | "AI Gateway" | "SpaceXAI" | "Alibaba"
                )
            })
            .map(str::to_owned)
            .unwrap_or_else(|| inferred_provider.to_owned());
        screen = screen.provider(provider);
        if let Some((key, format, bytes)) = logo {
            screen = screen.logo(key, Arc::new(Image::from_bytes(format, bytes.to_vec())));
        }
        let unit = model.pricing["unit"].as_str().unwrap_or("output");
        if unit == "output" {
            if let Some(credits) = model
                .credits_per_output
                .or_else(|| model.pricing["estimated_credits"].as_f64())
            {
                let credits = self.selected_credit_rate(model, credits);
                let outputs = self
                    .screen_options
                    .get(&(model.id.clone(), "num_outputs".into()))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                if outputs > 1 {
                    screen = screen.credit_hint(format!(
                        "Est. {:.0} credits · {outputs} outputs",
                        (credits * outputs as f64).ceil()
                    ));
                } else if credits.fract() == 0. && credits >= 0. && credits <= u32::MAX as f64 {
                    screen = screen.credits_per_output(credits as u32);
                } else {
                    screen = screen.credit_hint(format!("{credits:.2} credits / output"));
                }
            }
        } else if let Some(rate) = model.pricing["credits_per_unit"].as_f64() {
            let rate = self.selected_credit_rate(model, rate);
            match unit {
                "second" => {
                    let selected = |key: &str| {
                        self.screen_options
                            .get(&(model.id.clone(), key.into()))
                            .and_then(|value| value.parse::<f64>().ok())
                            .or_else(|| model.capabilities["controls"][key]["default"].as_f64())
                    };
                    let seconds = selected("duration")
                        .or_else(|| selected("duration_s"))
                        .or_else(|| {
                            selected("music_length_ms").map(|milliseconds| milliseconds / 1000.)
                        })
                        .or_else(|| model.pricing["default_units"].as_f64());
                    if let Some(seconds) =
                        seconds.filter(|seconds| seconds.is_finite() && *seconds > 0.)
                    {
                        screen = screen.credit_hint(format!(
                            "Est. {:.0} credits · {seconds}s",
                            (rate * seconds).ceil()
                        ));
                    } else {
                        screen = screen.credit_hint(format!("{rate:.3} credits / second"));
                    }
                }
                "character" => {
                    if prompt_units > 0 {
                        screen = screen.credit_hint(format!(
                            "Est. {:.0} credits · {prompt_units} chars",
                            (rate * prompt_units as f64).ceil()
                        ));
                    } else {
                        screen = screen.credit_hint(format!("{rate:.3} credits / character"));
                    }
                }
                _ => {}
            }
        } else if let Some(estimate) = model.pricing["estimated_credits"].as_f64() {
            screen = screen.credit_hint(format!("Est. {estimate:.0} credits · default settings"));
        } else if let Some(credits) = model.credits_per_output {
            screen = screen.credit_hint(format!("Est. {credits:.0} credits · default settings"));
        } else if model.kind == "chat" {
            screen = screen.credit_hint("Usage-based");
        }
        screen
    }

    fn selected_credit_rate(&self, model: &GenerationModel, default_rate: f64) -> f64 {
        let Some(fields) = model.pricing["variant_fields"].as_array() else {
            return default_rate;
        };
        let Some(rates) = model.pricing["variant_credits_per_unit"].as_object() else {
            return default_rate;
        };
        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(field) = field.as_str() else {
                return default_rate;
            };
            let control = model.capabilities["controls"]
                .as_object()
                .and_then(|controls| {
                    controls.iter().find(|(key, control)| {
                        key.as_str() == field || control["request_field"].as_str() == Some(field)
                    })
                });
            let selected = control.and_then(|(key, control)| {
                self.screen_options
                    .get(&(model.id.clone(), key.clone()))
                    .cloned()
                    .or_else(|| option_value(&control["default"]))
            });
            let selected =
                selected.or_else(|| option_value(&model.capabilities["fixed_input"][field]));
            let Some(selected) = selected else {
                return default_rate;
            };
            values.push(selected);
        }
        rates
            .get(&values.join("|"))
            .and_then(Value::as_f64)
            .filter(|rate| rate.is_finite() && *rate >= 0.)
            .unwrap_or(default_rate)
    }

    fn screen_option_groups(
        &self,
        model: &GenerationModel,
        recipe: GenerationRecipe,
    ) -> Vec<GenerationOptionGroup> {
        let mut groups = Vec::new();
        if let Some(controls) = model.capabilities["controls"].as_object() {
            let mut controls = controls.iter().collect::<Vec<_>>();
            controls.sort_by_key(|(key, _)| (control_priority(key), *key));
            for (key, control) in controls {
                let constrained_value = (self.end_frame.is_some()
                    && recipe == GenerationRecipe::ImageVideo)
                    .then(|| model.capabilities["constraints"]["end_frame"][key].as_str())
                    .flatten();
                let mode_resolutions = (key.as_str() == "resolution")
                    .then(|| {
                        self.screen_options
                            .get(&(model.id.clone(), "mode".into()))
                            .cloned()
                            .or_else(|| {
                                option_value(&model.capabilities["controls"]["mode"]["default"])
                            })
                    })
                    .flatten()
                    .and_then(|mode| {
                        model.capabilities["constraints"]["mode_resolution"][mode].as_array()
                    });
                let choices = control["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| {
                        let value = option_value(value)?;
                        if constrained_value.is_some_and(|required| value != required) {
                            return None;
                        }
                        if mode_resolutions.is_some_and(|allowed| {
                            !allowed
                                .iter()
                                .any(|candidate| candidate.as_str() == Some(value.as_str()))
                        }) {
                            return None;
                        }
                        Some(GenerationChoice::new(
                            value.clone(),
                            option_choice_label(key, &value),
                        ))
                    })
                    .collect::<Vec<_>>();
                if choices.is_empty() {
                    continue;
                }
                let selected = constrained_value
                    .map(str::to_owned)
                    .or_else(|| {
                        self.screen_options
                            .get(&(model.id.clone(), key.clone()))
                            .cloned()
                            .or_else(|| option_value(&control["default"]))
                    })
                    .filter(|selected| {
                        choices
                            .iter()
                            .any(|choice| choice.value.as_ref() == selected.as_str())
                    })
                    .unwrap_or_else(|| choices[0].value.to_string());
                let group =
                    GenerationOptionGroup::new(key.as_str(), option_label(key), choices, selected);
                groups.push(group.advanced(control_priority(key) > 0));
            }
            if !groups.is_empty() {
                return groups;
            }
        }
        if model.capabilities["gateway"].is_object() {
            if recipe == GenerationRecipe::TextImage {
                let sizes = model.sizes();
                if !sizes.is_empty() {
                    let selected = self
                        .screen_options
                        .get(&(model.id.clone(), "canvas".into()))
                        .cloned()
                        .unwrap_or_else(|| {
                            model.capabilities["size"]["default"]
                                .as_str()
                                .unwrap_or(&sizes[0])
                                .to_owned()
                        });
                    groups.push(GenerationOptionGroup::new(
                        "canvas",
                        "Canvas size",
                        sizes
                            .iter()
                            .map(|size| GenerationChoice::new(size.as_str(), size.as_str())),
                        selected,
                    ));
                }
                let max = model.capabilities["num_outputs"]["max"]
                    .as_u64()
                    .unwrap_or(1)
                    .min(4);
                if max > 1 {
                    let selected = self
                        .screen_options
                        .get(&(model.id.clone(), "num_outputs".into()))
                        .cloned()
                        .unwrap_or_else(|| "1".into());
                    groups.push(GenerationOptionGroup::new(
                        "num_outputs",
                        "Outputs",
                        (1..=max).map(|n| GenerationChoice::new(n.to_string(), n.to_string())),
                        selected,
                    ));
                }
            }
            return groups;
        }
        if recipe == GenerationRecipe::TextImage {
            let sizes = model.sizes();
            if !sizes.is_empty() {
                let selected = self
                    .screen_options
                    .get(&(model.id.clone(), "canvas".into()))
                    .cloned()
                    .unwrap_or_else(|| {
                        model.capabilities["size"]["default"]
                            .as_str()
                            .unwrap_or(&sizes[0])
                            .to_owned()
                    });
                groups.push(GenerationOptionGroup::new(
                    "canvas",
                    "Canvas size",
                    sizes
                        .iter()
                        .map(|size| GenerationChoice::new(size.as_str(), size.as_str())),
                    selected,
                ));
            }
            for (key, label, values) in [
                ("steps", "Steps", &[10, 20, 30, 40, 50][..]),
                ("guidance", "Prompt guidance", &[1, 2, 4, 6, 8, 10][..]),
            ] {
                if model.capabilities[key].is_object() {
                    let selected = self
                        .screen_options
                        .get(&(model.id.clone(), key.into()))
                        .cloned()
                        .unwrap_or_else(|| {
                            option_value(&model.capabilities[key]["default"])
                                .unwrap_or_else(|| values[0].to_string())
                        });
                    groups.push(
                        GenerationOptionGroup::new(
                            key,
                            label,
                            values.iter().map(|value| {
                                let value = value.to_string();
                                GenerationChoice::new(value.clone(), value)
                            }),
                            selected,
                        )
                        .advanced(true),
                    );
                }
            }
        }
        if matches!(
            recipe,
            GenerationRecipe::TextVideo | GenerationRecipe::ImageVideo
        ) {
            let hd = !model.id.contains("ltx");
            let canvas = if hd {
                ["1280x720", "720x1280", "768x768"]
            } else {
                ["768x512", "512x768", "768x768"]
            };
            for (key, label, choices, default, advanced) in [
                (
                    "frames",
                    "Duration",
                    vec![
                        ("17", "1 second"),
                        ("33", "2 seconds"),
                        ("49", "3 seconds"),
                        ("65", "4 seconds"),
                        ("81", "5 seconds"),
                    ],
                    "49",
                    false,
                ),
                (
                    "canvas",
                    "Frame size",
                    canvas.iter().map(|size| (*size, *size)).collect(),
                    canvas[0],
                    false,
                ),
                (
                    "fps",
                    "Frame rate",
                    vec![("16", "16 fps"), ("24", "24 fps")],
                    "16",
                    true,
                ),
            ] {
                let selected = self
                    .screen_options
                    .get(&(model.id.clone(), key.into()))
                    .cloned()
                    .unwrap_or_else(|| default.to_owned());
                groups.push(
                    GenerationOptionGroup::new(
                        key,
                        label,
                        choices
                            .into_iter()
                            .map(|(value, label)| GenerationChoice::new(value, label)),
                        selected,
                    )
                    .advanced(advanced),
                );
            }
        }
        if recipe == GenerationRecipe::Vectorize {
            for (key, label, choices, default) in [
                (
                    "mode",
                    "Style",
                    vec![
                        ("line_art", "Line art"),
                        ("flat_color", "Flat color"),
                        ("trace", "Trace"),
                    ],
                    "line_art",
                ),
                (
                    "detail",
                    "Detail",
                    vec![("low", "Low"), ("medium", "Medium"), ("high", "High")],
                    "medium",
                ),
            ] {
                let selected = self
                    .screen_options
                    .get(&(model.id.clone(), key.into()))
                    .cloned()
                    .unwrap_or_else(|| default.into());
                groups.push(GenerationOptionGroup::new(
                    key,
                    label,
                    choices
                        .into_iter()
                        .map(|(value, label)| GenerationChoice::new(value, label)),
                    selected,
                ));
            }
        }
        if recipe == GenerationRecipe::Speech && model.id.contains("kokoro") {
            let selected = self
                .screen_options
                .get(&(model.id.clone(), "speed".into()))
                .cloned()
                .unwrap_or_else(|| "1.0".into());
            groups.push(GenerationOptionGroup::new(
                "speed",
                "Pace",
                [
                    GenerationChoice::new("0.8", "Relaxed"),
                    GenerationChoice::new("1.0", "Natural"),
                    GenerationChoice::new("1.2", "Brisk"),
                ],
                selected,
            ));
        }
        groups
    }

    fn generation_view_data(&self, cx: &App) -> GenerationViewData {
        let prompt_units = self
            .generation_screen
            .as_ref()
            .filter(|screen| Some(screen.read(cx).kind()) == self.mode.generation_kind())
            .map(|screen| screen.read(cx).draft(cx).prompt.encode_utf16().count())
            .or_else(|| {
                self.mode
                    .generation_kind()
                    .and_then(|kind| self.screen_drafts.get(&kind))
                    .map(|draft| draft.prompt.encode_utf16().count())
            })
            .unwrap_or(0);
        let models = self
            .models
            .iter()
            .filter(|model| model.hosted_creation())
            .flat_map(|model| {
                model
                    .recipes()
                    .into_iter()
                    .map(|recipe| self.screen_model(model, recipe, prompt_units))
            })
            .collect::<Vec<_>>();
        let selected_model_id = self.selected_model.as_ref().and_then(|id| {
            models
                .iter()
                .any(|model| model.id.as_ref() == id && model.recipe == self.recipe)
                .then(|| SharedString::from(id.clone()))
        });
        let source = self.source.as_ref().map(|source| {
            GenerationSource::new(source.reference.to_string(), source.name.clone())
                .preview(source.preview.image.clone())
        });
        let end_frame = self.end_frame.as_ref().map(|source| {
            GenerationSource::new(source.reference.to_string(), source.name.clone())
                .preview(source.preview.image.clone())
        });
        let voice_reference = self
            .voice_reference
            .as_ref()
            .map(|source| GenerationVoiceReference::new(source.name.clone(), source.name.clone()));
        let mut outputs = Vec::new();
        let active_run = self
            .active_run
            .as_ref()
            .filter(|run| run.mode.generation_kind().is_some());
        let active_id = active_run.and_then(RunSummary::generation_id);
        if let Some(run) = active_run {
            let kind = run.mode.generation_kind().expect("creation run kind");
            if self.outputs.is_empty() && (self.pending || self.task.is_some()) {
                outputs.push(
                    GenerationOutput::new(
                        format!("gen:{}:0", run.generation_id().unwrap_or("vector")),
                        kind,
                        "Generating…",
                        run.model.clone(),
                        GenerationOutputStatus::Running,
                    )
                    .prompt(run.prompt.clone()),
                );
            }
            for (index, output) in self.outputs.iter().enumerate() {
                let id = format!("gen:{}:{index}", run.generation_id().unwrap_or("vector"));
                let mut entry = GenerationOutput::new(
                    id.clone(),
                    kind,
                    output.label.clone(),
                    run.model.clone(),
                    GenerationOutputStatus::Succeeded,
                )
                .detail(output.mime.clone());
                if !run.prompt.is_empty() {
                    entry = entry.prompt(run.prompt.clone());
                }
                if let Some(preview) = self.gallery_previews.get(&id) {
                    entry = entry.preview(preview.clone());
                }
                if index == self.selected_output {
                    if let Some(preview) = self.preview.as_ref() {
                        entry = entry.preview(preview.image.clone());
                    }
                    #[cfg(target_os = "macos")]
                    if let Some(playback) = self.playback.as_ref() {
                        entry = entry.playback_view(playback.clone());
                    }
                    #[cfg(target_os = "macos")]
                    if let Some(playback) = self.audio_playback.as_ref() {
                        entry = entry.playback_view(playback.clone());
                    }
                }
                outputs.push(entry);
            }
        }
        for record in &self.gallery {
            if Some(record.id.as_str()) == active_id {
                continue;
            }
            let model_label = self
                .models
                .iter()
                .find(|model| model.id == record.model)
                .map(GenerationModel::label)
                .unwrap_or_else(|| record.model.clone());
            let count = record.outputs.len().max(1);
            for index in 0..count {
                let id = format!("gen:{}:{index}", record.id);
                let mut entry = GenerationOutput::new(
                    id.clone(),
                    record.kind,
                    format!("{} · {}", model_label, record.created_at),
                    record.model.clone(),
                    record.status,
                )
                .created_at(record.created_at.clone());
                if let Some(output) = record.outputs.get(index) {
                    entry = entry.detail(output.mime.clone());
                }
                if let Some(run) = self
                    .history
                    .iter()
                    .find(|run| run.generation_id() == Some(record.id.as_str()))
                {
                    entry = entry.prompt(run.prompt.clone());
                }
                if let Some(preview) = self.gallery_previews.get(&id) {
                    entry = entry.preview(preview.clone());
                }
                outputs.push(entry);
            }
        }
        for run in self
            .history
            .iter()
            .filter(|run| run.mode.generation_kind().is_some())
        {
            let Some(id) = run.generation_id() else {
                continue;
            };
            if Some(id) == active_id || self.gallery.iter().any(|record| record.id == id) {
                continue;
            }
            let kind = run.mode.generation_kind().expect("creation run kind");
            outputs.push(
                GenerationOutput::new(
                    format!("gen:{id}:0"),
                    kind,
                    run.prompt.chars().take(48).collect::<String>(),
                    run.model.clone(),
                    GenerationOutputStatus::Succeeded,
                )
                .prompt(run.prompt.clone())
                .detail("Saved experiment"),
            );
        }
        GenerationViewData {
            models,
            selected_recipe: Some(self.recipe),
            templates: generation_templates(),
            outputs,
            selected_model_id,
            selected_template_id: self.selected_template.clone(),
            selected_output_id: (active_run.is_some() && !self.outputs.is_empty()).then(|| {
                format!(
                    "gen:{}:{}",
                    active_id.unwrap_or("vector"),
                    self.selected_output
                )
                .into()
            }),
            source,
            end_frame,
            voice_reference,
            voice_consent_granted: self.voice_consent_granted,
            busy: self.task.is_some() || self.unresolved_submission.is_some(),
            error: self.error.clone(),
            has_more: self.gallery_before.is_some(),
        }
    }

    fn handle_generation_action(
        &mut self,
        action: GenerationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            GenerationAction::KindSelected(kind) => {
                let mode = match kind {
                    GenerationKind::Image => GenerationMode::Image,
                    GenerationKind::Video => GenerationMode::Video,
                    GenerationKind::Audio => GenerationMode::Audio,
                    GenerationKind::Svg => GenerationMode::Vector,
                };
                self.set_mode(mode, window, cx);
            }
            GenerationAction::RecipeSelected(recipe) => {
                self.recipe = recipe;
                self.vector_operation = if recipe == GenerationRecipe::PromptSvg {
                    VectorOperation::Create
                } else {
                    VectorOperation::Trace
                };
                self.selected_model = self
                    .models
                    .iter()
                    .find(|model| model.hosted_creation() && model.recipes().contains(&recipe))
                    .map(|model| model.id.clone());
                self.selected_template = None;
                cx.notify();
            }
            GenerationAction::ModelSelected { id } => {
                let id = id.to_string();
                if let Some(model) = self
                    .models
                    .iter()
                    .find(|model| model.id == id && model.hosted_creation())
                {
                    if let Some(recipe) = model
                        .recipes()
                        .into_iter()
                        .find(|recipe| *recipe == self.recipe)
                    {
                        self.recipe = recipe;
                        self.selected_model = Some(id);
                        self.selected_template = None;
                        cx.notify();
                    }
                }
            }
            GenerationAction::OptionSelected {
                model_id,
                key,
                value,
            } => {
                self.screen_options
                    .insert((model_id.to_string(), key.to_string()), value.to_string());
                cx.notify();
            }
            GenerationAction::DraftChanged => cx.notify(),
            GenerationAction::SourceRequested => self.choose_source(cx),
            GenerationAction::SourceCleared => {
                self.source = None;
                self.end_frame = None;
                cx.notify();
            }
            GenerationAction::EndFrameRequested => self.choose_end_frame(cx),
            GenerationAction::EndFrameCleared => {
                self.end_frame = None;
                cx.notify();
            }
            GenerationAction::VoiceReferenceRequested => self.choose_voice_reference(cx),
            GenerationAction::VoiceReferenceCleared => {
                self.voice_reference = None;
                self.voice_consent_granted = false;
                cx.notify();
            }
            GenerationAction::VoiceConsentChanged(granted) => {
                self.voice_consent_granted = granted;
                cx.notify();
            }
            GenerationAction::TemplateSelected { id } => {
                self.selected_template = Some(id.clone());
                let recipe = match id.as_ref() {
                    "audio-music" => Some(GenerationRecipe::Music),
                    "audio-narration" => Some(GenerationRecipe::Speech),
                    "svg-symbol" | "svg-monogram" => Some(GenerationRecipe::PromptSvg),
                    "video-orbit" | "video-atmosphere" => Some(GenerationRecipe::TextVideo),
                    "image-editorial" | "image-product" => Some(GenerationRecipe::TextImage),
                    _ => None,
                };
                if let Some(recipe) = recipe {
                    self.recipe = recipe;
                    self.selected_model = self
                        .models
                        .iter()
                        .find(|model| model.hosted_creation() && model.recipes().contains(&recipe))
                        .map(|model| model.id.clone());
                }
                if let Some(model_id) = generation_templates()
                    .into_iter()
                    .find(|template| template.id == id)
                    .and_then(|template| template.model_id)
                {
                    self.selected_model = Some(model_id.to_string());
                }
                cx.notify();
            }
            GenerationAction::GenerateRequested(submission) => {
                self.generate_screen_submission(submission, cx);
            }
            GenerationAction::OutputSelected { id } => {
                self.select_output_id(&id, cx);
                cx.notify();
            }
            GenerationAction::PreviewClosed { .. } => {
                self.play_after_preview = false;
                #[cfg(target_os = "macos")]
                if let Some(playback) = self.playback.as_ref() {
                    playback.update(cx, |playback, cx| playback.pause(cx));
                }
                #[cfg(target_os = "macos")]
                if let Some(playback) = self.audio_playback.as_ref() {
                    playback.update(cx, |playback, cx| playback.pause(cx));
                }
            }
            GenerationAction::DownloadRequested { id } => {
                if self.select_output_id(&id, cx) {
                    self.save_output(cx);
                }
            }
            GenerationAction::PlayRequested { id } => {
                if !self.select_output_id(&id, cx) {
                    return;
                }
                self.play_after_preview = self.preview_task.is_some();
                #[cfg(target_os = "macos")]
                if let Some(playback) = self.playback.as_ref() {
                    playback.update(cx, |playback, cx| playback.play(cx));
                }
                #[cfg(target_os = "macos")]
                if let Some(playback) = self.audio_playback.as_ref() {
                    playback.update(cx, |playback, cx| playback.play(cx));
                }
                #[cfg(not(target_os = "macos"))]
                self.play_output(cx);
            }
            GenerationAction::ReusePromptRequested { id } => {
                let prompt = self
                    .generation_view_data(cx)
                    .outputs
                    .into_iter()
                    .find(|output| output.id == id)
                    .and_then(|output| output.prompt);
                if let (Some(screen), Some(prompt)) = (self.generation_screen.as_ref(), prompt) {
                    let kind = screen.read(cx).kind();
                    let mut draft = screen.read(cx).draft(cx);
                    draft.prompt = prompt;
                    screen.update(cx, |screen, cx| screen.set_draft(draft.clone(), window, cx));
                    self.screen_drafts.insert(kind, draft);
                }
                self.selected_template = None;
                cx.notify();
            }
            GenerationAction::RefreshRequested => {
                self.refresh_catalog(cx);
                self.refresh_gallery(false, cx);
            }
            GenerationAction::LoadMoreRequested => self.refresh_gallery(true, cx),
        }
    }

    fn generate_screen_submission(
        &mut self,
        submission: GenerationSubmission,
        cx: &mut Context<Self>,
    ) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        if self.unresolved_submission.is_some() {
            self.fail(
                anyhow!("Recover the saved request before starting a new generation."),
                cx,
            );
            return;
        }
        let result: Result<(GenerationModel, Value)> = (|| {
            let model = self
                .models
                .iter()
                .find(|model| model.id == submission.model_id && model.hosted_creation())
                .cloned()
                .context("The selected model is no longer available. Refresh models.")?;
            ensure!(
                model.recipes().contains(&submission.recipe),
                "The selected model does not support this operation."
            );
            ensure!(
                self.recipe == submission.recipe,
                "The operation changed. Review the model and try again."
            );
            submission.validate_against(&self.screen_model(
                &model,
                submission.recipe,
                submission.prompt.encode_utf16().count(),
            ))?;
            if let Some(source_id) = submission.source_id.as_ref() {
                let selected_source: Value = serde_json::from_str(source_id.as_ref())?;
                ensure!(
                    self.source
                        .as_ref()
                        .is_some_and(|source| source.reference == selected_source),
                    "The source image changed. Review the start frame and try again."
                );
            }
            if let Some(end_frame_id) = submission.end_frame_id.as_ref() {
                let selected_end_frame: Value = serde_json::from_str(end_frame_id.as_ref())?;
                ensure!(
                    self.end_frame
                        .as_ref()
                        .is_some_and(|source| source.reference == selected_end_frame),
                    "The end frame changed. Review it and try again."
                );
            }
            if let Some(reference_id) = submission.voice_reference_id.as_ref() {
                ensure!(
                    self.voice_reference
                        .as_ref()
                        .is_some_and(|source| source.name == reference_id.as_ref()),
                    "The reference voice changed. Choose it again."
                );
            }
            let request = build_screen_request(
                &model,
                &submission,
                self.source.as_ref(),
                self.end_frame.as_ref(),
                self.voice_reference.as_ref(),
            )?;
            Ok((model, request))
        })();
        let (model, request) = match result {
            Ok(result) => result,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        if submission.recipe == GenerationRecipe::PromptSvg && model.kind == "chat" {
            let size = submission
                .options
                .iter()
                .find(|option| option.key == "canvas")
                .map(|option| option.value.to_string())
                .unwrap_or_else(|| "1024x1024".into());
            self.generate_vectors_with(model, submission.prompt.to_string(), size, cx);
            return;
        }
        self.submit(
            Submission {
                key: uuid::Uuid::new_v4().to_string(),
                request,
                model: model.id,
                prompt: submission.prompt.to_string(),
                source: self.source.clone(),
                account: self.client.account_access_token(),
                mode: self.mode,
            },
            cx,
        );
    }

    fn select_output_id(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some((generation_id, index)) = parse_generation_output_id(id) else {
            return false;
        };
        if self
            .active_run
            .as_ref()
            .is_some_and(|run| run.generation_id().unwrap_or("vector") == generation_id)
        {
            if index < self.outputs.len() {
                if index != self.selected_output {
                    self.selected_output = index;
                    self.load_preview(cx);
                }
                return true;
            }
            return false;
        }
        if let Some(record) = self
            .gallery
            .iter()
            .find(|record| record.id == generation_id)
            .cloned()
        {
            let prompt = self
                .history
                .iter()
                .find(|run| run.generation_id() == Some(generation_id))
                .map(|run| run.prompt.clone())
                .unwrap_or_default();
            let run = RunSummary {
                result: RunResult::Generation {
                    id: record.id.clone(),
                },
                model: record.model,
                prompt,
                source: None,
                mode: match record.kind {
                    GenerationKind::Image => GenerationMode::Image,
                    GenerationKind::Video => GenerationMode::Video,
                    GenerationKind::Audio => GenerationMode::Audio,
                    GenerationKind::Svg => GenerationMode::Vector,
                },
            };
            if record.outputs.is_empty() {
                self.refresh_gallery(false, cx);
                return false;
            }
            if index >= record.outputs.len() {
                return false;
            }
            self.active_run = Some(run);
            self.outputs = record.outputs;
            self.selected_output = index;
            self.pending = matches!(
                record.status,
                GenerationOutputStatus::Queued | GenerationOutputStatus::Running
            );
            self.load_preview(cx);
            cx.notify();
            return true;
        }
        if let Some(run) = self
            .history
            .iter()
            .find(|run| run.generation_id() == Some(generation_id))
            .cloned()
        {
            self.check_status(run, cx);
            return true;
        }
        false
    }
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
            recipe: mode.default_recipe(),
            generation_screen: None,
            _generation_screen_subscription: None,
            screen_options: HashMap::new(),
            screen_drafts: HashMap::new(),
            selected_template: None,
            member_id: None,
            gallery: Vec::new(),
            gallery_before: None,
            gallery_task: None,
            gallery_previews: HashMap::new(),
            gallery_preview_task: None,
            play_after_preview: false,
            vector_operation: VectorOperation::Create,
            models: Vec::new(),
            selected_model: None,
            size: "1024x1024".into(),
            prompt_drafts: vec![(mode, prompt.clone())],
            prompt,
            negative: cx.new(|cx| InputField::new(window, cx, "What to avoid (optional)")),
            seed: cx.new(|cx| InputField::new(window, cx, "Random")),
            steps: cx.new(|cx| InputField::new(window, cx, "Model default")),
            guidance: cx.new(|cx| InputField::new(window, cx, "Model default")),
            duration: cx.new(|cx| InputField::new(window, cx, "5")),
            strength: cx.new(|cx| InputField::new(window, cx, "0.8")),
            source: None,
            end_frame: None,
            voice_reference: None,
            voice_consent_granted: false,
            mask: None,
            points: Vec::new(),
            exclude_points: false,
            source_bounds: None,
            outputs: Vec::new(),
            selected_output: 0,
            preview: None,
            prepared_video: None,
            #[cfg(target_os = "macos")]
            playback: None,
            #[cfg(target_os = "macos")]
            audio_playback: None,
            #[cfg(all(test, target_os = "macos"))]
            playback_factory: None,
            playback_active: true,
            playback_removed: Cell::new(false),
            preview_request: None,
            history: Vec::new(),
            active_run: None,
            pending: false,
            playback_file: None,
            unresolved_submission: None,
            recovered_submissions: Vec::new(),
            journal: None,
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
        this.mount_generation_screen(window, cx);
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
        self.screen_drafts.clear();
        self.member_id = None;
        self.gallery.clear();
        self.gallery_before = None;
        self.gallery_task = None;
        self.gallery_previews.clear();
        self.gallery_preview_task = None;
        self.play_after_preview = false;
        self.task = None;
        self.preview_task = None;
        self.prepared_video = None;
        self.clear_playback(cx);
        self.preview_request = None;
        self.unresolved_submission = None;
        self.recovered_submissions.clear();
        self.journal = None;
        self.source = None;
        self.end_frame = None;
        self.voice_reference = None;
        self.voice_consent_granted = false;
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
        if self.catalog_task.is_some() || self.task.is_some() {
            return;
        }
        match normalize_endpoint(&self.base_url) {
            Ok(base_url) => self.base_url = base_url,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        }
        self.journal = None;
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let store = db::kvp::KeyValueStore::global(cx);
        let request_id = uuid::Uuid::new_v4();
        self.catalog_request = Some(request_id);
        self.catalog_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let (models, profile) = futures::try_join!(
                    fetch_catalog(&client, &base_url, Some(&account), cx.background_executor()),
                    api_json(
                        &client,
                        &base_url,
                        Method::GET,
                        "/v1/me",
                        None,
                        None,
                        Some(&account),
                        cx.background_executor()
                    ),
                )?;
                let scope = JournalScope::new(
                    &base_url,
                    profile["user"]["id"]
                        .as_str()
                        .context("Your account identity is unavailable.")?,
                    profile["org"]["id"]
                        .as_str()
                        .context("Your account organization is unavailable.")?,
                )?;
                let member_id = profile["user"]["id"]
                    .as_str()
                    .context("Your account identity is unavailable.")?
                    .to_owned();
                let journal = GenerationJournal::new(store, scope)?;
                let snapshot = journal.load().await?;
                let restored = cx
                    .background_spawn({
                        let account = account.clone();
                        async move { restore_journal(snapshot, account) }
                    })
                    .await?;
                anyhow::Ok((models, journal, restored, member_id))
            }
            .await;
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
                    Ok((models, journal, (submissions, history), member_id)) => {
                        this.models = models;
                        this.journal = Some(journal);
                        this.member_id = Some(member_id);
                        this.restore_history(submissions, history);
                        this.choose_default_model();
                        if this.mode.generation_kind().is_some()
                            && !this.models.iter().any(|model| {
                                Some(model.id.as_str()) == this.selected_model.as_deref()
                                    && model.hosted_creation()
                                    && model.recipes().contains(&this.recipe)
                            })
                        {
                            this.selected_model = this
                                .models
                                .iter()
                                .find(|model| {
                                    model.hosted_creation()
                                        && model.recipes().contains(&this.recipe)
                                })
                                .map(|model| model.id.clone());
                        }
                        this.error = None;
                        // Legacy recovery fixtures provide strict HTTP scripts. Gallery
                        // requests are exercised separately from those submission flows.
                        #[cfg(not(test))]
                        this.refresh_gallery(false, cx);
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn refresh_gallery(&mut self, load_more: bool, cx: &mut Context<Self>) {
        if self.gallery_task.is_some() {
            return;
        }
        let Some(member_id) = self.member_id.clone() else {
            return;
        };
        let before = if load_more {
            self.gallery_before.clone()
        } else {
            None
        };
        if load_more && before.is_none() {
            return;
        }
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("limit", "24")
            .append_pair("member", &member_id)
            .finish();
        let query = if let Some(before) = before.as_deref() {
            format!(
                "{query}&{}",
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("before", before)
                    .finish()
            )
        } else {
            query
        };
        let path = format!("/v1/generations?{query}");
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = client.account_access_token();
        self.gallery_task = Some(cx.spawn(async move |this, cx| {
            let result = api_json(
                &client,
                &base_url,
                Method::GET,
                &path,
                None,
                None,
                account.as_deref(),
                cx.background_executor(),
            )
            .await
            .and_then(parse_gallery_response);
            this.update(cx, |this, cx| {
                this.gallery_task = None;
                if this.client.account_access_token() != account {
                    return;
                }
                match result {
                    Ok((records, next_before)) => {
                        if load_more {
                            for record in records {
                                if !this.gallery.iter().any(|existing| existing.id == record.id) {
                                    this.gallery.push(record);
                                }
                            }
                        } else {
                            this.gallery = records;
                        }
                        if let Some(active_id) =
                            this.active_run.as_ref().and_then(RunSummary::generation_id)
                        {
                            if let Some(active_record) =
                                this.gallery.iter().find(|record| record.id == active_id)
                            {
                                if !active_record.outputs.is_empty() {
                                    this.outputs = active_record.outputs.clone();
                                    this.pending = matches!(
                                        active_record.status,
                                        GenerationOutputStatus::Queued
                                            | GenerationOutputStatus::Running
                                    );
                                }
                            }
                        }
                        this.gallery_before = next_before;
                        this.load_gallery_previews(cx);
                    }
                    Err(error) => {
                        log::warn!("Fanta generation gallery: {error:#}");
                        this.error = Some(format!("Gallery unavailable: {error}").into());
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn load_gallery_previews(&mut self, cx: &mut Context<Self>) {
        self.gallery_preview_task = None;
        let previews = self
            .gallery
            .iter()
            .flat_map(|record| {
                record
                    .outputs
                    .iter()
                    .enumerate()
                    .map(move |(index, output)| {
                        (format!("gen:{}:{index}", record.id), output.clone())
                    })
            })
            .filter(|(id, output)| {
                output.mime.starts_with("image/") && !self.gallery_previews.contains_key(id)
            })
            .take(24)
            .collect::<Vec<_>>();
        if previews.is_empty() {
            return;
        }
        let client = self.client.clone();
        let account = client.account_access_token();
        self.gallery_preview_task = Some(cx.spawn(async move |this, cx| {
            for (id, output) in previews {
                let result = async {
                    let bytes =
                        media_bytes(&client, &output.location, cx.background_executor()).await?;
                    cx.background_spawn(async move { make_preview(&bytes, &output.mime) })
                        .await
                }
                .await;
                if this
                    .update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            return false;
                        }
                        if let Ok(preview) = result {
                            this.gallery_previews.insert(id, preview.image);
                            cx.notify();
                        }
                        true
                    })
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |this, _| this.gallery_preview_task = None)
                .log_err();
        }));
    }

    fn restore_history(&mut self, mut submissions: Vec<Submission>, history: Vec<RunSummary>) {
        self.unresolved_submission = if submissions.is_empty() {
            None
        } else {
            Some(submissions.remove(0))
        };
        self.recovered_submissions = submissions;
        self.history = history;
        if self.unresolved_submission.is_some() {
            self.status =
                "An earlier request needs recovery. Retry its saved submission to continue.".into();
        }
    }

    fn accepts_model(&self, model: &GenerationModel) -> bool {
        if self.mode.generation_kind().is_some() {
            model.hosted_creation()
                && self.mode.accepts(model)
                && model.recipes().contains(&self.recipe)
        } else {
            self.mode.accepts(model)
                && (self.mode != GenerationMode::Vector || self.vector_operation.accepts(model))
        }
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
                        model.id.starts_with("fanta-"),
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

    fn set_mode(&mut self, mode: GenerationMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.prompt = if let Some((_, prompt)) = self
            .prompt_drafts
            .iter()
            .find(|(draft_mode, _)| *draft_mode == mode)
        {
            prompt.clone()
        } else {
            let prompt =
                cx.new(|cx| InputField::new(window, cx, "Describe what you want to create"));
            prompt
                .read(cx)
                .editor()
                .clone()
                .set_multiline(Some(6), window, cx);
            self.prompt_drafts.push((mode, prompt.clone()));
            prompt
        };
        self.mode = mode;
        self.recipe = mode.default_recipe();
        if mode != GenerationMode::Image {
            self.mask = None;
        }
        self.end_frame = None;
        self.choose_default_model();
        self.selected_model = self
            .models
            .iter()
            .find(|model| model.hosted_creation() && model.recipes().contains(&self.recipe))
            .map(|model| model.id.clone());
        self.error = None;
        if self.task.is_none() {
            self.status = match mode {
                GenerationMode::Image => "Describe your image, then choose Generate.",
                GenerationMode::Video => "Describe a video, or choose an image to animate.",
                GenerationMode::Audio => "Write speech or describe music to generate audio.",
                GenerationMode::Vector => {
                    "Create editable vectors from a prompt or trace an image."
                }
                GenerationMode::Design => {
                    "Describe your design to prepare a brief for Fanta Agent."
                }
                GenerationMode::Masks => {
                    "Click your source image to mark an area, or describe what to select."
                }
            }
            .into();
        }
        self.prompt.read(cx).focus_handle(cx).focus(window, cx);
        self.mount_generation_screen(window, cx);
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
            self.fail(anyhow!("Use Retry submission to recover the saved request before starting another generation."), cx);
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
                mode: self.mode,
            },
            cx,
        );
    }

    fn generate_vectors(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model().cloned() else {
            self.fail(
                anyhow!("No vector creation model is available. Refresh the model list."),
                cx,
            );
            return;
        };
        let prompt = self.prompt.read(cx).text(cx);
        self.generate_vectors_with(model, prompt, self.size.clone(), cx);
    }

    fn generate_vectors_with(
        &mut self,
        model: GenerationModel,
        prompt: String,
        size: String,
        cx: &mut Context<Self>,
    ) {
        if self.task.is_some() {
            return;
        }
        let Some(journal) = self.journal.clone() else {
            self.fail(
                anyhow!("Account recovery is not ready. Refresh models before generating."),
                cx,
            );
            return;
        };
        let request = match build_vector_message_request(&model, &prompt, &size) {
            Ok(request) => request,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = client.account_access_token();
        let saved = SavedSubmission {
            key: uuid::Uuid::new_v4().to_string(),
            request: request.clone(),
            model: model.id.clone(),
            prompt: prompt.clone(),
            source: None,
            mode: GenerationMode::Vector.label().into(),
        };
        self.pending = false;
        self.outputs.clear();
        self.preview = None;
        self.preview_task = None;
        self.prepared_video = None;
        self.clear_playback(cx);
        self.preview_request = None;
        self.active_run = None;
        self.error = None;
        self.status = "Creating vector artwork…".into();
        self.task = Some(cx.spawn(async move |this, cx| {
            let response = api_json(&client, &base_url, Method::POST, "/v1/messages", Some(request), None, account.as_deref(), cx.background_executor()).await;
            let result = response.and_then(|response| parse_vector_message(&response));
            let recovery_error = match &result {
                Ok((message_id, svg)) => match std::str::from_utf8(svg) {
                    Ok(svg) => journal.record_completed(saved, SavedRunResult::VectorMessage {
                        message_id: message_id.clone(), svg: svg.to_owned(),
                    }).await.err(),
                    Err(error) => Some(error.into()),
                },
                Err(_) => None,
            };
            this.update(cx, |this, cx| {
                this.task = None;
                if this.client.account_access_token() != account { this.sync_account(cx); return; }
                match result {
                    Ok((message_id, svg)) => {
                        let run = RunSummary {
                            result: RunResult::VectorMessage { message_id, svg: svg.clone() },
                            model: model.id, prompt, source: None, mode: GenerationMode::Vector,
                        };
                        this.active_run = Some(run.clone());
                        this.history.insert(0, run);
                        let mut vectors = 0;
                        this.history.retain(|run| match run.result {
                            RunResult::VectorMessage { .. } => {
                                vectors += 1;
                                vectors <= HISTORY_LIMIT
                            }
                            RunResult::Generation { .. } => true,
                        });
                        this.outputs = vec![vector_output(svg)];
                        this.selected_output = 0;
                        this.status = "Vector artwork is ready. AI usage is billed to your Fanta credits.".into();
                        this.error = recovery_error.map(|error| format!("Your artwork is ready, but recovery could not be saved: {error}. Save the artwork before closing this tab.").into());
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
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(journal) = self.journal.clone() else {
            self.fail(
                anyhow!("Account recovery is not ready. Refresh models before generating."),
                cx,
            );
            return;
        };
        if submission.account.is_none() || submission.account != self.client.account_access_token()
        {
            self.fail(
                anyhow!("Your account changed. Open the saved request for the current account."),
                cx,
            );
            return;
        }
        let saved = match submission.saved() {
            Ok(saved) => saved,
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        };
        let retrying = self
            .unresolved_submission
            .as_ref()
            .is_some_and(|previous| previous.key == submission.key);
        self.unresolved_submission = Some(submission.clone());
        let Submission {
            key,
            model,
            prompt,
            source,
            account,
            mode,
            ..
        } = submission;
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        self.pending = false;
        self.error = None;
        self.status = "Saving your request for recovery…".into();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let previous = journal.prepare(saved).await.map_err(|error| anyhow!("{error:#}"))?;
                let mut response = match &previous.result {
                    Some(SavedRunResult::Generation { id, .. }) => {
                        let value = api_json(&client, &base_url, Method::GET,
                            &format!("/v1/generations/{id}"), None, None,
                            account.as_deref(), cx.background_executor()).await?;
                        let mut response = parse_generation_response(value)?;
                        ensure!(response.id == *id,
                            "The status response did not match the saved generation.");
                        // A stale retry must observe a permanent claim before deciding
                        // whether the original input can be delivered again.
                        let observed = journal.accept(&key, response.saved_result()).await?;
                        response.honor_saved_claim(observed.result.as_ref());
                        if observed.result.as_ref().is_some_and(SavedRunResult::requires_input)
                            && observed.request.is_some()
                        {
                            let value = api_json(&client, &base_url, Method::POST,
                                "/v1/generations", observed.request, Some(&key),
                                account.as_deref(), cx.background_executor()).await?;
                            let retried = parse_generation_response(value)?;
                            ensure!(retried.id == *id,
                                "The retry response did not match the saved generation. Keep this request for recovery.");
                            retried
                        } else {
                            response
                        }
                    }
                    Some(SavedRunResult::VectorMessage { .. }) => {
                        bail!("This experiment already has artwork. Open it from Recent experiments.")
                    }
                    None => {
                        let request = previous.request.context("The saved request is missing.")?;
                        let value = api_json(&client, &base_url, Method::POST,
                            "/v1/generations", Some(request), Some(&key),
                            account.as_deref(), cx.background_executor()).await?;
                        parse_generation_response(value)?
                    }
                };
                let accepted = journal.accept(&key, response.saved_result()).await
                    .with_context(|| format!("Generation {} was accepted, but its recovery record could not be saved. Retry the same request", response.id))?;
                response.honor_saved_claim(accepted.result.as_ref());
                let snapshot = journal.load().await?;
                let restored = cx.background_spawn({
                    let account = account.clone().context("Sign in to restore your experiments.")?;
                    async move { restore_journal(snapshot, account) }
                }).await?;
                anyhow::Ok((response, restored))
            }.await;
            match result {
                Ok((response, (submissions, history))) => {
                    let run = RunSummary {
                        result: RunResult::Generation { id: response.id.clone() },
                        model, prompt, source, mode,
                    };
                    let applied = this.update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            this.sync_account(cx);
                            return false;
                        }
                        this.restore_history(submissions, history);
                        this.active_run = Some(run);
                        this.outputs.clear();
                        this.preview = None;
                        this.preview_task = None;
                        this.prepared_video = None;
                        this.clear_playback(cx);
                        this.preview_request = None;
                        this.accept_response(&response, cx);
                        true
                    }).log_err().unwrap_or(false);
                    if applied && response.should_poll() {
                        Self::poll(this.clone(), client, base_url, response.id, account.clone(), journal.clone(), cx).await;
                    }
                }
                Err(mut error) => {
                    let mut discarded = false;
                    if error.downcast_ref::<ApiRejected>().is_some_and(|error| {
                        error.unreserved && error.status < 500 && error.status != 409 && !retrying
                    }) {
                        match journal.reject(&key).await {
                            Ok(()) => discarded = true,
                            Err(persistence_error) => {
                                error = anyhow!("{error}. The saved request could not be updated: {persistence_error}");
                            }
                        }
                    }
                    this.update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            this.sync_account(cx);
                            return;
                        }
                        if discarded {
                            this.unresolved_submission = if this.recovered_submissions.is_empty() {
                                None
                            } else { Some(this.recovered_submissions.remove(0)) };
                        }
                        this.fail(error, cx);
                    }).log_err();
                }
            }
            this.update(cx, |this, cx| {
                this.task = None;
                cx.notify();
            }).log_err();
        }));
        cx.notify();
    }

    async fn poll(
        this: WeakEntity<Self>,
        client: Arc<Client>,
        base_url: String,
        id: String,
        account: Option<Arc<str>>,
        journal: GenerationJournal,
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
                cx.background_executor(),
            )
            .await
            .and_then(parse_generation_response)
            .and_then(|response| {
                ensure!(
                    response.id == id,
                    "The status response did not match the saved generation."
                );
                Ok(response)
            });
            match result {
                Ok(mut response) => {
                    let persisted =
                        persist_generation_status(&journal, &mut response, account.clone(), cx)
                            .await;
                    let applied = this
                        .update(cx, |this, cx| {
                            if this.client.account_access_token() != account {
                                this.sync_account(cx);
                                return false;
                            }
                            let persistence_error = match persisted {
                                Ok(restored) => {
                                    if let Some((submissions, history)) = restored {
                                        this.restore_history(submissions, history);
                                    }
                                    None
                                }
                                Err(error) => Some(error),
                            };
                            this.accept_response(&response, cx);
                            if let Some(error) = persistence_error {
                                this.error = Some(format!("The saved status could not be updated: {error}. Check status again to finish recovery.").into());
                            }
                            true
                        })
                        .log_err().unwrap_or(false);
                    if !applied {
                        return;
                    }
                    if !response.should_poll() {
                        return;
                    }
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            this.sync_account(cx);
                            return;
                        }
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
            if this.client.account_access_token() != account {
                this.sync_account(cx);
                return;
            }
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
        let Some(journal) = self
            .journal
            .clone()
            .filter(|_| self.client.account_access_token().is_some())
        else {
            self.fail(
                anyhow!(
                    "Account recovery is not ready. Refresh models before opening this experiment."
                ),
                cx,
            );
            return;
        };
        let Some(run) = self
            .history
            .iter()
            .find(|saved| match (&saved.result, &run.result) {
                (RunResult::Generation { id: saved }, RunResult::Generation { id: requested }) => {
                    saved == requested
                }
                (
                    RunResult::VectorMessage {
                        message_id: saved_id,
                        svg: saved_svg,
                    },
                    RunResult::VectorMessage {
                        message_id: requested_id,
                        svg: requested_svg,
                    },
                ) => saved_id == requested_id && saved_svg == requested_svg,
                _ => false,
            })
            .cloned()
        else {
            self.fail(
                anyhow!("This experiment is no longer in the current account's saved history."),
                cx,
            );
            return;
        };
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let account = self.client.account_access_token();
        self.outputs.clear();
        self.preview = None;
        self.preview_task = None;
        self.prepared_video = None;
        self.clear_playback(cx);
        self.preview_request = None;
        self.pending = false;
        self.active_run = Some(run.clone());
        if let RunResult::VectorMessage { svg, .. } = &run.result {
            self.outputs = vec![vector_output(svg.clone())];
            self.selected_output = 0;
            self.error = None;
            self.status = "Vector artwork restored from your saved experiments.".into();
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
                cx.background_executor(),
            )
            .await
            .and_then(parse_generation_response)
            .and_then(|response| {
                ensure!(response.id == generation_id, "The status response did not match the saved generation.");
                Ok(response)
            });
            match result {
                Ok(mut response) => {
                    let persisted = persist_generation_status(&journal, &mut response, account.clone(), cx).await;
                    let applied = this.update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            this.sync_account(cx);
                            return false;
                        }
                        let persistence_error = match persisted {
                            Ok(restored) => {
                                if let Some((submissions, history)) = restored {
                                    this.restore_history(submissions, history);
                                }
                                None
                            }
                            Err(error) => Some(error),
                        };
                        this.accept_response(&response, cx);
                        if let Some(error) = persistence_error {
                            this.error = Some(format!("The saved status could not be updated: {error}. Check status again to finish recovery.").into());
                        }
                        true
                    })
                        .log_err().unwrap_or(false);
                    if applied && response.should_poll() {
                        Self::poll(
                            this.clone(),
                            client,
                            base_url,
                            response.id,
                            account.clone(),
                            journal.clone(),
                            cx,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        if this.client.account_access_token() != account {
                            this.sync_account(cx);
                            return;
                        }
                        this.fail(error, cx);
                    }).log_err();
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
            self.status = if response.awaiting_input() {
                "This job is waiting for input. Use its saved request to retry delivery."
            } else if response.status == "warming" {
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
                    if let Some(seed) = &response.seed {
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

    fn restore_video_playback(&mut self, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        {
            self.clear_playback(_cx);
            let Some(video) = self.prepared_video.as_ref() else {
                return;
            };
            let bytes = video.bytes.clone();
            #[cfg(test)]
            let playback = self
                .playback_factory
                .as_ref()
                .map(|factory| factory(bytes.clone(), _cx))
                .unwrap_or_else(|| {
                    _cx.new(|cx| {
                        crate::video_playback::VideoPlaybackView::new(bytes, PREVIEW_SIZE, cx)
                    })
                });
            #[cfg(not(test))]
            let playback = {
                _cx.new(|cx| crate::video_playback::VideoPlaybackView::new(bytes, PREVIEW_SIZE, cx))
            };
            playback.update(_cx, |playback, cx| {
                playback.set_active(self.playback_active, cx)
            });
            self.playback = Some(playback);
        }
    }

    fn clear_playback(&mut self, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if let Some(playback) = self.playback.take() {
            playback.update(_cx, |playback, cx| playback.close(cx));
        }
        #[cfg(target_os = "macos")]
        if let Some(playback) = self.audio_playback.take() {
            playback.update(_cx, |playback, cx| playback.close(cx));
        }
    }

    fn set_playback_active(&mut self, active: bool, _cx: &mut Context<Self>) {
        self.playback_active = active;
        #[cfg(target_os = "macos")]
        if let Some(playback) = self.playback.as_ref() {
            playback.update(_cx, |playback, cx| playback.set_active(active, cx));
        }
        #[cfg(target_os = "macos")]
        if !active {
            if let Some(playback) = self.audio_playback.as_ref() {
                playback.update(_cx, |playback, cx| playback.pause(cx));
            }
        }
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        self.play_after_preview = false;
        self.preview = None;
        self.preview_task = None;
        self.prepared_video = None;
        self.clear_playback(cx);
        self.preview_request = None;
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        if !output.mime.starts_with("image/")
            && !output.mime.starts_with("video/")
            && !output.mime.starts_with("audio/")
        {
            return;
        }
        let client = self.client.clone();
        let account = client.account_access_token();
        let request = uuid::Uuid::new_v4();
        self.preview_request = Some(request);
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes =
                    media_bytes(&client, &output.location, cx.background_executor()).await?;
                if output.mime.starts_with("audio/") {
                    Ok::<_, anyhow::Error>((None, None, Some(bytes)))
                } else if output.mime.starts_with("video/") {
                    network_deadline(
                        cx.background_executor(),
                        VIDEO_PREVIEW_TIMEOUT,
                        "The video preview took too long. Try loading the result again.",
                        cx.background_spawn(async move {
                            let prepared = generation_media::prepare_video(bytes).await?;
                            let preview = prepared
                                .poster
                                .as_ref()
                                .map(|poster| make_preview(&poster.png, "image/png"))
                                .transpose()?;
                            Ok((preview, Some(Arc::new(prepared)), None))
                        }),
                    )
                    .await
                } else {
                    let preview = cx
                        .background_spawn(async move { make_preview(&bytes, &output.mime) })
                        .await?;
                    Ok::<_, anyhow::Error>((Some(preview), None, None))
                }
            }
            .await;
            this.update(cx, |this, cx| {
                if this.client.account_access_token() != account {
                    this.sync_account(cx);
                    return;
                }
                if this.preview_request != Some(request) || this.playback_removed.get() {
                    return;
                }
                this.preview_task = None;
                this.preview_request = None;
                match result {
                    Ok((preview, video, audio)) => {
                        this.preview = preview;
                        this.prepared_video = video;
                        this.restore_video_playback(cx);
                        #[cfg(target_os = "macos")]
                        if let Some(bytes) = audio {
                            this.audio_playback = Some(cx.new(|_| AudioPlaybackView::new(bytes)));
                        }
                        #[cfg(not(target_os = "macos"))]
                        drop(audio);
                        if this.play_after_preview {
                            this.play_after_preview = false;
                            #[cfg(target_os = "macos")]
                            if let Some(playback) = this.playback.as_ref() {
                                playback.update(cx, |playback, cx| playback.play(cx));
                            }
                            #[cfg(target_os = "macos")]
                            if let Some(playback) = this.audio_playback.as_ref() {
                                playback.update(cx, |playback, cx| playback.play(cx));
                            }
                        }
                    }
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
        self.choose_image_source(false, cx);
    }

    fn choose_end_frame(&mut self, cx: &mut Context<Self>) {
        self.choose_image_source(true, cx);
    }

    fn choose_voice_reference(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a reference voice WAV".into()),
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<Option<VoiceSource>> = async {
                let Some(path) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                workspace::remember_user_selected_paths(std::slice::from_ref(&path))?;
                cx.background_spawn(async move {
                    ensure!(
                        std::fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
                        "Choose a reference voice smaller than 8 MB."
                    );
                    let bytes = std::fs::read(&path)?;
                    ensure!(
                        bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
                        "Choose a WAV reference voice."
                    );
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Reference voice.wav".into());
                    Ok(Some(VoiceSource {
                        name,
                        bytes: bytes.into(),
                    }))
                })
                .await
            }
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(Some(reference)) => {
                        this.voice_reference = Some(reference);
                        this.voice_consent_granted = false;
                        this.error = None;
                    }
                    Ok(None) => {}
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn choose_image_source(&mut self, end_frame: bool, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let hosted_frame = self.mode.generation_kind().is_some();
        let default_max_source_bytes = if hosted_frame {
            20 * 1024 * 1024
        } else {
            MAX_SOURCE_BYTES
        };
        let frame_input = self
            .selected_model
            .as_ref()
            .and_then(|selected| self.models.iter().find(|model| &model.id == selected))
            .map(|model| {
                &model.capabilities["inputs"][if end_frame {
                    "end_frame"
                } else {
                    "start_frame"
                }]
            });
        let max_source_bytes = frame_input
            .and_then(|input| input["max_bytes"].as_u64())
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(default_max_source_bytes)
            .min(default_max_source_bytes);
        let accepted_mime_types = frame_input
            .and_then(|input| input["mime_types"].as_array())
            .map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if end_frame {
                    "Choose end frame"
                } else {
                    "Choose start image"
                }
                .into(),
            ),
        });
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<Option<SourceImage>> = async {
                let Some(path) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                workspace::remember_user_selected_paths(std::slice::from_ref(&path))?;
                let (bytes, name, mime, preview) = cx
                    .background_spawn(async move {
                        ensure!(
                            std::fs::metadata(&path)?.len() <= max_source_bytes as u64,
                            "Choose an image no larger than {} MB for this model.",
                            max_source_bytes.div_ceil(1024 * 1024)
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
                        ensure!(
                            accepted_mime_types.is_empty()
                                || accepted_mime_types.iter().any(|accepted| accepted == mime),
                            "Choose an image format supported by this model."
                        );
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
                let asset = upload_image(
                    &client,
                    &base_url,
                    bytes,
                    &name,
                    mime,
                    &preview,
                    cx.background_executor(),
                )
                .await?;
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
                    Ok(Some(source)) if end_frame => {
                        this.end_frame = Some(source);
                        this.error = None;
                        cx.notify();
                    }
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
                    cx.background_executor(),
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

    fn use_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                self.set_mode(GenerationMode::Image, window, cx);
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
                    let bytes =
                        media_bytes(&client, &output.location, cx.background_executor()).await?;
                    let asset = upload_image(
                        &client,
                        &base_url,
                        bytes.to_vec(),
                        "Generated source",
                        &output.mime,
                        &preview,
                        cx.background_executor(),
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
                    let asset = api_json(&client, &base_url, Method::GET, &format!("/v1/assets/{id}"), None, None, account.as_deref(), cx.background_executor()).await?;
                    MediaLocation::Url(asset["url"].as_str().context("The source image is no longer available.")?.to_owned())
                } else if let Some(id) = source.reference["generation_id"].as_str() {
                    let generation = api_json(&client, &base_url, Method::GET, &format!("/v1/generations/{id}"), None, None, account.as_deref(), cx.background_executor()).await?;
                    let outputs: Vec<Value> = serde_json::from_value(generation["output"].clone())?;
                    normalize_outputs(&outputs)?.into_iter().next().context("The original image is unavailable.")?.location
                } else { bail!("The original source image is unavailable."); };
                let image = media_bytes(&client, &location, cx.background_executor()).await?;
                let mask = media_bytes(&client, &output.location, cx.background_executor()).await?;
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
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/flac" => "flac",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "png",
        };
        let name = format!("fanta-generation.{extension}");
        let path = cx.prompt_for_new_path(&PathBuf::from(paths::home_dir().as_path()), Some(&name));
        let client = self.client.clone();
        let account = client.account_access_token();
        let cached_video = self.prepared_video.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<bool> = async {
                let Some(path) = path.await?? else {
                    return Ok(false);
                };
                ensure!(
                    client.account_access_token() == account,
                    "Your Fanta account changed. Choose the result again before saving it."
                );
                let bytes = match cached_video {
                    Some(video) if output.mime.starts_with("video/") => video.bytes.clone(),
                    _ => media_bytes(&client, &output.location, cx.background_executor()).await?,
                };
                ensure!(
                    client.account_access_token() == account,
                    "Your Fanta account changed. Choose the result again before saving it."
                );
                #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                let saved_path = path.clone();
                cx.background_spawn(async move { generation_media::write_output(&path, &bytes) })
                    .await?;
                #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                if let Err(error) =
                    workspace::remember_user_selected_paths(std::slice::from_ref(&saved_path))
                {
                    log::warn!("Could not retain access to the saved generation: {error:#}");
                }
                Ok(true)
            }
            .await;
            this.update(cx, |this, cx| {
                if this.client.account_access_token() != account {
                    this.sync_account(cx);
                    return;
                }
                this.task = None;
                match result {
                    Ok(true) => {
                        this.error = None;
                        this.status = "Result saved.".into();
                    }
                    Ok(false) => {}
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    #[cfg(not(target_os = "macos"))]
    fn play_output(&mut self, cx: &mut Context<Self>) {
        self.sync_account(cx);
        if self.task.is_some() {
            return;
        }
        let Some(output) = self.outputs.get(self.selected_output).cloned() else {
            return;
        };
        let client = self.client.clone();
        let account = client.account_access_token();
        let cached_video = self.prepared_video.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let bytes = match cached_video {
                    Some(video) if output.mime.starts_with("video/") => video.bytes.clone(),
                    _ => media_bytes(&client, &output.location, cx.background_executor()).await?,
                };
                ensure!(
                    client.account_access_token() == account,
                    "Your Fanta account changed. Choose the result again before playing it."
                );
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
                if this.client.account_access_token() != account {
                    this.sync_account(cx);
                    return;
                }
                this.task = None;
                match result {
                    Ok(file) => {
                        this.error = None;
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
        if output.mime.starts_with("video/") && self.preview_task.is_some() {
            return;
        }
        let Some(item) = self.canvas_item.clone() else {
            self.fail(
                anyhow!("Open this tool from a design canvas to place the result."),
                cx,
            );
            return;
        };
        let client = self.client.clone();
        let run = self.active_run.clone();
        let account = client.account_access_token();
        let cached_video = self.prepared_video.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let media = if output.mime.starts_with("video/") {
                    let video = if let Some(video) = cached_video { video } else {
                        let bytes = media_bytes(&client, &output.location, cx.background_executor()).await?;
                        Arc::new(network_deadline(
                            cx.background_executor(), VIDEO_PREVIEW_TIMEOUT,
                            "The video preview took too long. Try adding the result again.",
                            cx.background_spawn(async move { generation_media::prepare_video(bytes).await }),
                        ).await?)
                    };
                    PreparedPlacement::Video(video)
                } else {
                    let bytes = media_bytes(&client, &output.location, cx.background_executor()).await?;
                    cx.background_spawn(async move {
                        if output.mime == "image/svg+xml" {
                            Ok::<_, anyhow::Error>(PreparedPlacement::Vector(generation_media::parse_svg(&bytes)?))
                        } else {
                            let preview = make_preview(&bytes, &output.mime)?;
                            Ok(PreparedPlacement::Image(STANDARD.encode(bytes), preview.width, preview.height))
                        }
                    }).await?
                };
                cx.update(|cx| {
                    ensure!(client.account_access_token() == account,
                        "Your Fanta account changed. Choose the result again before adding it.");
                    let item = item.upgrade().context("The source design was closed. Save the result and open a design to place it.")?;
                    agent_surface::set_active_item(item.downgrade(), cx);
                    let surface = design_surface::active(cx).context("The design canvas is unavailable.")?;
                    let (width, height) = match &media {
                        PreparedPlacement::Image(_, width, height) => (*width as f64, *height as f64),
                        PreparedPlacement::Vector(artwork) => (artwork.width, artwork.height),
                        PreparedPlacement::Video(video) => (video.metadata.width as f64, video.metadata.height as f64),
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
                                PreparedPlacement::Video(video) => generation_media::place_video(document, (*video).clone(), x, y, meta),
                                PreparedPlacement::Image(_, _, _) => (Err(anyhow!("The media type changed before placement.")), crate::document::DocChange::None),
                            }).context("The design is still loading.")?
                        }),
                    }
                })
            }.await;
            this.update(cx, |this, cx| {
                if this.client.account_access_token() != account {
                    this.sync_account(cx);
                    return;
                }
                this.task = None;
                match result { Ok(()) => { this.error = None; this.status = "Added to your design. Save the design to keep it.".into(); }, Err(error) => this.fail(error, cx) }
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
                "Create this design in the active Fanta canvas using editable native layers: {}\n\nUse design_state to inspect the current page and find empty_space for a new top-level frame. Keep existing work. Use design_edit to create frames, text, and shapes, applying auto layout where useful. Finish by checking design_screenshot.",
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
            .w_full()
            .flex_shrink_0()
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
                    .debug_selector(|| "generation-source-preview".into())
                    .relative()
                    .w_full()
                    .h(px(200.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .bg(cx.theme().colors().editor_background)
                    .rounded_md()
                    .child(
                        img(preview.image)
                            .absolute()
                            .inset_0()
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
                        .inset_0()
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
        #[cfg(target_os = "macos")]
        let inline_playback = self.playback.clone();
        #[cfg(not(target_os = "macos"))]
        let has_inline_playback = false;
        #[cfg(target_os = "macos")]
        let has_inline_playback = inline_playback.is_some();
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
                    .when_some(self.preview.clone().filter(|_| !has_inline_playback), |element, preview| {
                        element.child(
                            img(preview.image)
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .map(|element| {
                        #[cfg(target_os = "macos")]
                        let element = element.when_some(inline_playback, |element, playback| {
                            element.child(div().size_full().p_2().child(playback))
                        });
                        element
                    })
                    .when(self.preview.is_none() && !has_inline_playback, |element| {
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
                            .map(|element| {
                                #[cfg(not(target_os = "macos"))]
                                let element = element.when(output.as_ref().is_some_and(|output| output.mime.starts_with("video/")), |element| element.child(
                                    Button::new("play-generation", "Play video").disabled(self.task.is_some())
                                        .on_click(cx.listener(|this, _, _, cx| this.play_output(cx)))));
                                element
                            })
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
                                    .on_click(cx.listener(|this, _, window, cx| this.use_result(window, cx))),
                                )
                            })
                            .when(output.as_ref().is_some_and(|output| !output.mask) && self.canvas_item.is_some(), |element| {
                                element.child(
                                    Button::new("place-generation", "Add to design")
                                        .style(ButtonStyle::Filled)
                                        .disabled(self.task.is_some() || (output.as_ref().is_some_and(|output| output.mime.starts_with("video/")) && self.preview_task.is_some()))
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
        self.set_playback_active(true, cx);
        if let Some(screen) = self.generation_screen.clone() {
            let data = self.generation_view_data(cx);
            screen.update(cx, |screen, cx| screen.set_view_data(data, window, cx));
            return v_flex()
                .id("fanta-generation-workspace")
                .size_full()
                .min_h_0()
                .bg(cx.theme().colors().editor_background)
                .when(self.client.account_access_token().is_none(), |element| {
                    element.child(
                        h_flex()
                            .p_2()
                            .gap_2()
                            .child(
                                Label::new("Sign in to use Fanta generation.").color(Color::Muted),
                            )
                            .child(
                                Button::new("generation-sign-in", "Sign in")
                                    .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                            ),
                    )
                })
                .child(div().flex_1().min_h_0().child(screen))
                .when(
                    self.unresolved_submission.is_some() && self.task.is_none(),
                    |element| {
                        element.child(
                            h_flex()
                                .p_2()
                                .gap_2()
                                .child(
                                    Label::new("A saved request needs recovery.")
                                        .color(Color::Muted),
                                )
                                .child(
                                    Button::new(
                                        "retry-generation-submission",
                                        "Retry saved request",
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            if let Some(submission) =
                                                this.unresolved_submission.clone()
                                            {
                                                this.submit(submission, cx);
                                            }
                                        },
                                    )),
                                ),
                        )
                    },
                )
                .into_any_element();
        }
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
            .child(h_flex().flex_shrink_0().gap_2().flex_wrap().justify_between()
                .child(Label::new("Create with Fanta").size(LabelSize::Large).weight(gpui::FontWeight::SEMIBOLD))
                .child(h_flex().gap_2()
                    .child(Label::new(if signed_in { "Fanta account connected" } else { "Connect your account to generate" }).color(Color::Muted))
                    .when(!signed_in || self.error.is_some(), |element| element.child(Button::new("generation-sign-in", "Sign in")
                        .disabled(self.task.is_some()).on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))))
                    .child(Button::new("generation-billing", "Credits & billing").on_click(|_, _window, cx| {
                        #[cfg(feature = "mac_app_store")]
                        _window.dispatch_action(zed_actions::OpenAccountSettings.boxed_clone(), cx);
                        #[cfg(not(feature = "mac_app_store"))]
                        cx.open_url(&client::zed_urls::account_url(cx));
                    }))))
            .child(h_flex().flex_shrink_0().gap_1().flex_wrap().children(GenerationMode::CREATION_MODES.into_iter().map(|mode| {
                Button::new(("generation-mode", mode as usize), mode.label()).toggle_state(self.mode == mode)
                    .on_click(cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)))
            })))
            .child(h_flex().flex_shrink_0().items_start().gap_5().flex_wrap()
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
                        .child(Button::new("refresh-generation-models", "Refresh models").disabled(self.catalog_task.is_some() || self.task.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_catalog(cx)))))
                    .child(Label::new(if self.mode == GenerationMode::Masks { "What to select (optional)" } else if trace_vectors { "Guidance (optional)" } else { "Your idea" }).color(Color::Muted))
                    // Auto-height editors need a definite width; InputField's
                    // horizontal row measures this editor as zero-sized.
                    .child(v_flex().id("generation-prompt").w_full().min_h_8().flex_shrink_0().p_2()
                        .rounded_md().border_1().border_color(cx.theme().colors().border_variant)
                        .bg(cx.theme().colors().editor_background)
                        .when(self.prompt.read(cx).focus_handle(cx).contains_focused(window, cx), |element| element.border_color(cx.theme().colors().border_focused))
                        .child(self.prompt.read(cx).editor().render(window, cx)))
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
                    .child(div().debug_selector(|| "generation-submit".into()).child(Button::new("submit-generation", if is_design { "Prepare design brief" } else if self.mode == GenerationMode::Masks { "Generate masks" } else if prompt_vectors { "Create vectors" } else if trace_vectors { "Trace image" } else { "Generate" })
                        .style(ButtonStyle::Filled).full_width()
                        .disabled(self.task.is_some() || !signed_in || (!is_design && (self.journal.is_none() || self.model().is_none() || self.unresolved_submission.is_some())))
                        .on_click(cx.listener(|this, _, window, cx| this.generate(window, cx)))))
                    .when(self.unresolved_submission.is_some() && self.task.is_none(), |element| element
                        .child(Label::new("A saved request needs recovery. Retrying preserves its original input and recovery key.").color(Color::Muted))
                        .child(Button::new("retry-generation-submission", "Retry submission").disabled(self.journal.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(submission) = this.unresolved_submission.clone() { this.submit(submission, cx); }
                            })))
                        .children(self.recovered_submissions.iter().enumerate().map(|(index, submission)| {
                            Button::new(("retry-saved-generation", index), format!("Recover {} · {}", submission.mode.label(), submission.prompt.chars().take(48).collect::<String>()))
                                .disabled(self.journal.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if index < this.recovered_submissions.len() {
                                        let submission = this.recovered_submissions.remove(index);
                                        if let Some(previous) = this.unresolved_submission.replace(submission.clone()) {
                                            this.recovered_submissions.push(previous);
                                        }
                                        this.submit(submission, cx);
                                    }
                                }))
                        })))
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
            .into_any_element()
    }
}

impl EventEmitter<ItemEvent> for GenerationWorkspace {}
impl Focusable for GenerationWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.generation_screen
            .as_ref()
            .map(|screen| screen.read(cx).focus_handle(cx))
            .unwrap_or_else(|| self.prompt.read(cx).focus_handle(cx))
    }
}
impl Item for GenerationWorkspace {
    type Event = ItemEvent;
    fn added_to_workspace(&mut self, _: &mut Workspace, _: &mut Window, _cx: &mut Context<Self>) {
        if self.playback_removed.replace(false) {
            self.restore_video_playback(_cx);
        }
    }
    fn deactivated(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.set_playback_active(false, cx);
    }
    fn workspace_deactivated(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.set_playback_active(false, cx);
    }
    fn on_removed(&self, _cx: &mut Context<Self>) {
        self.playback_removed.set(true);
        #[cfg(target_os = "macos")]
        if let Some(playback) = self.playback.as_ref() {
            playback.update(_cx, |playback, cx| playback.close(cx));
        }
        #[cfg(target_os = "macos")]
        if let Some(playback) = self.audio_playback.as_ref() {
            playback.update(_cx, |playback, cx| playback.close(cx));
        }
    }
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
    executor: &gpui::BackgroundExecutor,
) -> Result<Vec<GenerationModel>> {
    let value = api_json(
        client,
        base_url,
        Method::GET,
        "/v1/models",
        None,
        None,
        expected_account,
        executor,
    )
    .await?;
    let mut models: Vec<GenerationModel> = serde_json::from_value(value["models"].clone())
        .context("The model catalog could not be read")?;
    models.sort_by_key(|model| {
        (
            !model.gateway(),
            model.capabilities["hosted"] != true,
            model.label().to_lowercase(),
        )
    });
    Ok(models)
}

async fn api_json(
    client: &Arc<Client>,
    base_url: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    idempotency_key: Option<&str>,
    expected_account: Option<&str>,
    executor: &gpui::BackgroundExecutor,
) -> Result<Value> {
    let generation_submission = method == Method::POST && path == "/v1/generations";
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
    let request = request.body(body)?;
    let limit = if path == "/v1/messages" {
        MAX_VECTOR_RESPONSE_BYTES
    } else {
        MAX_JSON_BYTES
    };
    let (status, unreserved, bytes) = network_deadline(
        executor,
        API_TIMEOUT,
        "The Fanta request timed out. Check your connection and try again.",
        async {
            let response = client
                .http_client()
                .send(request)
                .await
                .context("Could not reach Fanta. Check your connection and try again.")?;
            let status = response.status();
            let unreserved = generation_submission
                && response
                    .headers()
                    .get("x-fanta-generation-unreserved")
                    .is_some_and(|value| value == "true");
            let bytes = bounded_body(response.into_body(), limit).await?;
            Ok((status, unreserved, bytes))
        },
    )
    .await?;
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
            unreserved,
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

async fn network_deadline<T>(
    executor: &gpui::BackgroundExecutor,
    timeout: Duration,
    message: &'static str,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let deadline = executor.timer(timeout);
    futures::pin_mut!(operation, deadline);
    match futures::future::select(operation, deadline).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => Err(anyhow!(message)),
    }
}

async fn media_bytes(
    client: &Arc<Client>,
    location: &MediaLocation,
    executor: &gpui::BackgroundExecutor,
) -> Result<Arc<[u8]>> {
    match location {
        MediaLocation::Inline(bytes) => Ok(bytes.clone()),
        MediaLocation::Url(url) => {
            let url = url::Url::parse(url)?;
            ensure!(url.scheme() == "https", "The result URL must use HTTPS.");
            network_deadline(
                executor,
                MEDIA_TRANSFER_TIMEOUT,
                "The result download timed out. Try again, or select the experiment to refresh its link.",
                async {
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
                },
            )
            .await
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
    executor: &gpui::BackgroundExecutor,
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
        executor,
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
    network_deadline(
        executor,
        MEDIA_TRANSFER_TIMEOUT,
        "The image upload timed out. Choose the image again to retry.",
        async {
            let response = client.http_client().send(request).await?;
            ensure!(
                response.status().is_success(),
                "The image upload failed. Choose the image again to retry."
            );
            bounded_body(response.into_body(), MAX_JSON_BYTES).await?;
            Ok(())
        },
    )
    .await?;
    api_json(
        client,
        base_url,
        Method::POST,
        &format!("/v1/assets/uploads/{id}/complete"),
        Some(json!({})),
        None,
        account.as_deref(),
        executor,
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

type ProviderMark = (
    &'static str,
    Option<(&'static str, ImageFormat, &'static [u8])>,
);

fn provider_mark(id: &str, provider: Option<&str>) -> ProviderMark {
    let provider_is =
        |name: &str| provider.is_some_and(|provider| provider.eq_ignore_ascii_case(name));
    if id.contains("recraft") || provider_is("Recraft") {
        ("Recraft", Some(("recraft", ImageFormat::Png, RECRAFT_LOGO)))
    } else if id.contains("arrow") || id.contains("quiver") || provider_is("QuiverAI") {
        (
            "QuiverAI",
            Some(("quiverai", ImageFormat::Png, QUIVER_LOGO)),
        )
    } else if id.contains("seedream") || id.contains("seedance") || provider_is("ByteDance") {
        (
            "ByteDance Seed",
            Some(("bytedance-seed", ImageFormat::Png, BYTEDANCE_LOGO)),
        )
    } else if id.contains("minimax") || id.contains("speech-2.8") || provider_is("MiniMax") {
        (
            "MiniMax",
            Some(("minimax", ImageFormat::Jpeg, MINIMAX_LOGO)),
        )
    } else if id.contains("eleven") || provider_is("ElevenLabs") {
        (
            "ElevenLabs",
            Some(("elevenlabs", ImageFormat::Png, ELEVENLABS_LOGO)),
        )
    } else if id.contains("veo") || provider_is("Google") {
        ("Google", Some(("google", ImageFormat::Png, GOOGLE_LOGO)))
    } else if id.contains("kling") || provider_is("Kling") || provider_is("Kling AI") {
        ("Kling AI", Some(("kling", ImageFormat::Jpeg, KLING_LOGO)))
    } else if id.contains("muse") || provider_is("Meta") {
        ("Meta", Some(("meta", ImageFormat::Png, META_LOGO)))
    } else if id.contains("grok") || provider_is("xAI") {
        ("xAI", Some(("xai", ImageFormat::Png, XAI_LOGO)))
    } else if id.starts_with("gpt-") || id.starts_with("sora-") || provider_is("OpenAI") {
        ("OpenAI", Some(("openai", ImageFormat::Svg, OPENAI_LOGO)))
    } else if id.contains("qwen") || id == "fanta-image-1" || provider_is("Qwen") {
        ("Qwen", Some(("qwen", ImageFormat::Png, QWEN_LOGO)))
    } else if id.contains("z-image") || id == "fanta-image-fast-1" || provider_is("Tongyi-MAI") {
        (
            "Tongyi-MAI",
            Some(("tongyi", ImageFormat::Jpeg, TONGYI_LOGO)),
        )
    } else if id.contains("flux")
        || id.contains("black-forest")
        || id == "fanta-vectorize-1"
        || provider_is("Black Forest Labs")
    {
        (
            "Black Forest Labs",
            Some(("bfl", ImageFormat::Png, BFL_LOGO)),
        )
    } else if id.contains("ltx") || id == "fanta-video-1" || provider_is("Lightricks") {
        ("Lightricks", Some(("ltx", ImageFormat::Svg, LTX_LOGO)))
    } else if id.contains("wan")
        || id == "fanta-video-hd-1"
        || id == "fanta-animate-1"
        || provider_is("Wan")
    {
        ("Wan", Some(("wan", ImageFormat::Png, WAN_LOGO)))
    } else if id.contains("chatterbox")
        || id.contains("fanta-voice-") && !id.contains("fast")
        || provider_is("Resemble AI")
    {
        (
            "Resemble AI",
            Some(("resemble", ImageFormat::Png, RESEMBLE_LOGO)),
        )
    } else if id.contains("kokoro") || id == "fanta-voice-fast-1" || provider_is("Hexgrad") {
        ("Hexgrad", Some(("hexgrad", ImageFormat::Png, HEXGRAD_LOGO)))
    } else if id.contains("ace-step") || id == "fanta-music-1" || provider_is("ACE-Step") {
        ("ACE-Step", Some(("ace", ImageFormat::Jpeg, ACE_LOGO)))
    } else if id.contains("starvector") || id == "fanta-svg-1" || provider_is("StarVector") {
        (
            "StarVector",
            Some(("starvector", ImageFormat::Jpeg, STARVECTOR_LOGO)),
        )
    } else if id.starts_with("claude-") || provider_is("Anthropic") {
        ("Anthropic", Some(("claude", ImageFormat::Png, CLAUDE_LOGO)))
    } else {
        ("AI provider", None)
    }
}

fn option_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_bool().map(|value| value.to_string()))
        .or_else(|| value.as_number().map(ToString::to_string))
}

fn option_label(key: &str) -> String {
    match key {
        "aspect_ratio" => return "Aspect ratio".into(),
        "audio_format" | "output_format" => return "Output format".into(),
        "camera_motion" => return "Camera motion".into(),
        "duration" | "duration_s" | "music_length_ms" => return "Duration".into(),
        "fps" => return "Frame rate".into(),
        "size" | "canvas" => return "Canvas size".into(),
        "voice" | "voice_id" => return "Voice".into(),
        "speed" => return "Pace".into(),
        "generate_audio" => return "Generate audio".into(),
        "force_instrumental" => return "Instrumental".into(),
        "enable_prompt_expansion" => return "Prompt expansion".into(),
        _ => {}
    }
    let mut words = key.split('_');
    let first = words.next().unwrap_or(key);
    let mut label = String::new();
    let mut chars = first.chars();
    if let Some(first) = chars.next() {
        label.extend(first.to_uppercase());
        label.extend(chars);
    }
    for word in words {
        label.push(' ');
        label.push_str(word);
    }
    label
}

fn control_priority(key: &str) -> u8 {
    match key {
        "size" | "canvas" | "aspect_ratio" | "resolution" | "duration" | "duration_s"
        | "music_length_ms" | "voice" | "voice_id" => 0,
        _ => 1,
    }
}

fn option_choice_label(key: &str, value: &str) -> String {
    if matches!(value, "true" | "false") {
        return if value == "true" { "On" } else { "Off" }.into();
    }
    if matches!(key, "duration" | "duration_s") {
        return format!("{value} s");
    }
    if key == "music_length_ms" {
        if let Ok(milliseconds) = value.parse::<u64>() {
            return format!("{} s", milliseconds / 1000);
        }
    }
    if key == "aspect_ratio" {
        return match value {
            "16:9" => "Landscape · 16:9".into(),
            "9:16" => "Portrait · 9:16".into(),
            "1:1" => "Square · 1:1".into(),
            _ => value.to_owned(),
        };
    }
    if value.contains('_') {
        return value.replace('_', " ");
    }
    value.to_owned()
}

fn generation_templates() -> Vec<GenerationTemplate> {
    [
        ("image-editorial", "Editorial portrait", GenerationKind::Image, "An editorial portrait in soft window light, refined composition and natural skin texture"),
        ("image-product", "Product study", GenerationKind::Image, "A sculptural product photograph on a dark stone plinth with a single directional key light"),
        ("video-orbit", "Cinematic orbit", GenerationKind::Video, "A slow cinematic orbit around the subject, subtle depth of field and smooth natural motion"),
        ("video-atmosphere", "Atmospheric scene", GenerationKind::Video, "A quiet atmospheric scene with drifting mist, restrained camera movement and rich detail"),
        ("audio-narration", "Narration", GenerationKind::Audio, "Welcome to a new way of creating. Every detail begins with an idea."),
        ("audio-music", "Ambient score", GenerationKind::Audio, "A warm ambient score with soft synthesizers, subtle piano, and a gradual emotional rise"),
        ("svg-symbol", "Geometric symbol", GenerationKind::Svg, "Create a precise geometric compass symbol with clean paths and balanced negative space"),
        ("svg-monogram", "Monogram", GenerationKind::Svg, "Create a refined geometric monogram with consistent stroke weight and simple vector shapes"),
    ]
    .into_iter()
    .map(|(id, title, kind, prompt)| GenerationTemplate::new(id, title, kind, prompt))
    .collect()
}

fn build_screen_request(
    model: &GenerationModel,
    submission: &GenerationSubmission,
    source: Option<&SourceImage>,
    end_frame: Option<&SourceImage>,
    voice_reference: Option<&VoiceSource>,
) -> Result<Value> {
    let prompt_limit = model.capabilities["prompt_limit"]
        .as_u64()
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(1_200);
    ensure!(
        submission.prompt.encode_utf16().count() <= prompt_limit,
        "Keep the prompt under {prompt_limit} characters."
    );
    let mut request = json!({ "model": model.id, "kind": model.kind });
    if !submission.prompt.trim().is_empty() {
        request["prompt"] = json!(if submission.recipe == GenerationRecipe::Speech {
            submission.prompt.as_ref()
        } else {
            submission.prompt.trim()
        });
    }
    if let Some(negative) = submission.negative.as_ref() {
        request["negative"] = json!(negative.as_ref());
    }
    if let Some(seed) = submission.seed {
        request["seed"] = json!(seed);
    }
    let mut input = json!({});
    for option in &submission.options {
        let key = option.key.as_ref();
        let value = option.value.as_ref();
        let control = &model.capabilities["controls"][key];
        if !control.is_object() && (key == "canvas" || key == "size" && value.contains('x')) {
            let (width, height) = parse_size(value)?;
            request["width"] = json!(width);
            request["height"] = json!(height);
            continue;
        }
        if let Some(values) = control["values"].as_array() {
            ensure!(
                values
                    .iter()
                    .any(|candidate| option_value(candidate).as_deref() == Some(value)),
                "Choose a valid {}.",
                option_label(key),
            );
        }
        let field = control["request_field"].as_str().unwrap_or(key);
        let encoded = if control["default"].is_boolean() || value == "true" || value == "false" {
            ensure!(matches!(value, "true" | "false"), "Choose a valid {key}.");
            json!(value == "true")
        } else if control["default"].is_number()
            || matches!(
                key,
                "frames"
                    | "fps"
                    | "steps"
                    | "guidance"
                    | "duration"
                    | "speed"
                    | "music_length_ms"
                    | "num_outputs"
            )
        {
            let number: Value =
                serde_json::from_str(value).with_context(|| format!("Choose a valid {key}."))?;
            ensure!(number.is_number(), "Choose a valid {key}.");
            number
        } else {
            json!(value)
        };
        if control["location"] == "top" {
            request[field] = encoded;
        } else {
            input[field] = encoded;
        }
    }
    let selected_control = |key: &str| {
        submission
            .options
            .iter()
            .find(|option| option.key.as_ref() == key)
            .map(|option| option.value.to_string())
            .or_else(|| option_value(&model.capabilities["controls"][key]["default"]))
    };
    if let (Some(mode), Some(resolution)) =
        (selected_control("mode"), selected_control("resolution"))
    {
        if let Some(allowed) = model.capabilities["constraints"]["mode_resolution"][mode].as_array()
        {
            ensure!(
                allowed
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(resolution.as_str())),
                "Choose a resolution available in this mode."
            );
        }
    }
    if let Some(source) = source.filter(|_| submission.source_id.is_some()) {
        input["source"] = source.reference.clone();
    }
    if let Some(end_frame) = end_frame.filter(|_| submission.end_frame_id.is_some()) {
        input["end_frame"] = end_frame.reference.clone();
    }
    if let Some(reference) = voice_reference.filter(|_| submission.voice_reference_id.is_some()) {
        ensure!(
            submission.voice_consent_granted,
            "Confirm permission to use the reference voice."
        );
        input["voice_ref_b64"] = json!(STANDARD.encode(&reference.bytes));
        input["consent"] = json!(true);
    }
    if model.capabilities["operations"]
        .as_array()
        .is_some_and(|operations| operations.len() > 1)
    {
        input["operation"] = json!(match submission.recipe {
            GenerationRecipe::TextVideo => "text_to_video",
            GenerationRecipe::ImageVideo => "image_to_video",
            _ => bail!("This model does not support the selected operation."),
        });
    }
    if input.as_object().is_some_and(|input| !input.is_empty()) {
        request["input"] = input;
    }
    Ok(request)
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
            ensure!(
                points
                    .iter()
                    .all(|point| [point.x, point.y].into_iter().all(|coordinate| {
                        coordinate.is_finite() && (0.0..=u32::MAX as f64).contains(&coordinate)
                    })),
                "Mask points must be valid source image coordinates."
            );
            input["points"] = json!(
                points
                    .iter()
                    .map(|point| json!({"x":point.x.floor() as u32,"y":point.y.floor() as u32,"positive":point.positive}))
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
        if model.capabilities["negative_prompt"]["supported"].as_bool() == Some(true)
            && !negative.trim().is_empty()
        {
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

fn parse_gallery_response(value: Value) -> Result<(Vec<GalleryRecord>, Option<String>)> {
    let rows = value["generations"]
        .as_array()
        .context("The generation gallery returned no list.")?;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(id) = row["id"].as_str() else {
            continue;
        };
        let Some(kind) = row["kind"].as_str().and_then(|kind| match kind {
            "image" => Some(GenerationKind::Image),
            "video" => Some(GenerationKind::Video),
            "audio" => Some(GenerationKind::Audio),
            "svg" | "vectorize" => Some(GenerationKind::Svg),
            _ => None,
        }) else {
            continue;
        };
        let status = match row["status"].as_str() {
            Some("queued") => GenerationOutputStatus::Queued,
            Some("warming" | "processing") => GenerationOutputStatus::Running,
            Some("succeeded") => GenerationOutputStatus::Succeeded,
            Some("failed") => GenerationOutputStatus::Failed,
            Some("canceled") => GenerationOutputStatus::Canceled,
            _ => continue,
        };
        let outputs = row["output"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let outputs = match normalize_outputs(outputs) {
            Ok(outputs) => outputs,
            Err(error) => {
                log::warn!("Skipping unreadable gallery output {id}: {error:#}");
                Vec::new()
            }
        };
        records.push(GalleryRecord {
            id: id.to_owned(),
            kind,
            model: row["model"].as_str().unwrap_or("Unknown model").to_owned(),
            status,
            created_at: row["created_at"].as_str().unwrap_or_default().to_owned(),
            outputs,
        });
    }
    Ok((records, value["next_before"].as_str().map(str::to_owned)))
}

fn parse_generation_output_id(id: &str) -> Option<(&str, usize)> {
    let id = id.strip_prefix("gen:")?;
    let (generation_id, index) = id.rsplit_once(':')?;
    Some((generation_id, index.parse().ok()?))
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
    (x >= 0. && y >= 0. && x < width as f32 && y < height as f32)
        .then_some((x.floor() as f64, y.floor() as f64))
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

    struct RecoveryTestDirectory {
        _directory: tempfile::TempDir,
    }
    impl gpui::Global for RecoveryTestDirectory {}

    fn initialize_recovery_database(cx: &mut App) {
        if cx.has_global::<db::AppDatabase>() {
            return;
        }
        let directory = tempfile::tempdir().expect("recovery database directory");
        let path = directory.path().join("recovery.sqlite");
        let connection = gpui::block_on(
            db::sqlez::thread_safe_connection::ThreadSafeConnection::builder::<db::AppMigrator>(
                path.to_str().expect("test database path"),
                true,
            )
            .with_write_queue_constructor(db::sqlez::thread_safe_connection::locking_queue())
            .build(),
        )
        .expect("persistent recovery database");
        cx.set_global(db::AppDatabase(connection));
        cx.set_global(RecoveryTestDirectory {
            _directory: directory,
        });
    }

    fn recovery_account_fixture() -> Value {
        json!({
            "user": {"id": "93f29737-8b7f-41b5-9b53-ef6df21d76ce"},
            "org": {"id": "c6c984b9-d716-4ef3-88a3-b599a9824748"},
        })
    }

    fn visual_workspace(
        mode: GenerationMode,
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<GenerationWorkspace>, &mut gpui::VisualTestContext) {
        let http = http_client::FakeHttpClient::create(|_| async {
            panic!("Rendering a signed-out workspace must not submit requests")
        });
        let client = catalog_client(cx, http);
        visual_workspace_with_client(mode, client, cx)
    }

    fn visual_workspace_with_client(
        mode: GenerationMode,
        client: Arc<Client>,
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<GenerationWorkspace>, &mut gpui::VisualTestContext) {
        cx.update(|cx| {
            initialize_recovery_database(cx);
            assets::Assets.load_test_fonts(cx);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            gpui_component::init(cx);
            crate::theme_bridge::init(cx);
            editor::init(cx);
            Client::set_global(client, cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            GenerationWorkspace::new(WeakEntity::new_invalid(), None, mode, window, cx)
        });
        cx.simulate_resize(gpui::size(px(1000.), px(768.)));
        cx.run_until_parked();
        (view, cx)
    }

    #[gpui::test]
    fn generation_prompt_is_visible_and_modes_keep_separate_drafts(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Image, cx);
        view.update_in(cx, |view, window, cx| {
            let mut image = model("image");
            image.id = "replicate-image".into();
            image.capabilities = json!({"hosted":true,"operations":["text_to_image"]});
            let mut video = model("video");
            video.id = "replicate-video".into();
            video.capabilities = json!({"hosted":true,"operations":["text_to_video"]});
            view.models = vec![image, video];
            view.selected_model = Some("replicate-image".into());
            let screen = view.generation_screen.as_ref().expect("image screen");
            screen.update(cx, |screen, cx| {
                screen.set_draft(
                    GenerationDraft {
                        prompt: "a green landscape".into(),
                        ..GenerationDraft::default()
                    },
                    window,
                    cx,
                )
            });
        });
        view.update_in(cx, |view, window, cx| {
            view.set_mode(GenerationMode::Video, window, cx)
        });
        view.read_with(cx, |view, cx| {
            assert!(
                view.generation_screen
                    .as_ref()
                    .expect("video screen")
                    .read(cx)
                    .draft(cx)
                    .prompt
                    .is_empty()
            );
        });
        view.update_in(cx, |view, window, cx| {
            view.generation_screen
                .as_ref()
                .expect("video screen")
                .update(cx, |screen, cx| {
                    screen.set_draft(
                        GenerationDraft {
                            prompt: "orange circle".into(),
                            ..GenerationDraft::default()
                        },
                        window,
                        cx,
                    )
                });
        });
        view.update_in(cx, |view, window, cx| {
            view.set_mode(GenerationMode::Image, window, cx)
        });
        assert_eq!(
            view.read_with(cx, |view, cx| view
                .generation_screen
                .as_ref()
                .expect("image screen")
                .read(cx)
                .draft(cx)
                .prompt
                .to_string()),
            "a green landscape"
        );
    }

    #[gpui::test]
    fn generation_negative_draft_survives_model_and_mode_switches(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Image, cx);
        view.update_in(cx, |view, window, cx| {
            let mut supported = model("image");
            supported.id = "replicate-supported".into();
            supported.capabilities = json!({"hosted":true,"operations":["text_to_image"],"negative_prompt":{"supported":true}});
            let mut unsupported = supported.clone();
            unsupported.id = "replicate-no-negative".into();
            unsupported.capabilities = json!({"hosted":true,"operations":["text_to_image"],"negative_prompt":{"supported":false}});
            let mut video = model("video");
            video.id = "replicate-video".into();
            video.capabilities = json!({"hosted":true,"operations":["text_to_video"]});
            view.selected_model = Some(supported.id.clone());
            view.models = vec![supported, unsupported, video];
            view.generation_screen.as_ref().expect("image screen").update(cx, |screen, cx| {
                screen.set_draft(GenerationDraft {
                    prompt: "a landscape".into(),
                    negative: "blur and watermarks".into(),
                    ..GenerationDraft::default()
                }, window, cx)
            });
            let supported = view.models.iter().find(|model| model.id == "replicate-supported").expect("supported model");
            assert!(view.screen_model(supported, GenerationRecipe::TextImage, 0).supports_negative);

            view.selected_model = Some("replicate-no-negative".into());
            view.choose_default_model();
            assert_eq!(view.selected_model.as_deref(), Some("replicate-no-negative"));
            let unsupported = view.models.iter().find(|model| model.id == "replicate-no-negative").expect("unsupported model");
            assert!(!view.screen_model(unsupported, GenerationRecipe::TextImage, 0).supports_negative);

            view.set_mode(GenerationMode::Video, window, cx);
            assert_eq!(view.selected_model.as_deref(), Some("replicate-video"));

            view.set_mode(GenerationMode::Image, window, cx);
            let restored = view.generation_screen.as_ref().expect("restored image screen").read(cx).draft(cx);
            assert_eq!(restored.prompt.as_ref(), "a landscape");
            assert_eq!(restored.negative.as_ref(), "blur and watermarks");
        });
    }

    #[gpui::test]
    fn generation_source_clicks_map_to_the_visible_image(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Masks, cx);
        let image = image::DynamicImage::new_rgba8(512, 512);
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("fixture image");
        let preview = make_preview(png.get_ref(), "image/png").expect("source preview");
        view.update(cx, |view, cx| {
            view.set_source(
                SourceImage {
                    reference: json!({"asset_id":"fixture"}),
                    name: "Source".into(),
                    preview,
                },
                cx,
            );
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("generation-source-preview")
            .expect("source bounds");
        assert_eq!(
            bounds.size.height,
            px(200.),
            "the source must not shrink or crop"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.source_bounds),
            Some(bounds)
        );
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        view.read_with(cx, |view, _| {
            let point = view.points.first().expect("click should add a point");
            assert!((point.x - 256.).abs() < 1. && (point.y - 256.).abs() < 1.);
        });
        cx.simulate_click(
            bounds.center() + gpui::point(px(0.5), px(0.25)),
            gpui::Modifiers::none(),
        );
        view.read_with(cx, |view, _| {
            let point = view
                .points
                .last()
                .expect("off-grid click should add a point");
            assert_eq!((point.x, point.y), (257., 256.));
        });
    }

    #[gpui::test]
    fn generation_inpaint_controls_scroll_into_view_at_laptop_height(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = visual_workspace(GenerationMode::Image, cx);
        view.update_in(cx, |view, window, cx| {
            let mut model = model("image");
            model.capabilities =
                json!({"steps":{}, "guidance":{}, "negative_prompt":{"supported":true}});
            view.selected_model = Some(model.id.clone());
            view.models = vec![model];
            view.mask = Some(Arc::from([0u8]));
            let prompt = view.prompt.clone();
            prompt.read(cx).editor().clone().set_text(
                "one\ntwo\nthree\nfour\nfive\nsix",
                window,
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(320.), px(650.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1200.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        let submit = cx
            .debug_bounds("generation-submit")
            .expect("generate button after scrolling");
        assert!(
            submit.top() >= px(0.) && submit.bottom() <= px(768.),
            "Generate must be reachable: {submit:?}"
        );
        view.update_in(cx, |view, window, cx| {
            view.status = "Mask selected. White areas will be edited.".into();
            view.set_mode(GenerationMode::Vector, window, cx);
        });
        view.read_with(cx, |view, _| {
            assert!(view.mask.is_none());
            assert!(!view.status.contains("Mask selected"));
        });
    }

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
            &cx.executor(),
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
        let error = fetch_catalog(&client, "https://api.fantaisa.net", None, &cx.executor())
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
        let executor = cx.executor();
        let request = fetch_catalog(
            &client,
            "https://api.fantaisa.net",
            account.as_deref(),
            &executor,
        );
        futures::pin_mut!(request);
        assert!(futures::poll!(&mut request).is_pending());
        client.sign_out(&cx.to_async()).await;
        response_sender.send(()).expect("release response");
        let error = request.await.err().expect("discard old account response");
        assert!(error.to_string().contains("account changed"));
    }

    #[test]
    fn generation_response_seed_preserves_backend_text_and_numeric_compatibility() {
        for (seed, expected) in [
            (None, None),
            (Some(Value::Null), None),
            (Some(json!(42)), Some("42")),
            (Some(json!(u64::MAX)), Some("18446744073709551615")),
            (Some(json!("9007199254740993")), Some("9007199254740993")),
            (
                Some(json!("184467440737095516160001")),
                Some("184467440737095516160001"),
            ),
            (Some(json!("00042")), Some("00042")),
        ] {
            let mut value = json!({"id":"seed-contract", "status":"succeeded", "output":[]});
            if let Some(seed) = seed {
                value["seed"] = seed;
            }
            let response: GenerationResponse = serde_json::from_value(value).expect("backend seed");
            assert_eq!(
                response.seed.as_ref().map(ToString::to_string).as_deref(),
                expected
            );
        }
    }

    async fn assert_string_seed_generation_completes(pending: bool, cx: &mut gpui::TestAppContext) {
        let seed = "184467440737095516160001";
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("generated image");
        let completed = json!({
            "id":"seed-contract", "status":"succeeded", "model":"fanta-image-1",
            "seed":seed, "billed_credits":1,
            "output":[{"data_url":format!("data:image/png;base64,{}", STANDARD.encode(png.into_inner()))}],
        });
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = http_client::FakeHttpClient::create({
            let requests = requests.clone();
            move |request| {
                let completed = completed.clone();
                let requests = requests.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models":[]}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            requests.lock().expect("request log").push("submission");
                            if pending {
                                json!({"id":"seed-contract", "status":"processing"})
                            } else {
                                completed
                            }
                        }
                        (&Method::GET, "/v1/generations/seed-contract") => {
                            requests.lock().expect("request log").push("poll");
                            completed
                        }
                        route => panic!("Unexpected generation request: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Image, client, cx);
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "seed-contract".into(),
                    request: json!({"model":"fanta-image-1", "prompt":"A sunrise"}),
                    model: "fanta-image-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Image,
                },
                cx,
            )
        });
        cx.run_until_parked();
        if pending {
            assert!(view.read_with(cx, |view, _| view.pending && view.task.is_some()));
            cx.executor().advance_clock(Duration::from_secs(2));
            cx.run_until_parked();
        }
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(view.error.is_none());
            assert!(!view.pending);
            assert!(view.unresolved_submission.is_none());
            assert_eq!(view.outputs.len(), 1);
            assert_eq!(view.history.len(), 1);
            assert!(view.preview.is_some());
            assert!(
                view.status.contains(seed),
                "preserve the backend seed exactly: {}",
                view.status
            );
        });
        let expected = if pending {
            vec!["submission", "poll"]
        } else {
            vec!["submission"]
        };
        assert_eq!(*requests.lock().expect("request log"), expected);
    }

    #[gpui::test]
    async fn generation_completed_submission_accepts_backend_string_seed(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_string_seed_generation_completes(false, cx).await;
    }

    #[gpui::test]
    async fn generation_poll_accepts_backend_string_seed(cx: &mut gpui::TestAppContext) {
        assert_string_seed_generation_completes(true, cx).await;
    }

    struct PendingBody;

    async fn assert_generation_recovery_after_tab_close(
        accepted: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        let submissions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = http_client::FakeHttpClient::create({
            let submissions = submissions.clone();
            move |request| {
                let submissions = submissions.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models": []}),
                        (&Method::GET, "/v1/me") => json!({
                            "user": {"id": "93f29737-8b7f-41b5-9b53-ef6df21d76ce"},
                            "org": {"id": "c6c984b9-d716-4ef3-88a3-b599a9824748"},
                        }),
                        (&Method::POST, "/v1/generations") => {
                            let key = request.headers()["Idempotency-Key"].to_str()?.to_owned();
                            let body: Value = serde_json::from_slice(
                                &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                            )?;
                            submissions
                                .lock()
                                .expect("submission log")
                                .push((key, body));
                            if !accepted {
                                return stalled_response(false).await;
                            }
                            json!({"id": "accepted-before-close", "status": "processing"})
                        }
                        route => panic!("Restoring history must not start or poll jobs: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client.clone(), cx);
        let request = json!({"model": "fanta-video-1", "prompt": "A sunrise"});
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "recover-after-close".into(),
                    request: request.clone(),
                    model: "fanta-video-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account: account.clone(),
                    mode: GenerationMode::Video,
                },
                cx,
            );
        });
        cx.run_until_parked();
        if accepted {
            assert!(view.read_with(cx, |view, _| view.pending && view.history.len() == 1));
        } else {
            cx.executor().advance_clock(API_TIMEOUT);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.task.is_none() && view.unresolved_submission.is_some()
            }));
        }
        let previous = view.downgrade();
        let mut reopened_context = cx.cx.clone();
        cx.update(|window, _| window.remove_window());
        drop(view);
        reopened_context.run_until_parked();
        assert!(
            previous.upgrade().is_none(),
            "the original tab must be released"
        );
        let (reopened, cx) =
            visual_workspace_with_client(GenerationMode::Video, client, &mut reopened_context);
        reopened.read_with(cx, |view, _| {
            assert!(
                view.task.is_none(),
                "restore must require an explicit action"
            );
            if accepted {
                assert_eq!(
                    view.history.first().and_then(RunSummary::generation_id),
                    Some("accepted-before-close"),
                    "accepted jobs must remain available after closing their tab",
                );
            } else {
                let restored = view
                    .unresolved_submission
                    .as_ref()
                    .expect("unconfirmed requests must survive their tab");
                assert_eq!(restored.key, "recover-after-close");
                assert_eq!(restored.request, request);
                assert_eq!(restored.account, account);
            }
        });
        assert_eq!(submissions.lock().expect("submission log").len(), 1);
    }

    #[gpui::test]
    async fn generation_recovery_reopens_unconfirmed_submission_after_tab_close(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_recovery_after_tab_close(false, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_reopens_accepted_job_after_tab_close(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_recovery_after_tab_close(true, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_storage_failure_prevents_submission(
        cx: &mut gpui::TestAppContext,
    ) {
        let submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let submissions = submissions.clone();
            move |request| {
                let submissions = submissions.clone();
                async move {
                    let value = match request.uri().path() {
                        "/v1/models" => json!({"models": []}),
                        "/v1/me" => recovery_account_fixture(),
                        _ => {
                            submissions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            bail!("A request must not be sent without a saved recovery record")
                        }
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(value.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        let database = cx.update(|_, cx| db::kvp::KeyValueStore::global(cx));
        database
            .write(|connection| {
                connection
                    .exec(
                        "CREATE TRIGGER fail_generation_recovery BEFORE INSERT ON scoped_kv_store
             BEGIN SELECT RAISE(FAIL,'disk write failed'); END;",
                    )
                    .and_then(|mut statement| statement())
            })
            .await
            .expect("install storage failure");
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "must-save-first".into(),
                    request: json!({"model":"fanta-video-1", "prompt":"A sunrise"}),
                    model: "fanta-video-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Video,
                },
                cx,
            )
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(
                view.error.is_some(),
                "storage errors must reach the interface"
            );
            assert!(view.history.is_empty());
        });
        assert_eq!(submissions.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[gpui::test]
    async fn generation_recovery_uses_verified_identity_across_key_rotation(
        cx: &mut gpui::TestAppContext,
    ) {
        let submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let submissions = submissions.clone();
            move |request| {
                let submissions = submissions.clone();
                async move {
                    let value = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models": []}),
                        (&Method::GET, "/v1/me") => {
                            let mut profile = recovery_account_fixture();
                            if request
                                .headers()
                                .get("Authorization")
                                .and_then(|header| header.to_str().ok())
                                == Some("Bearer fnt_live_other_test")
                            {
                                profile["user"]["id"] =
                                    json!("d8e366fe-2f4d-4d4a-a516-3179b5c9b707");
                            }
                            profile
                        }
                        (&Method::POST, "/v1/generations") => {
                            submissions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            json!({"id":"original-account-job", "status":"processing"})
                        }
                        route => panic!("Restoring an account must not submit or poll: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(value.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client.clone(), cx);
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "original-account-request".into(),
                    request: json!({"model":"fanta-video-1", "prompt":"A sunrise"}),
                    model: "fanta-video-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Video,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.history.len() == 1));
        let mut context = cx.cx.clone();
        cx.update(|window, _| window.remove_window());
        drop(view);
        context.run_until_parked();
        for (token, expected_history) in [("fnt_live_other_test", 0), ("fnt_live_rotated_test", 1)]
        {
            client.sign_out(&context.to_async()).await;
            client.override_authenticate(move |_| {
                Task::ready(Ok(client::Credentials {
                    // Deliberately keep the legacy numeric ID unchanged: only /v1/me can isolate this account.
                    user_id: 1,
                    access_token: token.into(),
                }))
            });
            client
                .sign_in(false, &context.to_async())
                .await
                .expect("sign in with replacement key");
            let (view, cx) =
                visual_workspace_with_client(GenerationMode::Video, client.clone(), &mut context);
            view.read_with(cx, |view, _| {
                assert!(view.error.is_none());
                assert_eq!(view.history.len(), expected_history);
                if expected_history == 1 {
                    assert_eq!(
                        view.history.first().and_then(RunSummary::generation_id),
                        Some("original-account-job")
                    );
                }
            });
            cx.update(|window, _| window.remove_window());
            drop(view);
            cx.run_until_parked();
        }
        assert_eq!(submissions.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    async fn assert_generation_http_400_recovery(
        unreserved: bool,
        rejected_retry: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = http_client::FakeHttpClient::create({
            let requests = requests.clone();
            move |request| {
                let requests = requests.clone();
                async move {
                    let value = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models": []}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            let key = request.headers()["Idempotency-Key"].to_str()?.to_owned();
                            let body: Value = serde_json::from_slice(
                                &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                            )?;
                            let attempt = {
                                let mut requests = requests.lock().expect("request log");
                                requests.push((key, body));
                                requests.len()
                            };
                            if attempt == 1 || (rejected_retry && attempt == 2) {
                                let mut response = http_client::Response::builder().status(400);
                                if unreserved || attempt == 2 {
                                    response =
                                        response.header("x-fanta-generation-unreserved", "true");
                                }
                                return Ok(response.body(json!({"error": {"type": "invalid_request_error", "message": "Source is unavailable"}}).to_string().into())?);
                            }
                            json!({"id":"reserved-before-error", "status":"failed", "output":[], "error":"Source is unavailable", "idempotent_replay":true})
                        }
                        route => panic!("Unexpected recovery request: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(value.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client.clone(), cx);
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "recover-reserved-error".into(),
                    request: json!({"model":"fanta-video-1", "prompt":"A sunrise"}),
                    model: "fanta-video-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Video,
                },
                cx,
            )
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(view.error.is_some());
            assert_eq!(
                view.unresolved_submission.is_some(),
                !unreserved,
                "only an explicit unreserved response can discard the recovery key"
            );
        });
        let mut context = cx.cx.clone();
        cx.update(|window, _| window.remove_window());
        drop(view);
        context.run_until_parked();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, &mut context);
        assert_eq!(
            view.read_with(cx, |view, _| view.unresolved_submission.is_some()),
            !unreserved
        );
        if !unreserved {
            let submission = view.read_with(cx, |view, _| {
                view.unresolved_submission
                    .clone()
                    .expect("saved failed response")
            });
            view.update(cx, |view, cx| view.submit(submission.clone(), cx));
            cx.run_until_parked();
            if rejected_retry {
                assert!(
                    view.read_with(cx, |view, _| view.unresolved_submission.is_some()),
                    "a rejected retry cannot disprove acceptance of an earlier attempt"
                );
                view.update(cx, |view, cx| view.submit(submission, cx));
                cx.run_until_parked();
            }
            view.read_with(cx, |view, _| {
                assert!(view.unresolved_submission.is_none());
                assert_eq!(
                    view.history.first().and_then(RunSummary::generation_id),
                    Some("reserved-before-error")
                );
            });
        }
        let requests = requests.lock().expect("request log");
        assert_eq!(
            requests.len(),
            if unreserved {
                1
            } else if rejected_retry {
                3
            } else {
                2
            }
        );
        if !unreserved {
            assert_eq!(
                requests.first(),
                requests.get(1),
                "recovery must reuse the exact body and key"
            );
        }
    }

    #[gpui::test]
    async fn generation_recovery_http_400_keeps_unknown_reservation_after_reopen(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_http_400_recovery(false, false, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_http_400_clears_only_explicit_unreserved_request(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_http_400_recovery(true, false, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_http_400_unreserved_retry_keeps_earlier_uncertainty(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_http_400_recovery(false, true, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_accept_write_failure_reopens_exact_mask_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let journal_slot = Arc::new(std::sync::Mutex::new(None::<GenerationJournal>));
        let http = http_client::FakeHttpClient::create({
            let requests = requests.clone();
            let journal_slot = journal_slot.clone();
            move |request| {
                let requests = requests.clone();
                let journal_slot = journal_slot.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models": []}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            let key = request.headers()["Idempotency-Key"].to_str()?.to_owned();
                            let body: Value = serde_json::from_slice(
                                &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                            )?;
                            let journal = journal_slot
                                .lock()
                                .expect("journal slot")
                                .clone()
                                .expect("initialized journal");
                            let snapshot = journal.load().await?;
                            let record = snapshot.records.first().expect("saved before POST");
                            assert_eq!(record.key, key);
                            assert_eq!(record.request.as_ref(), Some(&body));
                            assert!(record.result.is_none());
                            requests
                                .lock()
                                .expect("request log")
                                .push(("POST", Some((key, body))));
                            json!({"id":"server-job-42", "status":"succeeded", "output":[{"svg":"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><rect width=\"4\" height=\"2\" fill=\"blue\"/></svg>"}]})
                        }
                        (&Method::GET, "/v1/generations/server-job-42") => {
                            requests.lock().expect("request log").push(("GET", None));
                            json!({"id":"server-job-42", "status":"succeeded", "output":[{"svg":"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><rect width=\"4\" height=\"2\" fill=\"blue\"/></svg>"}]})
                        }
                        route => panic!("Unexpected recovery request: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Image, client.clone(), cx);
        *journal_slot.lock().expect("journal slot") =
            view.read_with(cx, |view, _| view.journal.clone());
        let database = cx.update(|_, cx| db::kvp::KeyValueStore::global(cx));
        database
            .write(|connection| {
                connection.exec(
                    "CREATE TRIGGER fail_accepted_recovery BEFORE INSERT ON scoped_kv_store
                WHEN instr(NEW.value, 'server-job-42') > 0
                BEGIN SELECT RAISE(FAIL,'accepted record write failed'); END;",
                )?()
            })
            .await
            .expect("install acceptance failure");
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("source PNG");
        let png = png.into_inner();
        let mut preview = make_preview(&png, "image/png").expect("source preview");
        preview.width = 400;
        preview.height = 200;
        let source = SourceImage {
            reference: json!({"url":"https://media.example/original.png"}),
            name: "Original mask source".into(),
            preview,
        };
        let submission = Submission {
            key: "recover-accepted-write".into(),
            request: json!({"model":"fanta-image-1", "prompt":"Replace the sky", "seed":"9007199254740993",
                "input":{"operation":"inpaint", "image":source.reference.clone(),
                    "points":[{"x":123,"y":45,"positive":true}], "mask":{"data_url":"data:image/png;base64,AA=="}}}),
            model: "fanta-image-1".into(),
            prompt: "Replace the sky".into(),
            source: Some(source),
            account,
            mode: GenerationMode::Image,
        };
        let expected = submission.saved().expect("saved source");
        view.update(cx, |view, cx| view.submit(submission.clone(), cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(
                view.error
                    .as_ref()
                    .is_some_and(|error| error.contains("server-job-42"))
            );
            assert!(view.unresolved_submission.is_some());
            assert!(view.history.is_empty());
        });
        let previous = view.downgrade();
        let mut context = cx.cx.clone();
        cx.update(|window, _| window.remove_window());
        drop(view);
        context.run_until_parked();
        assert!(previous.upgrade().is_none());
        let (view, cx) = visual_workspace_with_client(GenerationMode::Image, client, &mut context);
        let recovered = view.read_with(cx, |view, _| {
            view.unresolved_submission
                .clone()
                .expect("saved unresolved request")
        });
        assert_eq!(recovered.saved().expect("restored source"), expected);
        assert_eq!(
            requests.lock().expect("request log").len(),
            1,
            "reopen cannot submit automatically"
        );
        database
            .write(|connection| connection.exec("DROP TRIGGER fail_accepted_recovery")?())
            .await
            .expect("restore storage");
        view.update(cx, |view, cx| view.submit(recovered, cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(
                view.error.is_none(),
                "recovered job error: {:?}",
                view.error
            );
            assert!(view.unresolved_submission.is_none());
            let run = view.history.first().expect("accepted history");
            assert_eq!(run.generation_id(), Some("server-job-42"));
            let source = run.source.as_ref().expect("accepted source");
            assert_eq!((source.preview.width, source.preview.height), (400, 200));
        });
        view.update(cx, |view, cx| view.submit(submission, cx));
        cx.run_until_parked();
        let requests = requests.lock().expect("request log");
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests.first(),
            requests.get(1),
            "recovery must reuse exact request bytes and key"
        );
        assert_eq!(
            requests.get(2).map(|request| request.0),
            Some("GET"),
            "a stale accepted submission must fetch its saved job instead of starting another request"
        );
    }

    async fn assert_generation_recovery_completed_vector(
        stale_after_sign_out: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24"><path d="M2 2L30 2L16 22Z" fill="#22c55e"/></svg>"##;
        let messages = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let messages = messages.clone();
            move |request| {
                let messages = messages.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models": [{
                            "id": "claude-sonnet-5", "kind": "chat", "max_output_tokens": 4096
                        }]}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/messages") => {
                            assert!(request.headers().get("Idempotency-Key").is_none());
                            let body: Value = serde_json::from_slice(
                                &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                            )?;
                            assert_eq!(body["model"], "claude-sonnet-5");
                            assert_eq!(body["messages"][0]["content"], "a green triangle");
                            assert_eq!(
                                messages.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                                0,
                                "restoring artwork must not repeat a metered Messages request"
                            );
                            json!({
                                "id": "msg_completed_recovery",
                                "stop_reason": "end_turn",
                                "content": [{"type": "text", "text": SVG}]
                            })
                        }
                        route => panic!(
                            "Vector restore/save must not create or poll a generation: {route:?}"
                        ),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Vector, client.clone(), cx);
        view.update_in(cx, |view, window, cx| {
            assert!(
                view.journal.is_some(),
                "verified account recovery must be ready"
            );
            assert_eq!(view.selected_model.as_deref(), Some("claude-sonnet-5"));
            let prompt = view.prompt.clone();
            prompt.update(cx, |prompt, cx| {
                prompt.set_text("a green triangle", window, cx)
            });
            view.generate(window, cx);
        });
        cx.run_until_parked();
        let run = view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(
                view.error.is_none(),
                "unexpected vector error: {:?}",
                view.error
            );
            assert!(view.preview.is_some());
            assert!(view.unresolved_submission.is_none());
            assert_eq!(view.history.len(), 1);
            let run = view
                .history
                .first()
                .expect("completed vector history")
                .clone();
            assert!(run.generation_id().is_none());
            assert_eq!(run.provenance()["message_id"], "msg_completed_recovery");
            run
        });
        let journal = view.read_with(cx, |view, _| view.journal.clone().expect("account journal"));
        let saved = journal.load().await.expect("durable completed vector");
        let record = saved.records.first().expect("one persisted vector");
        assert_eq!(saved.records.len(), 1);
        assert!(
            record.request.is_none(),
            "Messages has no safe submission replay"
        );
        assert!(matches!(
            &record.result,
            Some(SavedRunResult::VectorMessage { message_id, svg })
                if message_id.as_deref() == Some("msg_completed_recovery") && svg == SVG
        ));

        if stale_after_sign_out {
            client.sign_out(&cx.to_async()).await;
            cx.run_until_parked();
            view.read_with(cx, |view, _| {
                assert!(view.history.is_empty());
                assert!(view.journal.is_none());
                assert!(view.outputs.is_empty());
            });
            view.update(cx, |view, cx| view.check_status(run, cx));
            cx.run_until_parked();
            view.read_with(cx, |view, _| {
                assert!(
                    view.active_run.is_none(),
                    "a stale history callback must not restore the old account's run"
                );
                assert!(
                    view.outputs.is_empty(),
                    "signed-out history must not reveal old SVG bytes"
                );
                assert!(view.preview.is_none());
                assert!(view.preview_task.is_none());
                assert!(view.task.is_none());
                assert!(view.history.is_empty());
                assert!(view.journal.is_none());
            });
        } else {
            let previous = view.downgrade();
            let mut reopened_context = cx.cx.clone();
            cx.update(|window, _| window.remove_window());
            drop(view);
            reopened_context.run_until_parked();
            assert!(
                previous.upgrade().is_none(),
                "release the original vector tab"
            );
            let (reopened, cx) =
                visual_workspace_with_client(GenerationMode::Vector, client, &mut reopened_context);
            let restored = reopened.read_with(cx, |view, _| {
                assert!(view.task.is_none());
                assert!(view.error.is_none());
                assert!(view.unresolved_submission.is_none());
                assert!(
                    view.outputs.is_empty(),
                    "restore waits for an explicit history selection"
                );
                assert_eq!(view.history.len(), 1);
                view.history
                    .first()
                    .expect("reopened vector history")
                    .clone()
            });
            reopened.update(cx, |view, cx| view.check_status(restored, cx));
            cx.run_until_parked();
            reopened.read_with(cx, |view, _| {
                assert!(view.error.is_none());
                assert!(view.preview.is_some());
                assert!(view.task.is_none());
                assert_eq!(view.outputs.len(), 1);
                let output = view.outputs.first().expect("restored SVG output");
                assert_eq!(output.mime, "image/svg+xml");
                let MediaLocation::Inline(bytes) = &output.location else {
                    panic!("completed Messages artwork must restore its received bytes locally")
                };
                assert_eq!(bytes.as_ref(), SVG.as_bytes());
                assert!(
                    view.active_run
                        .as_ref()
                        .expect("restored run")
                        .generation_id()
                        .is_none()
                );
            });
            let directory = tempfile::tempdir().expect("restored vector output directory");
            let path = directory.path().join("restored.svg");
            reopened.update(cx, |view, cx| view.save_output(cx));
            assert!(cx.did_prompt_for_new_path());
            cx.simulate_new_path_selection(|_| Some(path.clone()));
            cx.run_until_parked();
            assert_eq!(
                std::fs::read(path).expect("saved restored SVG"),
                SVG.as_bytes()
            );
            reopened.read_with(cx, |view, _| {
                assert!(view.task.is_none());
                assert!(view.error.is_none());
                assert_eq!(view.status, "Result saved.");
            });
        }
        assert_eq!(messages.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[gpui::test]
    async fn generation_recovery_completed_vector_reopens_and_saves_without_resubmission(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_recovery_completed_vector(false, cx).await;
    }

    #[gpui::test]
    async fn generation_recovery_stale_vector_selection_after_sign_out_stays_cleared(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_generation_recovery_completed_vector(true, cx).await;
    }

    impl futures::AsyncRead for PendingBody {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buffer: &mut [u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }
    }

    async fn stalled_response(stall_body: bool) -> Result<http_client::Response<AsyncBody>> {
        if stall_body {
            Ok(http_client::Response::builder()
                .status(200)
                .body(AsyncBody::from_reader(
                    futures::io::Cursor::new(b"partial response".to_vec()).chain(PendingBody),
                ))?)
        } else {
            futures::future::pending().await
        }
    }

    async fn assert_submission_timeout_recovers(stall_body: bool, cx: &mut gpui::TestAppContext) {
        let submissions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = http_client::FakeHttpClient::create({
            let submissions = submissions.clone();
            move |request| {
                let submissions = submissions.clone();
                async move {
                    if request.uri().path() == "/v1/me" {
                        return Ok(http_client::Response::builder()
                            .status(200)
                            .body(recovery_account_fixture().to_string().into())?);
                    }
                    if request.uri().path() == "/v1/models" {
                        return Ok(http_client::Response::builder()
                            .status(200)
                            .body(r#"{"models":[]}"#.into())?);
                    }
                    if request.uri().path() == "/result.mp4" {
                        assert_eq!(request.method(), Method::GET);
                        assert!(request.headers().get("Authorization").is_none());
                        // Keep native decoding out of this deterministic submission-recovery test.
                        return stalled_response(false).await;
                    }
                    assert_eq!(request.method(), Method::POST);
                    assert_eq!(request.uri().path(), "/v1/generations");
                    let key = request.headers()["Idempotency-Key"].to_str()?.to_owned();
                    let body: Value = serde_json::from_slice(
                        &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                    )?;
                    let first_submission = {
                        let mut submissions = submissions.lock().expect("submission log");
                        submissions.push((key, body));
                        submissions.len() == 1
                    };
                    if first_submission {
                        return stalled_response(stall_body).await;
                    }
                    Ok(http_client::Response::builder().status(200).body(
                        json!({
                            "id":"accepted-before-timeout", "status":"succeeded",
                            "output":[{"url":"https://media.example/result.mp4", "mime":"video/mp4"}],
                            "billed_credits":1,
                            "seed":"9007199254740993", "idempotent_replay":true,
                        })
                        .to_string()
                        .into(),
                    )?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        let request = json!({"model":"fanta-video-1", "prompt":"A sunrise"});
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "original-submission-key".into(),
                    request: request.clone(),
                    model: "fanta-video-1".into(),
                    prompt: "A sunrise".into(),
                    source: None,
                    account: account.clone(),
                    mode: GenerationMode::Video,
                },
                cx,
            );
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(API_TIMEOUT - Duration::from_secs(1));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.task.is_some()));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        let retry = view.read_with(cx, |view, _| {
            assert!(view.task.is_none(), "the timeout must unlock recovery");
            assert!(
                view.error
                    .as_deref()
                    .is_some_and(|error| error.contains("timed out"))
            );
            assert!(view.history.is_empty());
            view.unresolved_submission
                .clone()
                .expect("retain the uncertain submission")
        });
        assert_eq!(retry.key, "original-submission-key");
        assert_eq!(retry.request, request);
        assert_eq!(retry.account, account);
        view.update(cx, |view, cx| view.submit(retry, cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(view.unresolved_submission.is_none());
            assert!(view.error.is_none());
            assert_eq!(view.history.len(), 1);
            assert_eq!(view.outputs.len(), 1);
            assert!(view.status.contains("Seed 9007199254740993"));
            assert_eq!(
                view.active_run.as_ref().and_then(RunSummary::generation_id),
                Some("accepted-before-timeout")
            );
        });
        let submissions = submissions.lock().expect("submission log");
        assert_eq!(submissions.len(), 2);
        assert_eq!(
            submissions[0], submissions[1],
            "retry must preserve the key and body"
        );
    }

    #[gpui::test]
    async fn generation_submission_header_timeout_preserves_same_key_retry(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_submission_timeout_recovers(false, cx).await;
    }

    #[gpui::test]
    async fn generation_submission_body_timeout_preserves_same_key_retry(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_submission_timeout_recovers(true, cx).await;
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn generation_inline_video_result_change_and_removal_close_retained_player(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        for remove_item in [false, true] {
            let player = view.update(cx, |view, cx| {
                let player = crate::video_playback::fake_playback(cx);
                view.playback = Some(player.clone());
                cx.notify();
                player
            });
            cx.run_until_parked();
            player.update_in(cx, |player, window, cx| player.tick(window, cx));
            let revision = player.read_with(cx, |player, _| player.frame_revision());
            assert!(player.read_with(cx, |player, _| player.frame().is_some()));
            view.update(cx, |view, cx| {
                if remove_item {
                    Item::on_removed(view, cx);
                } else {
                    view.outputs.clear();
                    view.load_preview(cx);
                }
            });
            cx.run_until_parked();
            player.read_with(cx, |player, _| {
                assert!(
                    player.frame().is_none(),
                    "an old rendered entity must release video pixels immediately"
                );
                assert!(player.frame_revision() > revision);
                assert_eq!(
                    player.status().state,
                    media::video::VideoPlaybackState::Paused
                );
            });
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn generation_inline_video_readded_tab_recreates_player_from_cached_bytes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        let project = project::Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let workspace =
            cx.update(|window, cx| cx.new(|cx| Workspace::test_new(project, window, cx)));
        let expected: Arc<[u8]> = Arc::from(&b"already downloaded video bytes"[..]);
        let creations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let original_item_id = view.entity_id();
        view.update(cx, |view, cx| {
            view.prepared_video = Some(Arc::new(generation_media::PreparedVideo {
                bytes: expected.clone(),
                asset: fanta_format::asset_id_for_bytes(&expected),
                metadata: generation_media::VideoMetadata {
                    width: 320,
                    height: 180,
                    duration_us: 10_000_000,
                },
                poster: None,
            }));
            let expected = expected.clone();
            let creations = creations.clone();
            view.playback_factory = Some(Box::new(move |bytes, cx| {
                assert!(
                    Arc::ptr_eq(&bytes, &expected),
                    "reuse the exact cached bytes without downloading"
                );
                creations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                crate::video_playback::fake_playback(cx)
            }));
            view.restore_video_playback(cx);
            cx.notify();
        });
        cx.run_until_parked();
        let original = view.read_with(cx, |view, _| view.playback.clone().expect("first player"));
        original.update_in(cx, |player, window, cx| player.tick(window, cx));
        assert!(original.read_with(cx, |player, _| player.frame().is_some()));
        view.update_in(cx, |view, window, cx| {
            Item::deactivated(view, window, cx);
            Item::on_removed(view, cx);
        });
        assert!(original.read_with(cx, |player, _| player.frame().is_none()));
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                view.update(cx, |view, cx| {
                    Item::added_to_workspace(view, workspace, window, cx);
                    cx.notify();
                });
            })
        });
        cx.run_until_parked();
        assert_eq!(view.entity_id(), original_item_id);
        assert_eq!(creations.load(std::sync::atomic::Ordering::SeqCst), 2);
        let replacement = view.read_with(cx, |view, _| {
            view.playback.clone().expect("replacement player")
        });
        assert_ne!(replacement.entity_id(), original.entity_id());
        replacement.update_in(cx, |player, window, cx| {
            player.play(cx);
            player.tick(window, cx);
        });
        assert_eq!(
            replacement.read_with(cx, |player, _| player.status().state),
            media::video::VideoPlaybackState::Playing
        );
        assert!(replacement.read_with(cx, |player, _| player.frame().is_some()));
        assert!(Arc::ptr_eq(
            &view.read_with(cx, |view, _| view
                .prepared_video
                .as_ref()
                .expect("cache")
                .bytes
                .clone()),
            &expected
        ));
        view.update(cx, |view, cx| view.clear_playback(cx));
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn generation_inline_video_signout_closes_retained_player(cx: &mut gpui::TestAppContext) {
        let http = http_client::FakeHttpClient::create(|request| async move {
            let body = match request.uri().path() {
                "/v1/models" => json!({"models":[]}),
                "/v1/me" => recovery_account_fixture(),
                route => panic!("inline player must not download again: {route}"),
            };
            Ok(http_client::Response::builder()
                .status(200)
                .body(body.to_string().into())?)
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client.clone(), cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let player = view.update(cx, |view, cx| {
            let player = crate::video_playback::fake_playback(cx);
            view.playback = Some(player.clone());
            cx.notify();
            player
        });
        cx.run_until_parked();
        player.update_in(cx, |player, window, cx| player.tick(window, cx));
        assert!(player.read_with(cx, |player, _| player.frame().is_some()));
        client.sign_out(&cx.to_async()).await;
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.playback.is_none()));
        assert!(player.read_with(cx, |player, _| player.frame().is_none()));
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn generation_inline_video_tab_and_workspace_deactivation_pause(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let player = view.update(cx, |view, cx| {
            let player = crate::video_playback::fake_playback(cx);
            view.playback = Some(player.clone());
            cx.notify();
            player
        });
        cx.run_until_parked();
        for workspace in [false, true] {
            view.update(cx, |view, cx| view.set_playback_active(true, cx));
            player.update_in(cx, |player, window, cx| {
                player.play(cx);
                player.tick(window, cx);
            });
            assert_eq!(
                player.read_with(cx, |player, _| player.status().state),
                media::video::VideoPlaybackState::Playing
            );
            view.update_in(cx, |view, window, cx| {
                if workspace {
                    Item::workspace_deactivated(view, window, cx);
                } else {
                    Item::deactivated(view, window, cx);
                }
            });
            assert_eq!(
                player.read_with(cx, |player, _| player.status().state),
                media::video::VideoPlaybackState::Paused
            );
        }
        view.update(cx, |view, cx| view.clear_playback(cx));
    }

    #[gpui::test]
    async fn generation_video_preview_switch_cancels_download(cx: &mut gpui::TestAppContext) {
        struct CancelWatch(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for CancelWatch {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let http = http_client::FakeHttpClient::create({
            let cancelled = cancelled.clone();
            let started = started.clone();
            move |request| {
                let cancelled = cancelled.clone();
                let started = started.clone();
                async move {
                    let body = match request.uri().path() {
                        "/v1/models" => json!({"models":[]}),
                        "/v1/me" => recovery_account_fixture(),
                        "/pending.mp4" => {
                            assert!(request.headers().get("Authorization").is_none());
                            started.store(true, std::sync::atomic::Ordering::SeqCst);
                            let _watch = CancelWatch(cancelled);
                            return futures::future::pending().await;
                        }
                        route => panic!("unexpected preview request: {route}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(body.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        view.update(cx, |view, cx| {
            view.outputs = vec![MediaOutput {
                label: "Video".into(),
                mime: "video/mp4".into(),
                mask: false,
                location: MediaLocation::Url("https://media.example/pending.mp4".into()),
            }];
            view.load_preview(cx);
        });
        cx.run_until_parked();
        assert!(started.load(std::sync::atomic::Ordering::SeqCst));
        assert!(view.read_with(cx, |view, _| view.preview_task.is_some()));
        view.update(cx, |view, cx| {
            let mut png = Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(32, 24)
                .write_to(&mut png, image::ImageFormat::Png)
                .expect("replacement image");
            view.outputs = vec![MediaOutput {
                label: "Image".into(),
                mime: "image/png".into(),
                mask: false,
                location: MediaLocation::Inline(png.into_inner().into()),
            }];
            view.load_preview(cx);
        });
        cx.run_until_parked();
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
        view.read_with(cx, |view, _| {
            let preview = view.preview.as_ref().expect("new image preview");
            assert_eq!((preview.width, preview.height), (32, 24));
            assert!(view.prepared_video.is_none());
            assert!(view.preview_task.is_none());
            assert!(view.error.is_none());
        });
        cx.executor().advance_clock(MEDIA_TRANSFER_TIMEOUT);
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.error.is_none()),
            "cancelled video must not overwrite the next result"
        );
    }

    async fn assert_pending_save_cannot_cross_sign_out(
        retain_save_task: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        let http = http_client::FakeHttpClient::create(|request| async move {
            let body = match request.uri().path() {
                "/v1/models" => json!({"models":[]}),
                "/v1/me" => recovery_account_fixture(),
                route => panic!("an inline result must not make media requests: {route}"),
            };
            Ok(http_client::Response::builder()
                .status(200)
                .body(body.to_string().into())?)
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Vector, client.clone(), cx);
        let directory = tempfile::tempdir().expect("save directory");
        let path = directory.path().join("existing.svg");
        let existing = b"existing customer file";
        std::fs::write(&path, existing).expect("existing destination");
        let retained_task = view.update(cx, |view, cx| {
            view.outputs = vec![vector_output(Arc::from(
                &br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24"><path d="M0 0H32V24H0Z" fill="#22c55e"/></svg>"##[..],
            ))];
            view.save_output(cx);
            assert!(view.task.is_some(), "the save must be pending on the dialog");
            if retain_save_task {
                // Keep the future alive so this case verifies the account guard,
                // independently of the account observer dropping the task handle.
                Some(view.task.take().expect("pending save task"))
            } else {
                None
            }
        });
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        assert_eq!(
            std::fs::read(&path).expect("destination before reply"),
            existing
        );

        client.sign_out(&cx.to_async()).await;
        cx.run_until_parked();
        assert!(client.account_access_token().is_none());
        view.read_with(cx, |view, _| {
            assert!(view.account.is_none());
            assert!(view.outputs.is_empty());
            assert!(view.task.is_none());
        });
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        if let Some(task) = retained_task {
            task.await;
        }
        cx.run_until_parked();
        assert_eq!(
            std::fs::read(&path).expect("destination after old dialog reply"),
            existing,
            "answering an old account's save dialog must not overwrite a file"
        );
        view.read_with(cx, |view, _| {
            assert!(view.outputs.is_empty());
            assert!(view.task.is_none());
            assert!(view.error.is_none());
            assert_ne!(view.status.as_ref(), "Result saved.");
        });
    }

    #[gpui::test]
    async fn generation_save_dialog_sign_out_cancels_old_account_result(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_pending_save_cannot_cross_sign_out(false, cx).await;
    }

    #[gpui::test]
    async fn generation_save_dialog_sign_out_blocks_retained_task(cx: &mut gpui::TestAppContext) {
        assert_pending_save_cannot_cross_sign_out(true, cx).await;
    }

    #[gpui::test]
    async fn generation_video_save_reuses_preview_bytes(cx: &mut gpui::TestAppContext) {
        let http = http_client::FakeHttpClient::create(|request| async move {
            let body = match request.uri().path() {
                "/v1/models" => json!({"models":[]}),
                "/v1/me" => recovery_account_fixture(),
                route => panic!("cached video must not be downloaded again: {route}"),
            };
            Ok(http_client::Response::builder()
                .status(200)
                .body(body.to_string().into())?)
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        let directory = tempfile::tempdir().expect("save directory");
        let path = directory.path().join("cached.mp4");
        let expected: Arc<[u8]> = Arc::from(&b"exact previously downloaded MP4 bytes"[..]);
        view.update(cx, |view, cx| {
            view.outputs = vec![MediaOutput {
                label: "Video".into(),
                mime: "video/mp4".into(),
                mask: false,
                location: MediaLocation::Url("https://media.example/expired.mp4".into()),
            }];
            view.prepared_video = Some(Arc::new(generation_media::PreparedVideo {
                bytes: expected.clone(),
                asset: fanta_format::asset_id_for_bytes(&expected),
                metadata: generation_media::VideoMetadata {
                    width: 320,
                    height: 180,
                    duration_us: 1_000_000,
                },
                poster: None,
            }));
            view.save_output(cx);
        });
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read(path).expect("saved video"), expected.as_ref());
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(view.error.is_none());
            assert_eq!(view.status.as_ref(), "Result saved.");
        });
    }

    async fn assert_download_timeout_preserves_result(
        stall_body: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        let downloads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let downloads = downloads.clone();
            move |request| {
                let downloads = downloads.clone();
                async move {
                    if request.uri().path() == "/v1/me" {
                        return Ok(http_client::Response::builder()
                            .status(200)
                            .body(recovery_account_fixture().to_string().into())?);
                    }
                    if request.uri().path() == "/v1/models" {
                        return Ok(http_client::Response::builder()
                            .status(200)
                            .body(r#"{"models":[]}"#.into())?);
                    }
                    assert_eq!(request.uri().path(), "/result.mp4");
                    assert!(request.headers().get("Authorization").is_none());
                    if downloads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        return stalled_response(stall_body).await;
                    }
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body("saved media bytes".into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        let directory = tempfile::tempdir().expect("output directory");
        let path = directory.path().join("result.mp4");
        std::fs::write(&path, "existing output").expect("existing output file");
        view.update(cx, |view, cx| {
            let run = RunSummary {
                result: RunResult::Generation {
                    id: "download-job".into(),
                },
                model: "fanta-video-1".into(),
                prompt: "A sunrise".into(),
                source: None,
                mode: GenerationMode::Video,
            };
            view.active_run = Some(run.clone());
            view.history = vec![run];
            view.outputs = vec![MediaOutput {
                label: "Video".into(),
                mime: "video/mp4".into(),
                location: MediaLocation::Url("https://media.example/result.mp4".into()),
                mask: false,
            }];
            view.save_output(cx);
        });
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        cx.executor().advance_clock(MEDIA_TRANSFER_TIMEOUT);
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            assert!(
                view.error
                    .as_deref()
                    .is_some_and(|error| error.contains("download timed out"))
            );
            assert_eq!(
                view.active_run.as_ref().and_then(RunSummary::generation_id),
                Some("download-job")
            );
            assert_eq!(view.history.len(), 1);
            assert_eq!(view.outputs.len(), 1);
        });
        assert_eq!(
            std::fs::read(&path).expect("untouched file"),
            b"existing output"
        );
        view.update(cx, |view, cx| view.save_output(cx));
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        assert_eq!(
            std::fs::read(path).expect("saved file"),
            b"saved media bytes"
        );
        assert_eq!(downloads.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(view.read_with(cx, |view, _| view.task.is_none()
            && view.error.is_none()
            && view.status == "Result saved."));
    }

    #[gpui::test]
    async fn generation_download_header_timeout_preserves_save_retry(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_download_timeout_preserves_result(false, cx).await;
    }

    #[gpui::test]
    async fn generation_download_body_timeout_preserves_save_retry(cx: &mut gpui::TestAppContext) {
        assert_download_timeout_preserves_result(true, cx).await;
    }

    async fn assert_upload_timeout_does_not_complete(
        stall_body: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = http_client::FakeHttpClient::create({
            let requests = requests.clone();
            move |request| {
                requests
                    .lock()
                    .expect("request log")
                    .push(request.uri().path().to_owned());
                async move {
                    match request.uri().path() {
                        "/v1/assets/uploads" => Ok(http_client::Response::builder().status(200).body(
                            r#"{"asset_id":"pending-upload","upload_url":"https://media.example/upload"}"#.into(),
                        )?),
                        "/upload" => {
                            assert_eq!(request.method(), Method::PUT);
                            assert!(request.headers().get("Authorization").is_none());
                            stalled_response(stall_body).await
                        }
                        path => panic!("An unconfirmed upload must not be completed: {path}"),
                    }
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("source image");
        let preview = make_preview(png.get_ref(), "image/png").expect("preview");
        let executor = cx.executor();
        let upload = upload_image(
            &client,
            "https://api.fantaisa.net",
            png.into_inner(),
            "source.png",
            "image/png",
            &preview,
            &executor,
        );
        futures::pin_mut!(upload);
        assert!(futures::poll!(&mut upload).is_pending());
        executor.advance_clock(MEDIA_TRANSFER_TIMEOUT);
        let error = upload.await.err().expect("stalled upload must time out");
        assert!(error.to_string().contains("upload timed out"));
        assert_eq!(
            *requests.lock().expect("request log"),
            ["/v1/assets/uploads", "/upload"]
        );
    }

    #[gpui::test]
    async fn generation_upload_header_timeout_does_not_complete_asset(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_upload_timeout_does_not_complete(false, cx).await;
    }

    #[gpui::test]
    async fn generation_upload_body_timeout_does_not_complete_asset(cx: &mut gpui::TestAppContext) {
        assert_upload_timeout_does_not_complete(true, cx).await;
    }

    fn model(kind: &str) -> GenerationModel {
        GenerationModel {
            id: format!("fanta-{kind}-1"),
            kind: kind.into(),
            display_name: None,
            credits_per_output: None,
            max_output_tokens: None,
            capabilities: json!({"operations":["inpaint"],"steps":{"min":10,"max":50}}),
            pricing: Value::Null,
        }
    }

    fn screen_submission(
        model: &GenerationModel,
        recipe: GenerationRecipe,
        prompt: &str,
    ) -> GenerationSubmission {
        GenerationSubmission {
            kind: recipe.kind(),
            recipe,
            model_id: model.id.clone().into(),
            prompt: prompt.to_owned().into(),
            negative: None,
            seed: None,
            source_id: None,
            end_frame_id: None,
            voice_reference_id: None,
            voice_consent_granted: false,
            options: Vec::new(),
        }
    }

    #[test]
    fn generation_catalog_shows_verified_hosted_models_only() {
        let mut model = model("image");
        assert!(!model.hosted_creation());
        model.capabilities = json!({});
        model.id = "flux-schnell".into();
        assert!(model.hosted_creation());
        model.kind = "chat".into();
        model.id = "claude-sonnet-5".into();
        assert!(model.hosted_creation());
        model.kind = "image".into();
        model.id = "replicate-seedream-5-lite".into();
        model.capabilities = json!({"hosted":true});
        assert!(model.hosted_creation());
        model.id = "seedream-5-lite".into();
        model.capabilities =
            json!({"hosted":true,"operations":["text_to_image"],"provider":"AI Gateway"});
        assert!(model.hosted_creation());
        model.id = "flux-fill".into();
        model.capabilities = json!({"hosted":true,"operations":["inpaint"]});
        assert!(!model.hosted_creation());
        model.id = "fanta-image-1".into();
        model.capabilities = json!({"gateway":{"duration":4}});
        assert!(!model.hosted_creation());
    }

    #[gpui::test]
    fn existing_gateway_images_and_videos_remain_available(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Image, cx);
        let mut image = model("image");
        image.id = "recraft-v4.1-flash".into();
        image.capabilities = json!({
            "hosted": true,
            "provider": "AI Gateway",
            "operations": [],
            "gateway": {},
            "size": {"values": ["1024x1024", "1280x768", "768x1280"], "default": "1024x1024"},
            "num_outputs": {"min": 1, "max": 4, "default": 1}
        });
        assert!(image.hosted_creation());
        assert_eq!(image.recipes(), vec![GenerationRecipe::TextImage]);
        let groups = view.read_with(cx, |view, _| {
            view.screen_option_groups(&image, GenerationRecipe::TextImage)
        });
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key.as_ref(), "canvas");
        assert_eq!(groups[0].choices.len(), 3);
        assert_eq!(groups[1].key.as_ref(), "num_outputs");
        assert_eq!(groups[1].choices.len(), 4);
        let mut submission = screen_submission(&image, GenerationRecipe::TextImage, "A fern");
        submission.options = vec![
            fanta_gpui::generation::GenerationOptionSelection {
                key: "canvas".into(),
                value: "1280x768".into(),
            },
            fanta_gpui::generation::GenerationOptionSelection {
                key: "num_outputs".into(),
                value: "2".into(),
            },
        ];
        let request = build_screen_request(&image, &submission, None, None, None)
            .expect("legacy Gateway image request");
        assert_eq!(request["width"], 1280);
        assert_eq!(request["height"], 768);
        assert_eq!(request["input"]["num_outputs"], 2);

        let mut video = model("video");
        video.id = "veo-3.1-fast".into();
        video.capabilities = json!({
            "operations": [],
            "gateway": {"duration": 4, "resolution": "1280x720", "generateAudio": false}
        });
        assert!(video.hosted_creation());
        assert_eq!(video.recipes(), vec![GenerationRecipe::TextVideo]);
        let screen = view.read_with(cx, |view, _| {
            view.screen_model(&video, GenerationRecipe::TextVideo, 0)
        });
        assert!(screen.description.contains("4s"));
        assert!(screen.option_groups.is_empty());

        video.id = "kling-2.6-i2v".into();
        video.capabilities = json!({
            "operations": [],
            "gateway": {"duration": 5, "generateAudio": false, "requires_image": true}
        });
        assert!(video.hosted_creation());
        assert_eq!(video.recipes(), vec![GenerationRecipe::ImageVideo]);
    }

    #[gpui::test]
    fn creation_gallery_excludes_historical_mask_runs(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Image, cx);
        view.update_in(cx, |view, _, cx| {
            let mask_run = RunSummary {
                result: RunResult::Generation {
                    id: "old-mask".into(),
                },
                model: "fanta-segment-1".into(),
                prompt: "Select the subject".into(),
                source: None,
                mode: GenerationMode::Masks,
            };
            let image_run = RunSummary {
                result: RunResult::Generation {
                    id: "saved-image".into(),
                },
                model: "recraft-v4.1-flash".into(),
                prompt: "A fern".into(),
                source: None,
                mode: GenerationMode::Image,
            };
            view.history = vec![mask_run.clone(), image_run];
            view.active_run = Some(mask_run);
            view.outputs = vec![MediaOutput {
                label: "Mask".into(),
                mime: "image/png".into(),
                location: MediaLocation::Inline(Arc::from(&b"mask"[..])),
                mask: true,
            }];
            let data = view.generation_view_data(cx);
            assert_eq!(data.outputs.len(), 1);
            assert_eq!(data.outputs[0].id.as_ref(), "gen:saved-image:0");
            assert!(data.selected_output_id.is_none());
        });
    }

    #[gpui::test]
    fn generation_controls_follow_model_capabilities(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        let mut model = model("video");
        model.id = "wan-3-video".into();
        model.display_name = Some("Wan 3 Video".into());
        model.capabilities = json!({
            "gateway": {"model": "wan/wan-3"},
            "operations": ["text_to_video", "image_to_video"],
            "controls": {
                "duration": {"values": [4, 8], "default": 4, "request_field": "duration", "location": "input"},
                "resolution": {"values": ["720p", "1080p"], "default": "720p", "request_field": "resolution", "location": "input"},
                "generate_audio": {"values": [true, false], "default": true, "request_field": "generate_audio", "location": "input"},
                "camera_motion": {"values": ["static", "dolly_in"], "default": "static", "request_field": "camera_motion", "location": "input"}
            }
        });
        let groups = view.read_with(cx, |view, _| {
            view.screen_option_groups(&model, GenerationRecipe::ImageVideo)
        });
        assert_eq!(model.label(), "Wan 3 Video");
        assert!(model.hosted_creation());
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].key.as_ref(), "duration");
        assert_eq!(groups[0].choices[0].label.as_ref(), "4 s");
        assert!(!groups[0].advanced);
        assert_eq!(groups[1].key.as_ref(), "resolution");
        assert!(!groups[1].advanced);
        assert!(
            groups
                .iter()
                .find(|group| group.key == "camera_motion")
                .expect("camera motion")
                .advanced
        );
        assert!(
            groups
                .iter()
                .find(|group| group.key == "generate_audio")
                .expect("audio switch")
                .advanced
        );
    }

    #[test]
    fn gateway_control_values_preserve_provider_input_types() {
        let mut model = model("image");
        model.id = "gateway-image".into();
        model.capabilities = json!({
            "gateway": {"model": "example/image"},
            "operations": ["text_to_image"],
            "controls": {
                "size": {"values": ["1024x1024", "1536x1024"], "default": "1024x1024", "request_field": "size", "location": "input"},
                "quality": {"values": [1, 2], "default": 1, "request_field": "quality", "location": "input"},
                "transparent": {"values": [true, false], "default": false, "request_field": "transparent", "location": "input"}
            }
        });
        let mut submission = screen_submission(&model, GenerationRecipe::TextImage, "A fox icon");
        for (key, value) in [
            ("size", "1536x1024"),
            ("quality", "2"),
            ("transparent", "true"),
        ] {
            submission
                .options
                .push(fanta_gpui::generation::GenerationOptionSelection {
                    key: key.into(),
                    value: value.into(),
                });
        }
        let request = build_screen_request(&model, &submission, None, None, None)
            .expect("valid catalog controls");
        assert_eq!(request["kind"], "image");
        assert_eq!(request["input"]["size"], "1536x1024");
        assert_eq!(request["input"]["quality"], 2);
        assert_eq!(request["input"]["transparent"], true);
        assert!(request.get("width").is_none());

        submission.options[1].value = "3".into();
        assert!(build_screen_request(&model, &submission, None, None, None).is_err());
    }

    #[gpui::test]
    fn generation_cost_hint_tracks_gateway_paid_variants(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        let mut model = model("video");
        model.id = "gateway-kling-video".into();
        model.capabilities = json!({
            "gateway": {"model": "kling/video"},
            "operations": ["text_to_video"],
            "controls": {
                "duration": {"values": [5, 10], "default": 5, "request_field": "duration", "location": "input"},
                "mode": {"values": ["std", "pro"], "default": "std", "request_field": "mode", "location": "input"},
                "generate_audio": {"values": [true, false], "default": false, "request_field": "generateAudio", "location": "input"}
            }
        });
        model.pricing = json!({
            "unit": "second",
            "credits_per_unit": 23.52,
            "variant_fields": ["mode", "generateAudio"],
            "variant_credits_per_unit": {
                "std|false": 23.52,
                "std|true": 35.28,
                "pro|false": 31.36,
                "pro|true": 47.04
            }
        });
        view.update_in(cx, |view, _, _| {
            assert_eq!(view.selected_credit_rate(&model, 23.52), 23.52);
            view.screen_options
                .insert((model.id.clone(), "mode".into()), "pro".into());
            view.screen_options
                .insert((model.id.clone(), "generate_audio".into()), "true".into());
            assert_eq!(view.selected_credit_rate(&model, 23.52), 47.04);
            let screen = view.screen_model(&model, GenerationRecipe::TextVideo, 0);
            assert_eq!(screen.credit_hint.as_deref(), Some("Est. 236 credits · 5s"));
        });
    }

    #[gpui::test]
    fn gateway_resolution_choices_follow_selected_mode(cx: &mut gpui::TestAppContext) {
        let (view, cx) = visual_workspace(GenerationMode::Video, cx);
        let mut model = model("video");
        model.id = "gateway-kling-3-t2v".into();
        model.capabilities = json!({
            "gateway": {"model": "kling/video"},
            "operations": ["text_to_video"],
            "controls": {
                "mode": {"values": ["std", "pro"], "default": "std", "request_field": "mode", "location": "input"},
                "resolution": {"values": ["1280x720", "1920x1080"], "default": "1280x720", "request_field": "resolution", "location": "input"}
            },
            "constraints": {
                "mode_resolution": {
                    "std": ["1280x720"],
                    "pro": ["1280x720", "1920x1080"]
                }
            }
        });
        view.update_in(cx, |view, _, _| {
            let standard = view.screen_option_groups(&model, GenerationRecipe::TextVideo);
            let resolution = standard
                .iter()
                .find(|group| group.key == "resolution")
                .expect("resolution group");
            assert_eq!(resolution.choices.len(), 1);
            assert_eq!(resolution.selected.as_ref(), "1280x720");

            view.screen_options
                .insert((model.id.clone(), "mode".into()), "pro".into());
            let pro = view.screen_option_groups(&model, GenerationRecipe::TextVideo);
            let resolution = pro
                .iter()
                .find(|group| group.key == "resolution")
                .expect("resolution group");
            assert_eq!(resolution.choices.len(), 2);
        });

        let mut submission = screen_submission(&model, GenerationRecipe::TextVideo, "Ocean mist");
        submission.options = vec![
            fanta_gpui::generation::GenerationOptionSelection {
                key: "mode".into(),
                value: "std".into(),
            },
            fanta_gpui::generation::GenerationOptionSelection {
                key: "resolution".into(),
                value: "1920x1080".into(),
            },
        ];
        assert!(build_screen_request(&model, &submission, None, None, None).is_err());
        submission.options[0].value = "pro".into();
        assert!(build_screen_request(&model, &submission, None, None, None).is_ok());
    }

    #[test]
    fn hosted_speech_preserves_script_and_uses_model_prompt_limit() {
        let mut model = model("audio");
        model.id = "replicate-speech-2.8-hd".into();
        model.capabilities = json!({"hosted":true,"prompt_limit":10000,"operations":["speech"]});
        let script = "  Pause after this sentence.\nThen continue.  ";
        let request = build_screen_request(
            &model,
            &screen_submission(&model, GenerationRecipe::Speech, script),
            None,
            None,
            None,
        )
        .expect("valid speech request");
        assert_eq!(request["prompt"], script);
        assert!(
            build_screen_request(
                &model,
                &screen_submission(&model, GenerationRecipe::Speech, &"word ".repeat(2_001)),
                None,
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn hosted_video_operation_and_controls_map_to_backend_input() {
        let mut model = model("video");
        model.id = "replicate-ltx-2.3-pro".into();
        model.capabilities = json!({
            "hosted":true,
            "operations":["text_to_video","image_to_video"],
            "controls":{"duration":{"request_field":"duration","location":"input"}}
        });
        let mut submission = screen_submission(
            &model,
            GenerationRecipe::TextVideo,
            "A slow orbit around a sculpture",
        );
        submission
            .options
            .push(fanta_gpui::generation::GenerationOptionSelection {
                key: "duration".into(),
                value: "6".into(),
            });
        let request = build_screen_request(&model, &submission, None, None, None)
            .expect("valid hosted video request");
        assert_eq!(request["input"]["operation"], "text_to_video");
        assert_eq!(request["input"]["duration"], 6.0);
        assert!(request["input"].get("frames").is_none());
        assert!(request["input"].get("fps").is_none());
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
            mode: GenerationMode::Vector,
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
            json!({"source":{"asset_id":"source"},"text":"person","points":[{"x":120,"y":80,"positive":false}]})
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
    fn negative_prompt_requires_explicit_model_support() {
        for (capabilities, negative, expected) in [
            (
                json!({"negative_prompt":{"supported":true}}),
                "  blur  ",
                Some("blur"),
            ),
            (json!({"negative_prompt":{"supported":true}}), "   ", None),
            (json!({"negative_prompt":{"supported":false}}), "blur", None),
            (json!({}), "blur", None),
            (
                json!({"negative_prompt":{"supported":"true"}}),
                "blur",
                None,
            ),
        ] {
            let mut model = model("image");
            model.capabilities = capabilities;
            let request = build_request(
                &model,
                "a landscape",
                negative,
                "1024x1024",
                "",
                "",
                "",
                "",
                "",
                None,
                None,
                &[],
            )
            .expect("valid request");
            assert_eq!(
                request.get("negative").and_then(Value::as_str),
                expected,
                "capabilities: {}",
                model.capabilities
            );
        }
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
    fn generation_mask_points_use_integer_source_pixels() {
        let (x, y) = image_point(100.5, 100.25, 200., 200., 512, 512).expect("off-grid click");
        assert_eq!((x, y), (257., 256.));
        let (last_x, last_y) =
            image_point(199.99, 199.99, 200., 200., 512, 512).expect("last pixel");
        assert_eq!((last_x, last_y), (511., 511.));
        assert_eq!(image_point(-0.01, 100., 200., 200., 512, 512), None);
        assert_eq!(image_point(100., -0.01, 200., 200., 512, 512), None);
        assert_eq!(image_point(200., 100., 200., 200., 512, 512), None);
        assert_eq!(image_point(100., 200., 200., 200., 512, 512), None);
        let request = build_request(
            &model("segment"),
            "person",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            Some(json!({"asset_id":"source"})),
            None,
            &[
                MaskPoint {
                    x,
                    y,
                    positive: false,
                },
                MaskPoint {
                    x: last_x,
                    y: last_y,
                    positive: true,
                },
            ],
        )
        .expect("source pixel request");
        assert_eq!(request["input"]["points"][0]["x"].as_u64(), Some(257));
        assert_eq!(request["input"]["points"][0]["y"].as_u64(), Some(256));
        assert_eq!(request["input"]["points"][0]["positive"], false);
        assert_eq!(request["input"]["points"][1]["x"].as_u64(), Some(511));
        assert_eq!(request["input"]["points"][1]["y"].as_u64(), Some(511));
        assert_eq!(request["input"]["points"][1]["positive"], true);
        for coordinate in [-1., f64::NAN, f64::INFINITY] {
            assert!(
                build_request(
                    &model("segment"),
                    "person",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    Some(json!({"asset_id":"source"})),
                    None,
                    &[MaskPoint {
                        x: coordinate,
                        y: 0.,
                        positive: true
                    }],
                )
                .is_err()
            );
        }
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
    async fn assert_accepted_dispatch_recovery(
        retry_state: usize,
        fail_claim_write: bool,
        cx: &mut gpui::TestAppContext,
    ) {
        const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect width="8" height="4" fill="#22c55e"/></svg>"##;
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let posts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let requests = requests.clone();
            let posts = posts.clone();
            let state = state.clone();
            move |request| {
                let requests = requests.clone();
                let posts = posts.clone();
                let state = state.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models":[]}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            let key = request.headers()["Idempotency-Key"].to_str()?.to_owned();
                            let body: Value = serde_json::from_slice(
                                &bounded_body(request.into_body(), MAX_JSON_BYTES).await?,
                            )?;
                            requests
                                .lock()
                                .expect("request log")
                                .push(("POST", Some((key, body))));
                            let count = posts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            if count == 0 {
                                json!({"id":"dispatch-job", "status":"processing", "dispatch_protocol":"fanta-dispatch-v1", "dispatch_state":"awaiting_input", "retry_requires_input":true})
                            } else {
                                assert_eq!(
                                    count, 1,
                                    "one explicit redelivery must retain the same accepted job"
                                );
                                json!({"id":"dispatch-job", "status":"succeeded", "output":[{"svg":SVG}], "dispatch_protocol":"fanta-dispatch-v1", "dispatch_state":"terminal", "retry_requires_input":false})
                            }
                        }
                        (&Method::GET, "/v1/generations/dispatch-job") => {
                            requests.lock().expect("request log").push(("GET", None));
                            if posts.load(std::sync::atomic::Ordering::SeqCst) > 1 {
                                json!({"id":"dispatch-job", "status":"succeeded", "output":[{"svg":SVG}]})
                            } else {
                                match state.load(std::sync::atomic::Ordering::SeqCst) {
                                    0 => {
                                        json!({"id":"dispatch-job", "status":"processing", "dispatch_protocol":"fanta-dispatch-v1", "dispatch_state":"awaiting_input", "retry_requires_input":true})
                                    }
                                    1 => {
                                        json!({"id":"dispatch-job", "status":"processing", "dispatch_protocol":"fanta-dispatch-v1", "dispatch_state":"claimed", "retry_requires_input":false})
                                    }
                                    2 => {
                                        json!({"id":"dispatch-job", "status":"processing", "retry_requires_input":true})
                                    }
                                    _ => panic!("Unexpected dispatch state"),
                                }
                            }
                        }
                        route => panic!("Unexpected dispatch recovery route: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Image, client.clone(), cx);
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("source PNG");
        let mut preview = make_preview(&png.into_inner(), "image/png").expect("source preview");
        preview.width = 400;
        preview.height = 200;
        let submission = Submission {
            key: "durable-unclaimed-key".into(),
            request: json!({"model":"fanta-image-1","prompt":"Replace the sky","seed":"18446744073709551615",
                "input":{"source":{"asset_id":"source-original"},"mask":"AA==","mask_polarity":"edit_white","points":[{"x":123,"y":45,"positive":true}]}}),
            model: "fanta-image-1".into(),
            prompt: "Replace the sky".into(),
            source: Some(SourceImage {
                reference: json!({"asset_id":"source-original"}),
                name: "Original mask source".into(),
                preview,
            }),
            account,
            mode: GenerationMode::Image,
        };
        let expected = submission.saved().expect("saved exact source");
        let journal = view.read_with(cx, |view, _| {
            view.journal.clone().expect("persistent journal")
        });
        view.update(cx, |view, cx| view.submit(submission.clone(), cx));
        cx.run_until_parked();
        let snapshot = journal.load().await.expect("saved acceptance");
        assert_eq!(
            snapshot
                .records
                .first()
                .and_then(|record| record.request.as_ref()),
            Some(&expected.request),
            "acceptance before worker claim must retain the only input"
        );
        view.read_with(cx, |view, _| {
            assert!(view.error.is_none(), "acceptance error: {:?}", view.error);
            assert!(
                view.task.is_none(),
                "unclaimed input waits for explicit user retry, not polling"
            );
            assert_eq!(
                view.history.first().and_then(RunSummary::generation_id),
                Some("dispatch-job")
            );
            assert_eq!(
                view.unresolved_submission
                    .as_ref()
                    .map(|saved| saved.key.as_str()),
                Some(expected.key.as_str())
            );
        });
        let mut context = cx.cx.clone();
        cx.update(|window, _| window.remove_window());
        drop(view);
        context.run_until_parked();
        let (view, cx) =
            visual_workspace_with_client(GenerationMode::Image, client.clone(), &mut context);
        let recovered = view.read_with(cx, |view, _| {
            view.unresolved_submission
                .clone()
                .expect("accepted input restored")
        });
        assert_eq!(recovered.saved().expect("restored source"), expected);
        assert_eq!(
            requests.lock().expect("request log").len(),
            1,
            "reopening must not POST or automatically redeliver"
        );
        let database = cx.update(|_, cx| db::kvp::KeyValueStore::global(cx));
        if fail_claim_write {
            database
                .write(|connection| {
                    connection.exec(
                        "CREATE TRIGGER fail_dispatch_claim BEFORE INSERT ON scoped_kv_store
                 WHEN instr(NEW.value, 'Claimed') > 0
                 BEGIN SELECT RAISE(FAIL,'injected claim write failure'); END;",
                    )?()
                })
                .await
                .expect("install claim write failure");
        }
        state.store(retry_state, std::sync::atomic::Ordering::SeqCst);
        view.update(cx, |view, cx| view.submit(recovered, cx));
        cx.run_until_parked();
        if fail_claim_write {
            view.read_with(cx, |view, _| {
                assert!(view.error.is_some());
                assert!(view.task.is_none());
                assert!(view.unresolved_submission.is_some());
            });
            assert_eq!(
                journal.load().await.expect("unchanged acceptance"),
                snapshot,
                "a failed claim write must preserve the prior request and dispatch state"
            );
            assert_eq!(
                posts.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "claim persistence failure cannot lead to another POST"
            );
            let mut context = cx.cx.clone();
            cx.update(|window, _| window.remove_window());
            drop(view);
            context.run_until_parked();
            let (view, cx) =
                visual_workspace_with_client(GenerationMode::Image, client, &mut context);
            let recovered = view.read_with(cx, |view, _| {
                view.unresolved_submission
                    .clone()
                    .expect("failed claim still recoverable")
            });
            assert_eq!(recovered.saved().expect("source after failure"), expected);
            database
                .write(|connection| connection.exec("DROP TRIGGER fail_dispatch_claim")?())
                .await
                .expect("restore claim storage");
            view.update(cx, |view, cx| view.submit(recovered, cx));
            cx.run_until_parked();
            assert!(
                journal
                    .load()
                    .await
                    .expect("saved claim")
                    .records
                    .first()
                    .is_some_and(|record| record.request.is_none())
            );
            assert_eq!(posts.load(std::sync::atomic::Ordering::SeqCst), 1);
            view.update(cx, |view, _| view.task = None);
            return;
        }
        if retry_state == 2 {
            assert_eq!(
                posts.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "an unrecognized/missing protocol cannot authorize accepted-job POST"
            );
            let retained = journal.load().await.expect("mixed rollout state");
            assert_eq!(
                retained
                    .records
                    .first()
                    .and_then(|record| record.request.as_ref()),
                Some(&expected.request)
            );
            view.update(cx, |view, _| view.task = None);
            let run = view.read_with(cx, |view, _| {
                assert!(
                    view.unresolved_submission.is_none(),
                    "unknown metadata must not offer an unsafe retry"
                );
                view.history.first().cloned().expect("accepted history")
            });
            state.store(0, std::sync::atomic::Ordering::SeqCst);
            view.update(cx, |view, cx| view.check_status(run, cx));
            cx.run_until_parked();
            assert_eq!(
                posts.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "status check cannot redeliver input"
            );
            view.read_with(cx, |view, _| {
                assert!(view.task.is_none());
                assert_eq!(
                    view.unresolved_submission
                        .as_ref()
                        .map(|saved| saved.key.as_str()),
                    Some(expected.key.as_str())
                );
            });
        } else {
            assert!(
                journal
                    .load()
                    .await
                    .expect("released input")
                    .records
                    .first()
                    .is_some_and(|record| record.request.is_none())
            );
            let requests = requests.lock().expect("request log").clone();
            if retry_state == 0 {
                view.read_with(cx, |view, _| {
                    assert!(
                        view.error.is_none(),
                        "completed recovery error: {:?}",
                        view.error
                    );
                    assert_eq!(view.outputs.len(), 1);
                    assert!(view.preview.is_some());
                });
                assert_eq!(
                    requests.iter().map(|request| request.0).collect::<Vec<_>>(),
                    vec!["POST", "GET", "POST"]
                );
                assert_eq!(
                    requests.first(),
                    requests.get(2),
                    "redelivery preserves the original body and idempotency key"
                );
                view.update(cx, |view, cx| view.submit(submission, cx));
                cx.run_until_parked();
                assert_eq!(
                    posts.load(std::sync::atomic::Ordering::SeqCst),
                    2,
                    "a stale accepted submission fetches its finished job and cannot repeat it"
                );
            } else {
                assert_eq!(
                    requests.iter().map(|request| request.0).collect::<Vec<_>>(),
                    vec!["POST", "GET"]
                );
                assert_eq!(
                    posts.load(std::sync::atomic::Ordering::SeqCst),
                    1,
                    "a claim discovered after reopen prevents redelivery"
                );
                view.update(cx, |view, _| view.task = None);
            }
        }
    }

    #[gpui::test]
    async fn generation_dispatch_reopens_accepted_input_and_redelivers_only_explicitly(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_accepted_dispatch_recovery(0, false, cx).await;
    }

    #[gpui::test]
    async fn generation_dispatch_claim_discovered_after_restart_prevents_redelivery(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_accepted_dispatch_recovery(1, false, cx).await;
    }

    #[gpui::test]
    async fn generation_dispatch_unknown_rollout_keeps_input_without_post_permission(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_accepted_dispatch_recovery(2, false, cx).await;
    }

    #[gpui::test]
    async fn generation_dispatch_claim_write_failure_preserves_restart_recovery(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_accepted_dispatch_recovery(1, true, cx).await;
    }

    #[test]
    fn generation_dispatch_contract_requires_recognized_explicit_permission() {
        for (protocol, state, retry, can_retry, releases, polls) in [
            (
                Some("fanta-dispatch-v1"),
                Some("awaiting_input"),
                true,
                true,
                false,
                false,
            ),
            (
                Some("fanta-dispatch-v1"),
                Some("delivering"),
                true,
                true,
                false,
                true,
            ),
            (
                Some("fanta-dispatch-v1"),
                Some("awaiting_input"),
                false,
                false,
                false,
                true,
            ),
            (
                Some("fanta-dispatch-v1"),
                Some("claimed"),
                false,
                false,
                true,
                true,
            ),
            (
                Some("fanta-dispatch-v1"),
                Some("claimed"),
                true,
                false,
                false,
                true,
            ),
            (
                Some("fanta-dispatch-v1"),
                Some("unknown"),
                true,
                false,
                false,
                true,
            ),
            (
                Some("fanta-dispatch-v2"),
                Some("awaiting_input"),
                true,
                false,
                false,
                true,
            ),
            (None, Some("awaiting_input"), true, false, false, true),
            (Some("fanta-dispatch-v1"), None, true, false, false, true),
        ] {
            let response =
                parse_generation_response(json!({"id":"dispatch-contract","status":"processing",
                "dispatch_protocol":protocol,"dispatch_state":state,"retry_requires_input":retry}))
                .expect("response");
            assert_eq!(
                response.requires_input(),
                can_retry,
                "{protocol:?}/{state:?}/{retry}"
            );
            assert_eq!(
                matches!(
                    response.saved_result(),
                    SavedRunResult::Generation {
                        dispatch: SavedDispatch::Claimed,
                        ..
                    }
                ),
                releases
            );
            assert_eq!(response.should_poll(), polls);
        }
    }

    #[gpui::test]
    async fn generation_dispatch_delivering_keeps_polling_through_claim_and_completion(
        cx: &mut gpui::TestAppContext,
    ) {
        let posts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let posts = posts.clone();
            let gets = gets.clone();
            move |request| {
                let posts = posts.clone();
                let gets = gets.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models":[]}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            assert_eq!(
                                posts.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                                0,
                                "polling cannot repeat a submission"
                            );
                            json!({"id":"dispatch-poll","status":"processing","dispatch_protocol":"fanta-dispatch-v1","dispatch_state":"delivering","retry_requires_input":true})
                        }
                        (&Method::GET, "/v1/generations/dispatch-poll") => {
                            let count = gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            if count == 0 {
                                json!({"id":"dispatch-poll","status":"processing","dispatch_protocol":"fanta-dispatch-v1","dispatch_state":"claimed","retry_requires_input":false})
                            } else {
                                json!({"id":"dispatch-poll","status":"failed","error":{"message":"Synthetic worker terminal result"},"dispatch_protocol":"fanta-dispatch-v1","dispatch_state":"terminal","retry_requires_input":false})
                            }
                        }
                        route => panic!("Unexpected polling route: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client, cx);
        let journal = view.read_with(cx, |view, _| view.journal.clone().expect("journal"));
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "dispatch-poll-key".into(),
                    request: json!({"model":"fanta-video-1","prompt":"Sunrise"}),
                    model: "fanta-video-1".into(),
                    prompt: "Sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Video,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.pending && view.task.is_some()));
        assert!(
            journal
                .load()
                .await
                .expect("delivery saved")
                .records
                .first()
                .is_some_and(|record| record.request.is_some())
        );
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.pending && view.task.is_some()));
        assert!(
            journal
                .load()
                .await
                .expect("claim saved")
                .records
                .first()
                .is_some_and(|record| record.request.is_none())
        );
        assert!(view.read_with(cx, |view, _| view.unresolved_submission.is_none()));
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.pending);
            assert!(view.task.is_none());
            assert!(
                view.error
                    .as_ref()
                    .is_some_and(|error| error.contains("Synthetic worker terminal result"))
            );
        });
        assert_eq!(posts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(gets.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(matches!(
            journal
                .load()
                .await
                .expect("terminal saved")
                .records
                .first()
                .and_then(|record| record.result.as_ref()),
            Some(SavedRunResult::Generation { finished: true, .. })
        ));
    }
    async fn assert_dispatch_retry_race(sign_out: bool, cx: &mut gpui::TestAppContext) {
        let (sender, receiver) = futures::channel::oneshot::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let posts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let http = http_client::FakeHttpClient::create({
            let gate = gate.clone();
            let posts = posts.clone();
            let gets = gets.clone();
            move |request| {
                let gate = gate.clone();
                let posts = posts.clone();
                let gets = gets.clone();
                async move {
                    let response = match (request.method(), request.uri().path()) {
                        (&Method::GET, "/v1/models") => json!({"models":[]}),
                        (&Method::GET, "/v1/me") => recovery_account_fixture(),
                        (&Method::POST, "/v1/generations") => {
                            assert_eq!(
                                posts.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                                0,
                                "a stale recovery response cannot authorize another POST"
                            );
                            json!({"id":"dispatch-race","status":"processing","dispatch_protocol":"fanta-dispatch-v1","dispatch_state":"awaiting_input","retry_requires_input":true})
                        }
                        (&Method::GET, "/v1/generations/dispatch-race") => {
                            gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let receiver = gate
                                .lock()
                                .expect("response gate")
                                .take()
                                .context("status requested more than once")?;
                            receiver.await?;
                            json!({"id":"dispatch-race","status":"processing","dispatch_protocol":"fanta-dispatch-v1","dispatch_state":"awaiting_input","retry_requires_input":true})
                        }
                        route => panic!("Unexpected race route: {route:?}"),
                    };
                    Ok(http_client::Response::builder()
                        .status(200)
                        .body(response.to_string().into())?)
                }
            }
        });
        let client = catalog_client(cx, http);
        sign_in_catalog_client(&client, cx).await;
        let account = client.account_access_token();
        let (view, cx) = visual_workspace_with_client(GenerationMode::Video, client.clone(), cx);
        let journal = view.read_with(cx, |view, _| view.journal.clone().expect("journal"));
        view.update(cx, |view, cx| {
            view.submit(
                Submission {
                    key: "dispatch-race-key".into(),
                    request: json!({"model":"fanta-video-1","prompt":"Sunrise"}),
                    model: "fanta-video-1".into(),
                    prompt: "Sunrise".into(),
                    source: None,
                    account,
                    mode: GenerationMode::Video,
                },
                cx,
            )
        });
        cx.run_until_parked();
        let submission = view.read_with(cx, |view, _| {
            assert!(view.task.is_none());
            view.unresolved_submission
                .clone()
                .expect("accepted input requires recovery")
        });
        view.update(cx, |view, cx| view.submit(submission, cx));
        cx.run_until_parked();
        assert_eq!(gets.load(std::sync::atomic::Ordering::SeqCst), 1);
        if sign_out {
            let task = view.update(cx, |view, _| {
                view.task.take().expect("retain in-flight operation")
            });
            client.sign_out(&cx.to_async()).await;
            cx.run_until_parked();
            sender.send(()).expect("release old-account response");
            task.await;
            cx.run_until_parked();
            view.read_with(cx, |view, _| {
                assert!(view.active_run.is_none());
                assert!(view.history.is_empty());
                assert!(view.unresolved_submission.is_none());
            });
            assert!(
                journal
                    .load()
                    .await
                    .expect("old account recovery retained")
                    .records
                    .first()
                    .is_some_and(|record| record.request.is_some())
            );
        } else {
            let claimed: SavedRunResult = serde_json::from_value(
                json!({"Generation":{"id":"dispatch-race","finished":false,"dispatch":"Claimed"}}),
            )
            .expect("permanent worker claim");
            journal
                .accept("dispatch-race-key", claimed)
                .await
                .expect("another tab persists worker claim");
            sender.send(()).expect("release stale unclaimed response");
            cx.run_until_parked();
            assert!(
                journal
                    .load()
                    .await
                    .expect("claim wins")
                    .records
                    .first()
                    .is_some_and(|record| record.request.is_none())
            );
            view.read_with(cx,|view,_| {
                assert!(view.unresolved_submission.is_none());
                assert!(view.task.is_some(),"a durable claim overrides stale awaiting-input metadata and continues GET polling");
            });
            view.update(cx, |view, _| view.task = None);
        }
        assert_eq!(posts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[gpui::test]
    async fn generation_dispatch_concurrent_claim_wins_over_stale_unclaimed_get(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_dispatch_retry_race(false, cx).await;
    }

    #[gpui::test]
    async fn generation_dispatch_signout_during_retry_get_cannot_redeliver(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_dispatch_retry_race(true, cx).await;
    }
}
