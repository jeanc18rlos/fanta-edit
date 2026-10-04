//! Plan & build: the editor's side of the v2 agent sessions
//! (`/v2/agent/sessions`; the backend's `docs/editor-blueprint.md` §7.3).
//!
//! The user writes a brief. The backend's commander plans a few tasks and its
//! executor writes each task's design ops. The panel applies a task's ops to
//! the canvas as one undoable batch, a preview the user then reviews:
//!
//! - **Accept** sends the resulting page to be checked (lint and critic). The
//!   next task's ops arrive, or the same task's again when the check failed.
//! - **Retry** undoes the preview and asks for new ops, with optional notes.
//! - **Skip** undoes the preview and moves to the next task.
//!
//! With auto-accept on, each preview is accepted as soon as it lands.

use std::{pin::pin, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use client::{Client, ClientSettings};
use design_surface::{DESIGN_OP_CAPABILITIES, DesignOp};
use futures::{AsyncReadExt as _, future::Either};
use gpui::{App, Context, Entity, Task, WeakEntity, Window};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde::Deserialize;
use serde_json::{Value, json};
use settings::Settings as _;
use ui::{Switch, ToggleState, prelude::*};
use ui_input::InputField;

use crate::document::FigItem;
use crate::inspector_components::{InspectorMessage, InspectorSectionHeader};

/// Planning runs two model calls before it answers, so allow for slow models.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(240);
const MAX_RESPONSE_BYTES: usize = 4 << 20;
/// The backend reads at most 50 top-level layers and 64 KiB of context.
const SCENE_LAYERS: usize = 50;
const SCENE_DEPTH: u32 = 8;
const CONTEXT_CHARS: usize = 60_000;

/// An agent session as `/v2/agent/sessions` returns it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AgentSession {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) plan: SessionPlan,
    #[serde(default)]
    pub(crate) cursor: usize,
    #[serde(default)]
    pub(crate) pending: Option<PendingOps>,
    #[serde(default)]
    pub(crate) results: Vec<TaskResult>,
    #[serde(default)]
    pub(crate) credits_used: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SessionPlan {
    #[serde(default)]
    pub(crate) summary: String,
    pub(crate) tasks: Vec<PlanTask>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PlanTask {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) instructions: String,
}

/// The executor's ops for the current task.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PendingOps {
    #[serde(default)]
    pub(crate) ops: Vec<Value>,
    #[serde(default)]
    pub(crate) notes: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TaskResult {
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) passed: bool,
    #[serde(default)]
    pub(crate) skipped: bool,
    #[serde(default)]
    pub(crate) issues: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    Waiting,
    Current,
    Passed,
    Failed,
    Skipped,
}

impl AgentSession {
    /// Whether the session is waiting on the editor (ops to apply and report).
    pub(crate) fn is_open(&self) -> bool {
        self.status == "awaiting_observation"
    }

    pub(crate) fn task_status(&self, index: usize) -> TaskStatus {
        if self.is_open() && index == self.cursor {
            return TaskStatus::Current;
        }
        let Some(task) = self.plan.tasks.get(index) else {
            return TaskStatus::Waiting;
        };
        match self
            .results
            .iter()
            .rev()
            .find(|result| result.task == task.id)
        {
            Some(result) if result.skipped => TaskStatus::Skipped,
            Some(result) if result.passed => TaskStatus::Passed,
            Some(_) if index < self.cursor || !self.is_open() => TaskStatus::Failed,
            _ => TaskStatus::Waiting,
        }
    }

    /// The issues the last check raised on the current task, when it is being
    /// retried.
    pub(crate) fn current_issues(&self) -> &[String] {
        let Some(task) = self.plan.tasks.get(self.cursor) else {
            return &[];
        };
        match self.results.last() {
            Some(result) if self.is_open() && result.task == task.id && !result.passed => {
                &result.issues
            }
            _ => &[],
        }
    }

    /// The current task's ops in the editor's vocabulary.
    pub(crate) fn pending_ops(&self) -> Result<Vec<DesignOp>> {
        let pending = self
            .pending
            .as_ref()
            .context("the agent returned no ops for this task")?;
        if pending.ops.is_empty() {
            bail!("the agent returned no ops for this task");
        }
        pending
            .ops
            .iter()
            .enumerate()
            .map(|(index, op)| {
                serde_json::from_value(op.clone())
                    .with_context(|| format!("op {index} is not one this editor understands"))
            })
            .collect()
    }

    pub(crate) fn pending_notes(&self) -> Option<String> {
        let notes = match self.pending.as_ref().map(|pending| &pending.notes)? {
            Value::String(notes) => notes.trim().to_owned(),
            Value::Array(notes) => notes
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        (!notes.is_empty()).then_some(notes)
    }

    fn count(&self, status: TaskStatus) -> usize {
        (0..self.plan.tasks.len())
            .filter(|index| self.task_status(*index) == status)
            .count()
    }
}

/// A task's ops as applied to the canvas, awaiting the user's review.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preview {
    /// The history depth right after the batch, so the panel only undoes the
    /// preview while it is still the latest edit.
    undo_depth: Option<usize>,
    /// Why the batch could not be applied (it was then rolled back).
    errors: Vec<String>,
    op_count: usize,
}

impl Preview {
    fn applied(&self) -> bool {
        self.errors.is_empty()
    }
}

struct Inputs {
    brief: Entity<InputField>,
    notes: Entity<InputField>,
}

impl Inputs {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        let brief = cx.new(|cx| InputField::new(window, cx, "Describe what to design"));
        brief
            .read(cx)
            .editor()
            .clone()
            .set_multiline(Some(5), window, cx);
        let notes = cx.new(|cx| InputField::new(window, cx, "What should change? (optional)"));
        Self { brief, notes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No request in flight.
    Ready,
    Busy(&'static str),
}

pub(crate) struct PlanBuildPanel {
    item: WeakEntity<FigItem>,
    /// Absent in builds without an account client (some tests).
    client: Option<Arc<Client>>,
    base_url: String,
    /// Created on first render, which has the window an input needs.
    inputs: Option<Inputs>,
    session: Option<AgentSession>,
    preview: Option<Preview>,
    phase: Phase,
    error: Option<SharedString>,
    auto_accept: bool,
    _request: Option<Task<()>>,
}

impl PlanBuildPanel {
    pub(crate) fn new(item: WeakEntity<FigItem>, cx: &mut Context<Self>) -> Self {
        let client = Client::try_global(cx);
        let base_url = ClientSettings::try_get(cx)
            .map(|settings| settings.server_url.trim_end_matches('/').to_owned())
            .unwrap_or_default();
        Self::with_client(item, client, base_url)
    }

    fn with_client(
        item: WeakEntity<FigItem>,
        client: Option<Arc<Client>>,
        base_url: String,
    ) -> Self {
        Self {
            item,
            client,
            base_url,
            inputs: None,
            session: None,
            preview: None,
            phase: Phase::Ready,
            error: None,
            auto_accept: false,
            _request: None,
        }
    }

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Busy(_))
    }

    fn input_text(&self, input: fn(&Inputs) -> &Entity<InputField>, cx: &App) -> String {
        self.inputs
            .as_ref()
            .map(|inputs| input(inputs).read(cx).text(cx).trim().to_owned())
            .unwrap_or_default()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let brief = self.input_text(|inputs| &inputs.brief, cx);
        if brief.chars().count() < 3 {
            self.error = Some("Describe what to design first.".into());
            cx.notify();
            return;
        }
        let body = self.scene(cx).map(|scene| {
            json!({
                "brief": brief,
                "scene": scene,
                "context": self.context(cx),
                "capabilities": DESIGN_OP_CAPABILITIES,
            })
        });
        self.send(
            Method::POST,
            "/v2/agent/sessions".into(),
            body,
            "Planning…",
            cx,
        );
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(preview)) = (&self.session, &self.preview) else {
            return;
        };
        let path = format!("/v2/agent/sessions/{}/observe", session.id);
        let (applied, errors) = (preview.applied(), preview.errors.clone());
        let body = self
            .scene(cx)
            .map(|scene| json!({ "scene": scene, "applied": applied, "errors": errors }));
        self.send(Method::POST, path, body, "Checking the result…", cx);
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let path = format!("/v2/agent/sessions/{}/retry", session.id);
        let feedback = self.input_text(|inputs| &inputs.notes, cx);
        if let Some(inputs) = &self.inputs {
            inputs.notes.update(cx, |notes, cx| notes.clear(window, cx));
        }
        self.discard_preview(cx);
        let body = self
            .scene(cx)
            .map(|scene| json!({ "scene": scene, "feedback": feedback }));
        self.send(Method::POST, path, body, "Getting new ops…", cx);
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let path = format!("/v2/agent/sessions/{}/skip", session.id);
        self.discard_preview(cx);
        let body = self.scene(cx).map(|scene| json!({ "scene": scene }));
        self.send(Method::POST, path, body, "Moving on…", cx);
    }

    /// Stop the session. What the panel already applied stays on the canvas.
    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let path = format!("/v2/agent/sessions/{}/cancel", session.id);
        self.preview = None;
        self.send(Method::POST, path, Ok(Value::Null), "Stopping…", cx);
    }

    fn new_plan(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.preview = None;
        self.error = None;
        cx.notify();
    }

    fn send(
        &mut self,
        method: Method,
        path: String,
        body: Result<Value>,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let body = match body {
            Ok(body) => body,
            Err(error) => {
                self.error = Some(format!("{error:#}").into());
                cx.notify();
                return;
            }
        };
        let Some((client, token)) = self
            .client
            .clone()
            .and_then(|client| Some((client.clone(), client.account_access_token()?)))
        else {
            self.error = Some("Sign in to Fanta to plan and build.".into());
            cx.notify();
            return;
        };
        self.phase = Phase::Busy(label);
        self.error = None;
        let url = format!("{}{path}", self.base_url);
        let executor = cx.background_executor().clone();
        self._request = Some(cx.spawn(async move |this, cx| {
            let request = pin!(request_session(client, url, token, method, body));
            let result =
                match futures::future::select(request, executor.timer(REQUEST_TIMEOUT)).await {
                    Either::Left((result, _)) => result,
                    Either::Right(_) => Err(anyhow!(
                        "The agent took too long to answer. Try again in a moment."
                    )),
                };
            this.update(cx, |this, cx| this.receive(result, cx)).ok();
        }));
        cx.notify();
    }

    fn receive(&mut self, result: Result<AgentSession>, cx: &mut Context<Self>) {
        self.phase = Phase::Ready;
        self._request = None;
        match result {
            Ok(session) => {
                let open = session.is_open();
                self.session = Some(session);
                self.preview = None;
                if open {
                    self.preview_pending(cx);
                }
            }
            Err(error) => self.error = Some(format!("{error:#}").into()),
        }
        cx.notify();
    }

    /// Apply the current task's ops to the canvas as one undoable batch.
    fn preview_pending(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let task = session
            .plan
            .tasks
            .get(session.cursor)
            .map(|task| task.title.clone())
            .unwrap_or_default();
        let preview = match (session.pending_ops(), self.item.upgrade()) {
            (Err(error), _) => Preview {
                undo_depth: None,
                errors: vec![format!("{error:#}")],
                op_count: 0,
            },
            (Ok(_), None) => {
                self.error = Some("The design was closed.".into());
                return;
            }
            (Ok(ops), Some(item)) => {
                let label = format!("Plan & build: {task}");
                let op_count = ops.len();
                match crate::agent_surface::apply_ops_to_item(&item, &ops, &label, cx) {
                    Ok(outcome) if outcome["applied"] == json!(true) => Preview {
                        undo_depth: undo_depth(&item, cx),
                        errors: Vec::new(),
                        op_count,
                    },
                    Ok(outcome) => Preview {
                        undo_depth: None,
                        errors: batch_errors(&outcome),
                        op_count,
                    },
                    Err(error) => Preview {
                        undo_depth: None,
                        errors: vec![format!("{error:#}")],
                        op_count,
                    },
                }
            }
        };
        self.preview = Some(preview);
        if self.auto_accept {
            self.accept(cx);
        }
    }

    /// Undo the preview while it is still the latest edit. After later edits
    /// it stays, since undoing would take the user's edits with it.
    fn discard_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        let (Some(depth), Some(item)) = (preview.undo_depth, self.item.upgrade()) else {
            return;
        };
        if undo_depth(&item, cx) != Some(depth) {
            self.error = Some(
                "The preview stays: the canvas changed after it. Undo it yourself if you don't want it."
                    .into(),
            );
            return;
        }
        if let Err(error) = item.update(cx, |item, cx| item.undo(cx)) {
            self.error = Some(format!("Could not undo the preview: {error:#}").into());
        }
    }

    fn scene(&self, cx: &App) -> Result<Vec<Value>> {
        let item = self.item.upgrade().context("The design was closed.")?;
        crate::agent_surface::page_scene(item.read(cx), SCENE_DEPTH, SCENE_LAYERS)
    }

    /// The project's design specification, which the planner and executor
    /// follow alongside the brief.
    fn context(&self, cx: &App) -> String {
        let spec = self
            .item
            .upgrade()
            .and_then(|item| {
                item.read(cx)
                    .project_root()
                    .map(std::path::Path::to_path_buf)
            })
            .and_then(|root| {
                design_surface::read_project_design_spec(&root)
                    .ok()
                    .flatten()
            });
        spec.map(|spec| spec.prompt_context().chars().take(CONTEXT_CHARS).collect())
            .unwrap_or_default()
    }
}

fn undo_depth(item: &Entity<FigItem>, cx: &App) -> Option<usize> {
    item.read(cx).doc().map(|doc| doc.history.undo_depth())
}

/// The failing op's message from a rolled-back batch.
fn batch_errors(outcome: &Value) -> Vec<String> {
    let failed = outcome["ops"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|status| {
            (status["status"] == "failed").then(|| {
                format!(
                    "op {}: {}",
                    status["index"],
                    status["error"].as_str().unwrap_or("failed")
                )
            })
        });
    let mut errors: Vec<String> = failed.collect();
    if errors.is_empty() {
        errors.push(
            outcome["error"]
                .as_str()
                .unwrap_or("the batch could not be applied")
                .to_owned(),
        );
    }
    errors
}

async fn request_session(
    client: Arc<Client>,
    url: String,
    token: Arc<str>,
    method: Method,
    body: Value,
) -> Result<AgentSession> {
    let mut request = Request::builder()
        .method(method)
        .uri(url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"));
    let body = if body.is_null() {
        AsyncBody::empty()
    } else {
        request = request.header("Content-Type", "application/json");
        AsyncBody::from(serde_json::to_vec(&body)?)
    };
    let response = client
        .http_client()
        .send(request.body(body)?)
        .await
        .context("Could not reach Fanta. Check your connection and try again.")?;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .into_body()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .context("Fanta's answer was cut off. Try again.")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        bail!("Fanta's answer was too large to read.");
    }
    if !status.is_success() {
        bail!(rejection_message(status.as_u16(), &bytes));
    }
    serde_json::from_slice(&bytes)
        .context("Fanta returned an agent session this editor can't read.")
}

fn rejection_message(status: u16, body: &[u8]) -> String {
    match status {
        401 => return "Your Fanta session expired. Sign in again to continue.".into(),
        402 => return "You need more Fanta credits. Open Credits & billing to top up.".into(),
        404 => {
            return "Plan & build needs the Fanta v2 API; this build's server doesn't offer it."
                .into();
        }
        409 => return "The session moved on. Start a new plan.".into(),
        429 => return "Too many agent requests. Wait a moment and try again.".into(),
        _ => {}
    }
    let value: Value = serde_json::from_slice(body).unwrap_or_default();
    value["error"]["message"]
        .as_str()
        .or_else(|| value["detail"].as_str())
        .or_else(|| value["message"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Fanta answered with an error ({status})."))
}

impl Render for PlanBuildPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.inputs.is_none() {
            self.inputs = Some(Inputs::new(window, cx));
        }
        let root = v_flex()
            .id("fanta-plan-build")
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().colors().panel_background)
            .child(InspectorSectionHeader::new("Plan & build"));
        let body = match &self.session {
            None => self.render_brief(cx),
            Some(session) => self.render_session(session, cx),
        };
        root.child(body)
            .when_some(self.error.clone(), |root, error| {
                root.child(
                    div()
                        .px_3()
                        .pb_2()
                        .child(Label::new(error).size(LabelSize::Small).color(Color::Error)),
                )
            })
    }
}

impl PlanBuildPanel {
    fn render_brief(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .px_3()
            .gap_2()
            .child(
                Label::new(
                    "The agent plans the work as tasks, then builds each one on the canvas for you to accept, retry or skip.",
                )
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .children(self.inputs.as_ref().map(|inputs| inputs.brief.clone()))
            .child(self.render_auto_accept(cx))
            .child(
                Button::new("fanta-plan-build-start", "Plan & build")
                    .start_icon(Icon::new(IconName::Sparkle).size(IconSize::XSmall))
                    .full_width()
                    .disabled(self.busy())
                    .on_click(cx.listener(|panel, _, _, cx| panel.start(cx))),
            )
            .when_some(self.busy_label(), |flex, label| flex.child(busy_row(label)))
            .into_any_element()
    }

    fn render_auto_accept(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .child(
                Switch::new(
                    "fanta-plan-build-auto-accept",
                    ToggleState::from(self.auto_accept),
                )
                .on_click(cx.listener(|panel, _: &ToggleState, _, cx| {
                    panel.auto_accept = !panel.auto_accept;
                    cx.notify();
                })),
            )
            .child(
                Label::new("Accept each task automatically")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

    fn busy_label(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Busy(label) => Some(label),
            Phase::Ready => None,
        }
    }

    fn render_session(&self, session: &AgentSession, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex().px_3().gap_1();
        if !session.plan.summary.trim().is_empty() {
            list = list.child(
                Label::new(session.plan.summary.trim().to_owned())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }
        for (index, task) in session.plan.tasks.iter().enumerate() {
            let status = session.task_status(index);
            let (icon, color) = match status {
                TaskStatus::Waiting => (IconName::Circle, Color::Muted),
                TaskStatus::Current => (IconName::ArrowRight, Color::Accent),
                TaskStatus::Passed => (IconName::Check, Color::Success),
                TaskStatus::Failed => (IconName::XCircle, Color::Error),
                TaskStatus::Skipped => (IconName::Dash, Color::Muted),
            };
            let title = if task.title.trim().is_empty() {
                task.instructions.clone()
            } else {
                task.title.clone()
            };
            list = list.child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(icon).size(IconSize::Small).color(color))
                    .child(Label::new(title).size(LabelSize::Small).color(
                        if status == TaskStatus::Current {
                            Color::Default
                        } else {
                            Color::Muted
                        },
                    )),
            );
            if status == TaskStatus::Current {
                list = list.child(self.render_current(session, task, cx));
            }
        }
        if !session.is_open() {
            list = list.child(self.render_finished(session, cx));
        }
        list.child(
            Label::new(format!(
                "{} credits used",
                format_credits(session.credits_used)
            ))
            .size(LabelSize::XSmall)
            .color(Color::Muted),
        )
        .into_any_element()
    }

    fn render_current(
        &self,
        session: &AgentSession,
        task: &PlanTask,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut detail = v_flex().pl_6().gap_1();
        if !task.instructions.trim().is_empty() && !task.title.trim().is_empty() {
            detail = detail.child(
                Label::new(task.instructions.trim().to_owned())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }
        let issues = session.current_issues();
        if !issues.is_empty() {
            detail = detail.child(
                Label::new(format!("Last check: {}", issues.join("; ")))
                    .size(LabelSize::XSmall)
                    .color(Color::Warning),
            );
        }
        if let Some(label) = self.busy_label() {
            return detail.child(busy_row(label)).into_any_element();
        }
        if let Some(notes) = session.pending_notes() {
            detail = detail.child(Label::new(notes).size(LabelSize::XSmall));
        }
        if let Some(preview) = &self.preview {
            detail = detail.child(if preview.applied() {
                Label::new(format!(
                    "{} {} applied. Review the canvas.",
                    preview.op_count,
                    if preview.op_count == 1 { "op" } else { "ops" }
                ))
                .size(LabelSize::XSmall)
                .color(Color::Muted)
            } else {
                Label::new(format!(
                    "Could not apply: {}. Accept to send this back to the agent.",
                    preview.errors.join("; ")
                ))
                .size(LabelSize::XSmall)
                .color(Color::Error)
            });
        }
        detail
            .children(self.inputs.as_ref().map(|inputs| inputs.notes.clone()))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("fanta-plan-build-accept", "Accept")
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .disabled(self.preview.is_none())
                            .on_click(cx.listener(|panel, _, _, cx| panel.accept(cx))),
                    )
                    .child(
                        Button::new("fanta-plan-build-retry", "Retry")
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(|panel, _, window, cx| panel.retry(window, cx))),
                    )
                    .child(
                        Button::new("fanta-plan-build-skip", "Skip")
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(|panel, _, _, cx| panel.skip(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        IconButton::new("fanta-plan-build-stop", IconName::Stop)
                            .icon_size(IconSize::Small)
                            .tooltip(ui::Tooltip::text("Stop the plan"))
                            .on_click(cx.listener(|panel, _, _, cx| panel.stop(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_finished(&self, session: &AgentSession, cx: &mut Context<Self>) -> AnyElement {
        let summary = if session.status == "done" {
            format!(
                "Done: {} passed, {} failed, {} skipped.",
                session.count(TaskStatus::Passed),
                session.count(TaskStatus::Failed),
                session.count(TaskStatus::Skipped)
            )
        } else {
            "Stopped. What was applied stays on the canvas.".to_owned()
        };
        v_flex()
            .pt_2()
            .gap_2()
            .child(InspectorMessage::new(summary))
            .when_some(self.busy_label(), |flex, label| flex.child(busy_row(label)))
            .child(
                Button::new("fanta-plan-build-new", "New plan")
                    .full_width()
                    .disabled(self.busy())
                    .on_click(cx.listener(|panel, _, _, cx| panel.new_plan(cx))),
            )
            .into_any_element()
    }
}

fn busy_row(label: &'static str) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(
            Icon::new(IconName::LoadCircle)
                .size(IconSize::Small)
                .color(Color::Muted),
        )
        .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
}

fn format_credits(credits: f64) -> String {
    if credits.fract() == 0.0 {
        format!("{credits:.0}")
    } else {
        format!("{credits:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(value: Value) -> AgentSession {
        serde_json::from_value(value).expect("a session")
    }

    fn plan() -> Value {
        json!({ "summary": "Landing page", "tasks": [
            { "id": "t1", "title": "Layout", "instructions": "Stack the page" },
            { "id": "t2", "title": "Hero", "instructions": "Add a heading" },
            { "id": "t3", "title": "Footer", "instructions": "Add links" },
        ]})
    }

    #[test]
    fn task_status_follows_the_cursor_and_results() {
        let open = session(json!({
            "id": "s", "status": "awaiting_observation", "plan": plan(), "cursor": 1,
            "pending": { "task": "t2", "ops": [{"op": "delete", "id": "1"}], "notes": "A heading" },
            "results": [
                { "task": "t1", "passed": false, "issues": ["contrast"] },
                { "task": "t1", "passed": true, "issues": [] },
            ],
            "credits_used": 12,
        }));
        assert_eq!(open.task_status(0), TaskStatus::Passed);
        assert_eq!(open.task_status(1), TaskStatus::Current);
        assert_eq!(open.task_status(2), TaskStatus::Waiting);
        assert!(open.current_issues().is_empty());
        assert_eq!(open.pending_notes().as_deref(), Some("A heading"));
        assert_eq!(open.pending_ops().unwrap().len(), 1);

        let retrying = session(json!({
            "id": "s", "status": "awaiting_observation", "plan": plan(), "cursor": 0,
            "pending": { "task": "t1", "ops": [] },
            "results": [{ "task": "t1", "passed": false, "issues": ["contrast"] }],
        }));
        assert_eq!(retrying.task_status(0), TaskStatus::Current);
        assert_eq!(retrying.current_issues(), ["contrast".to_owned()]);
        assert!(
            retrying.pending_ops().is_err(),
            "no ops is reported, not applied"
        );

        let done = session(json!({
            "id": "s", "status": "done", "plan": plan(), "cursor": 3, "pending": null,
            "results": [
                { "task": "t1", "passed": true },
                { "task": "t2", "passed": false, "issues": ["overlap"] },
                { "task": "t3", "passed": false, "skipped": true },
            ],
        }));
        assert_eq!(
            (0..3)
                .map(|index| done.task_status(index))
                .collect::<Vec<_>>(),
            [TaskStatus::Passed, TaskStatus::Failed, TaskStatus::Skipped]
        );
        assert_eq!(done.count(TaskStatus::Failed), 1);
    }

    #[test]
    fn unknown_ops_are_reported_instead_of_applied() {
        let session = session(json!({
            "id": "s", "status": "awaiting_observation", "plan": plan(),
            "pending": { "task": "t1", "ops": [{"op": "delete", "id": "1"}, {"op": "explode"}] },
        }));
        let error = session.pending_ops().unwrap_err();
        assert!(format!("{error:#}").contains("op 1"), "{error:#}");
    }

    #[test]
    fn a_rolled_back_batch_reports_the_failing_op() {
        let outcome = json!({ "applied": false, "error": "op 1 failed; the batch was rolled back",
            "ops": [{"index": 0, "status": "ok"}, {"index": 1, "status": "failed", "error": "node 9 does not exist"},
                    {"index": 2, "status": "skipped"}] });
        assert_eq!(batch_errors(&outcome), ["op 1: node 9 does not exist"]);
        assert_eq!(
            batch_errors(&json!({ "applied": false, "error": "nope" })),
            ["nope"]
        );
    }

    fn page_names(item: &Entity<FigItem>, cx: &mut gpui::VisualTestContext) -> Vec<String> {
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let page = doc.active_page().expect("page");
            doc.scene
                .children_of(Some(page))
                .iter()
                .filter_map(|id| doc.scene.get(*id).map(|node| node.name.clone()))
                .collect()
        })
    }

    fn frame_op(name: &str) -> Value {
        json!({"op": "create_node", "node_type": "frame", "name": name,
               "x": 0, "y": 0, "width": 200, "height": 100})
    }

    #[gpui::test]
    async fn previews_each_task_and_accepts_retries_and_skips_through_the_session_api(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::sync::Mutex;

        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let recorded = calls.clone();
        let http = http_client::FakeHttpClient::create(move |request| {
            let recorded = recorded.clone();
            async move {
                let path = request.uri().path().to_owned();
                assert_eq!(
                    request
                        .headers()
                        .get("Authorization")
                        .and_then(|value| value.to_str().ok()),
                    Some("Bearer fnt_live_plan_test")
                );
                let mut bytes = Vec::new();
                request.into_body().read_to_end(&mut bytes).await?;
                let body: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                recorded.lock().unwrap().push((path.clone(), body));
                let tasks = json!({ "summary": "A landing page", "tasks": [
                    { "id": "t1", "title": "Hero", "instructions": "Add the hero" },
                    { "id": "t2", "title": "Footer", "instructions": "Add the footer" },
                ]});
                let session = match path.as_str() {
                    "/v2/agent/sessions" => json!({ "id": "s1", "status": "awaiting_observation",
                        "plan": tasks, "cursor": 0, "credits_used": 3,
                        "pending": { "task": "t1", "ops": [frame_op("Hero")], "notes": "A hero" } }),
                    "/v2/agent/sessions/s1/retry" => {
                        json!({ "id": "s1", "status": "awaiting_observation",
                        "plan": tasks, "cursor": 0, "credits_used": 4,
                        "pending": { "task": "t1", "ops": [frame_op("Bigger hero")] } })
                    }
                    "/v2/agent/sessions/s1/observe" => {
                        json!({ "id": "s1", "status": "awaiting_observation",
                        "plan": tasks, "cursor": 1, "credits_used": 6,
                        "results": [{ "task": "t1", "passed": true }],
                        "pending": { "task": "t2", "ops": [frame_op("Footer")] } })
                    }
                    "/v2/agent/sessions/s1/skip" => json!({ "id": "s1", "status": "done",
                        "plan": tasks, "cursor": 2, "credits_used": 6, "pending": null,
                        "results": [{ "task": "t1", "passed": true },
                                    { "task": "t2", "passed": false, "skipped": true }] }),
                    other => panic!("unexpected request to {other}"),
                };
                Ok(http_client::Response::builder()
                    .status(200)
                    .body(session.to_string().into())?)
            }
        });
        let client = cx.update(|cx| {
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            release_channel::init_test(
                semver::Version::new(0, 0, 0),
                release_channel::ReleaseChannel::Stable,
                cx,
            );
            cx.set_http_client(http);
            Client::production(cx)
        });
        client.override_authenticate(|_| {
            Task::ready(Ok(client::Credentials {
                user_id: 1,
                access_token: "fnt_live_plan_test".into(),
            }))
        });
        client
            .sign_in(false, &cx.to_async())
            .await
            .expect("sign in");
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            gpui_component::init(cx);
            crate::theme_bridge::init(cx);
            editor::init(cx);
        });
        let project = project::Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = fanta_doc::Doc::new();
        let page = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(Default::default()));
        let page_id = page.id;
        doc.apply(fanta_doc::Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let weak = item.downgrade();
        let (panel, cx) = cx.add_window_view(move |window, cx| {
            let mut panel =
                PlanBuildPanel::with_client(weak, Some(client), "https://api-v2.test".into());
            panel.inputs = Some(Inputs::new(window, cx));
            panel
        });

        panel.update_in(cx, |panel, window, cx| {
            let brief = panel.inputs.as_ref().unwrap().brief.clone();
            brief.update(cx, |brief, cx| brief.set_text("A landing page", window, cx));
            panel.start(cx);
        });
        cx.run_until_parked();
        assert_eq!(
            page_names(&item, cx),
            ["Hero"],
            "the first task is previewed"
        );
        {
            let calls = calls.lock().unwrap();
            let (_, body) = &calls[0];
            assert_eq!(body["brief"], "A landing page");
            assert_eq!(body["scene"], json!([]));
            assert!(
                body["capabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("op:componentize"))
            );
        }

        panel.update_in(cx, |panel, window, cx| {
            let notes = panel.inputs.as_ref().unwrap().notes.clone();
            notes.update(cx, |notes, cx| notes.set_text("Make it bigger", window, cx));
            panel.retry(window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            page_names(&item, cx),
            ["Bigger hero"],
            "retry undoes the preview and previews the new ops"
        );
        assert_eq!(calls.lock().unwrap()[1].1["feedback"], "Make it bigger");

        panel.update_in(cx, |panel, _, cx| panel.accept(cx));
        cx.run_until_parked();
        {
            let calls = calls.lock().unwrap();
            let (path, body) = &calls[2];
            assert_eq!(path, "/v2/agent/sessions/s1/observe");
            assert_eq!(body["applied"], true);
            assert_eq!(body["scene"][0]["name"], "Bigger hero");
        }
        assert_eq!(page_names(&item, cx), ["Bigger hero", "Footer"]);

        panel.update_in(cx, |panel, _, cx| panel.skip(cx));
        cx.run_until_parked();
        assert_eq!(
            page_names(&item, cx),
            ["Bigger hero"],
            "skip undoes the previewed footer"
        );
        panel.read_with(cx, |panel, _| {
            let session = panel.session.as_ref().expect("session");
            assert_eq!(session.status, "done");
            assert_eq!(session.task_status(0), TaskStatus::Passed);
            assert_eq!(session.task_status(1), TaskStatus::Skipped);
            assert!(panel.error.is_none(), "{:?}", panel.error);
        });
    }

    #[test]
    fn rejections_explain_what_to_do() {
        assert!(rejection_message(402, b"{}").contains("credits"));
        assert!(rejection_message(404, b"{}").contains("v2 API"));
        assert_eq!(
            rejection_message(400, br#"{"error": {"message": "brief too short"}}"#),
            "brief too short"
        );
        assert_eq!(
            rejection_message(500, b"oops"),
            "Fanta answered with an error (500)."
        );
    }
}
