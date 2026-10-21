//! The cloud engine worker: runs an ordered list of steps on one Fanta
//! project and answers with each step's result. The wire format is frozen by
//! `docs/cloud-engine-contracts.md`; change both together. `fanta-engine run
//! --request` runs the same path on a request file.

use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use base64::Engine as _;
use fanta_design_ops::{CommentDraft, DesignOp};
use fanta_headless::{HeadlessProject, PageRef, RenderTarget};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The longest a run may take; Cloud Run ends the request at 300 s.
pub const MAX_DEADLINE_MS: u64 = 280_000;
pub const MAX_RENDER_TARGETS: usize = 8;
/// The most the overlay's decoded files may add up to.
pub const MAX_OVERLAY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// Echoed in the response and in logs.
    pub request_id: String,
    /// Milliseconds the run may take, at most [`MAX_DEADLINE_MS`].
    pub deadline_ms: u64,
    pub source: Source,
    pub steps: Vec<Step>,
}

/// Where the project comes from. The engine works on a private copy and never
/// writes to the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// A commit of a GitHub repository, read with an installation token scoped
    /// to that repository.
    Github {
        /// `owner/name`.
        repo: String,
        /// The full 40-character commit SHA.
        sha: String,
        /// The project folder inside the repository, `/`-separated; empty for
        /// the repository root.
        #[serde(default)]
        path: String,
        token: Token,
        /// Unpushed files from the open app, laid over the commit.
        #[serde(default)]
        overlay: Option<Overlay>,
    },
    /// A project directory on the engine's own disk. Development only.
    Dir { dir: PathBuf },
}

/// A credential that never appears in `Debug` output, so logging a request
/// cannot leak it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(pub String);

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(<redacted>)")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    pub files: Vec<OverlayFile>,
}

/// One changed file: either its new content or `deleted: true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayFile {
    /// Relative to the project folder, `/`-separated.
    pub path: String,
    #[serde(default)]
    pub content_b64: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// One step of a run. Steps share one loaded document, in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// A struct variant so `deny_unknown_fields` applies: serde ignores extra
    /// fields beside a unit variant's tag.
    Inspect {},
    CreatePage {
        name: String,
    },
    Apply {
        /// The page parentless ops place on; the project's default page when
        /// omitted.
        #[serde(default)]
        page: Option<PageRef>,
        label: String,
        ops: Vec<DesignOp>,
        /// When false, check the batch and roll it back.
        #[serde(default = "default_commit")]
        commit: bool,
    },
    AddComments {
        label: String,
        comments: Vec<CommentDraft>,
    },
    Project {
        page: PageRef,
        #[serde(default = "default_project_depth")]
        depth: u32,
        #[serde(default = "default_project_limit")]
        limit: usize,
    },
    Render {
        targets: Vec<RenderTarget>,
    },
    Changeset {
        /// Include each changed file's content.
        #[serde(default)]
        contents: bool,
        /// Include the operation journal the app replays.
        #[serde(default)]
        ops: bool,
    },
}

fn default_commit() -> bool {
    true
}

fn default_project_depth() -> u32 {
    8
}

fn default_project_limit() -> usize {
    50
}

impl Step {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Inspect {} => "inspect",
            Self::CreatePage { .. } => "create_page",
            Self::Apply { .. } => "apply",
            Self::AddComments { .. } => "add_comments",
            Self::Project { .. } => "project",
            Self::Render { .. } => "render",
            Self::Changeset { .. } => "changeset",
        }
    }
}

/// What `GET /healthz` answers, and the `engine` of every run response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub version: String,
    pub git_sha: String,
    /// The newest project schema this engine reads.
    pub schema_version: u32,
    /// `step:<name>` for every step this build runs.
    pub capabilities: Vec<String>,
}

impl EngineInfo {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            git_sha: option_env!("FANTA_ENGINE_GIT_SHA")
                .unwrap_or("unknown")
                .to_owned(),
            schema_version: fanta_doc::SCHEMA_VERSION,
            capabilities: [
                "inspect",
                "create_page",
                "add_comments",
                "project",
                "render",
                "changeset",
            ]
            .into_iter()
            .map(|step| format!("step:{step}"))
            .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResponse {
    pub request_id: String,
    pub engine: EngineInfo,
    pub source: SourceInfo,
    /// One entry per step that ran. A failed step is the last entry: the run
    /// stops there.
    pub results: Vec<StepResult>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// The commit the run read; `None` for a `dir` source.
    pub sha: Option<String>,
    /// The project's `fanta.json` `project_id`.
    pub project_id: Option<String>,
    pub warnings: Vec<String>,
    pub fonts: FontReport,
}

/// Font families the project names that this engine does not have.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FontReport {
    pub missing: Vec<String>,
    pub substituted: Vec<FontSubstitution>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSubstitution {
    pub family: String,
    pub used: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepResult {
    pub step: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    /// The repository, commit or project folder does not exist, or the token
    /// cannot see it.
    NotFound,
    /// The project was saved by a newer Fanta than this engine.
    SchemaTooNew,
    PayloadTooLarge,
    ProjectLoadFailed,
    Busy,
    DeadlineExceeded,
    Internal,
    /// A step ran and failed; the run stopped there. Only in step results.
    StepFailed,
}

impl ErrorCode {
    /// The HTTP status a request-level error answers with.
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::SchemaTooNew => 409,
            Self::PayloadTooLarge => 413,
            Self::ProjectLoadFailed => 422,
            Self::Busy => 429,
            Self::DeadlineExceeded => 504,
            Self::Internal | Self::StepFailed => 500,
        }
    }
}

/// A request-level failure: the body of a non-200 response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineError {
    pub error: ErrorBody,
}

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ErrorBody {
                code,
                message: message.into(),
            },
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} ({}): {}",
            self.error.code,
            self.error.code.status(),
            self.error.message
        )
    }
}

impl std::error::Error for EngineError {}

/// Check what can be checked before the project is read.
pub fn validate(request: &RunRequest) -> Result<(), EngineError> {
    let bad_request = |message: String| Err(EngineError::new(ErrorCode::BadRequest, message));
    if request.deadline_ms == 0 || request.deadline_ms > MAX_DEADLINE_MS {
        return bad_request(format!(
            "deadline_ms must be between 1 and {MAX_DEADLINE_MS}"
        ));
    }
    if request.steps.is_empty() {
        return bad_request("a run needs at least one step".into());
    }
    for (index, step) in request.steps.iter().enumerate() {
        if let Step::Render { targets } = step
            && (targets.is_empty() || targets.len() > MAX_RENDER_TARGETS)
        {
            return bad_request(format!(
                "step {index} renders {} targets; a render step takes 1 to {MAX_RENDER_TARGETS}",
                targets.len()
            ));
        }
    }
    if let Source::Github {
        overlay: Some(overlay),
        ..
    } = &request.source
    {
        let mut total = 0;
        for file in &overlay.files {
            match (&file.content_b64, file.deleted) {
                (Some(content), false) => {
                    total += content.len() / 4 * 3;
                }
                (None, true) => {}
                _ => {
                    return bad_request(format!(
                        "overlay file {:?} needs either content_b64 or deleted: true",
                        file.path
                    ));
                }
            }
        }
        if total > MAX_OVERLAY_BYTES {
            return Err(EngineError::new(
                ErrorCode::PayloadTooLarge,
                format!("the overlay exceeds {MAX_OVERLAY_BYTES} bytes"),
            ));
        }
    }
    Ok(())
}

/// Run `request` against a project folder the caller already prepared: a
/// private copy the run may write to.
pub fn run_in(request: &RunRequest, work_dir: &Path) -> Result<RunResponse, EngineError> {
    validate(request)?;
    let started = Instant::now();
    let deadline = Duration::from_millis(request.deadline_ms);
    let mut project = HeadlessProject::open(work_dir)
        .map_err(|error| EngineError::new(ErrorCode::ProjectLoadFailed, format!("{error:#}")))?;
    let mut results = Vec::with_capacity(request.steps.len());
    for step in &request.steps {
        if started.elapsed() >= deadline {
            results.push(StepResult {
                step: step.name().into(),
                ok: false,
                result: None,
                error: Some(ErrorBody {
                    code: ErrorCode::DeadlineExceeded,
                    message: "the run's deadline passed before this step started".into(),
                }),
            });
            break;
        }
        match run_step(&mut project, step) {
            Ok(result) => results.push(StepResult {
                step: step.name().into(),
                ok: true,
                result: Some(result),
                error: None,
            }),
            Err(error) => {
                results.push(StepResult {
                    step: step.name().into(),
                    ok: false,
                    result: None,
                    error: Some(ErrorBody {
                        code: ErrorCode::StepFailed,
                        message: format!("{error:#}"),
                    }),
                });
                break;
            }
        }
    }
    Ok(RunResponse {
        request_id: request.request_id.clone(),
        engine: EngineInfo::current(),
        source: SourceInfo {
            sha: None,
            project_id: Some(project.doc().id.to_string()),
            warnings: project.warnings().to_vec(),
            fonts: FontReport::default(),
        },
        results,
    })
}

fn run_step(project: &mut HeadlessProject, step: &Step) -> anyhow::Result<serde_json::Value> {
    match step {
        Step::Inspect {} => project.inspect(),
        Step::CreatePage { name } => {
            let root = project.create_page(name)?;
            Ok(json!({ "page": { "id": root.to_string() } }))
        }
        Step::Apply {
            page,
            label,
            ops,
            commit,
        } => {
            let page = page
                .as_ref()
                .map(|page| project.resolve_page(page))
                .transpose()?;
            project.apply(page, label, ops, *commit)
        }
        Step::AddComments { label, comments } => {
            let outcomes = project.add_comments(label, comments)?;
            Ok(json!({
                "comments": outcomes
                    .into_iter()
                    .map(|outcome| match outcome {
                        Ok(id) => json!({ "ok": true, "id": id }),
                        Err(error) => json!({ "ok": false, "error": format!("{error:#}") }),
                    })
                    .collect::<Vec<_>>(),
            }))
        }
        Step::Project { page, depth, limit } => {
            let root = project.resolve_page(page)?;
            Ok(json!({ "nodes": project.project_page(root, *depth, *limit)? }))
        }
        Step::Render { targets } => {
            let mut images = Vec::with_capacity(targets.len());
            for target in targets {
                let image = project.render(target)?;
                images.push(json!({
                    "target_id": image.target_id,
                    "png_b64": base64::engine::general_purpose::STANDARD.encode(&image.png),
                    "width": image.width,
                    "height": image.height,
                    "bounds": image.bounds,
                }));
            }
            Ok(json!({ "images": images }))
        }
        Step::Changeset { contents, ops } => {
            project.save()?;
            let files = fanta_headless::changeset(project.loaded_hashes(), project.root())?;
            let mut listed = Vec::with_capacity(files.len());
            for file in files {
                let mut entry = serde_json::to_value(&file)?;
                if *contents && file.sha256.is_some() {
                    let bytes = std::fs::read(project.root().join(&file.path))?;
                    entry["content_b64"] =
                        json!(base64::engine::general_purpose::STANDARD.encode(bytes));
                }
                listed.push(entry);
            }
            let mut result = json!({ "files": listed });
            if *ops {
                result["ops"] = serde_json::to_value(project.journal())?;
            }
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: serde_json::Value) -> RunRequest {
        serde_json::from_value(value).expect("the request parses")
    }

    #[test]
    fn a_review_run_parses_every_step_of_the_contract() {
        let parsed = request(json!({
            "request_id": "rev_1",
            "deadline_ms": 120000,
            "source": {
                "kind": "github",
                "repo": "acme/site",
                "sha": "0123456789abcdef0123456789abcdef01234567",
                "path": "design",
                "token": "ghs_secret",
                "overlay": {"files": [
                    {"path": "pages/home/page.fnx", "content_b64": "PFBhZ2UvPg=="},
                    {"path": "pages/old/page.fnx", "deleted": true}
                ]}
            },
            "steps": [
                {"step": "inspect"},
                {"step": "create_page", "name": "Review"},
                {"step": "apply", "page": {"created": 0}, "label": "Add a frame", "ops": [
                    {"op": "create_node", "node_type": "frame", "x": 0, "y": 0, "width": 10, "height": 10}
                ]},
                {"step": "add_comments", "label": "AI Review", "comments": [
                    {"page": 0, "node": "n_01J0000000000000000000000A", "text": "**Contrast**\n\nToo low.",
                     "author": "Fanta Review", "severity": "major", "category": "accessibility"}
                ]},
                {"step": "project", "page": {"index": 0}},
                {"step": "render", "targets": [{"id": "home", "page": {"index": 0}}]},
                {"step": "changeset", "contents": true, "ops": true}
            ]
        }));
        let names: Vec<_> = parsed.steps.iter().map(Step::name).collect();
        assert_eq!(
            names,
            [
                "inspect",
                "create_page",
                "apply",
                "add_comments",
                "project",
                "render",
                "changeset"
            ]
        );
        assert!(matches!(parsed.steps[2], Step::Apply { commit: true, .. }));
        assert!(matches!(
            parsed.steps[4],
            Step::Project {
                depth: 8,
                limit: 50,
                ..
            }
        ));
        validate(&parsed).expect("the request is valid");
        assert!(!format!("{parsed:?}").contains("ghs_secret"));
    }

    #[test]
    fn unknown_steps_and_fields_are_refused() {
        for value in [
            json!({"step": "publish"}),
            json!({"step": "inspect", "page": 0}),
            json!({"step": "render", "targets": [], "format": "jpeg"}),
        ] {
            assert!(
                serde_json::from_value::<Step>(value.clone()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn validation_bounds_deadlines_renders_and_overlays() {
        let base = json!({
            "request_id": "r",
            "deadline_ms": 1000,
            "source": {"kind": "dir", "dir": "/tmp/project"},
            "steps": [{"step": "inspect"}]
        });
        let code =
            |value: serde_json::Value| validate(&request(value)).map_err(|error| error.error.code);
        assert_eq!(code(base.clone()), Ok(()));

        let mut late = base.clone();
        late["deadline_ms"] = json!(MAX_DEADLINE_MS + 1);
        assert_eq!(code(late), Err(ErrorCode::BadRequest));

        let mut wide = base.clone();
        wide["steps"] = json!([{"step": "render", "targets": (0..9)
            .map(|index| json!({"id": index.to_string(), "page": {"index": 0}}))
            .collect::<Vec<_>>()}]);
        assert_eq!(code(wide), Err(ErrorCode::BadRequest));

        let mut ambiguous = base.clone();
        ambiguous["source"] = json!({"kind": "github", "repo": "a/b", "sha": "c", "token": "t",
            "overlay": {"files": [{"path": "fanta.json", "content_b64": "e30=", "deleted": true}]}});
        assert_eq!(code(ambiguous), Err(ErrorCode::BadRequest));

        let mut large = base;
        large["source"] = json!({"kind": "github", "repo": "a/b", "sha": "c", "token": "t",
            "overlay": {"files": [{"path": "big.bin",
                "content_b64": "A".repeat(MAX_OVERLAY_BYTES / 3 * 4 + 8)}]}});
        assert_eq!(code(large), Err(ErrorCode::PayloadTooLarge));
    }

    #[test]
    fn a_review_run_answers_every_step_on_a_real_project() {
        use fanta_doc::{
            CanvasNode, Doc, GroupNode, NodeData, Operation, Transform2D, VectorNode, color::Color,
        };
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Home".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        let mut card = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            120.0,
            80.0,
            Color::WHITE,
        )));
        card.name = "Card".into();
        card.parent = Some(page_id);
        card.transform = Transform2D::translation(10.0, 10.0);
        let card_id = card.id;
        doc.apply(Operation::create_node(card)).expect("card");
        let directory = tempfile::tempdir().expect("project directory");
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())
            .expect("write the project");

        let response = run_in(
            &request(json!({
                "request_id": "rev_2",
                "deadline_ms": 60000,
                "source": {"kind": "dir", "dir": directory.path()},
                "steps": [
                    {"step": "inspect"},
                    {"step": "add_comments", "label": "AI Review", "comments": [
                        {"page": 0, "node": card_id.to_string(), "text": "Too faint",
                         "author": "Fanta Review", "severity": "minor", "category": "color"}
                    ]},
                    {"step": "project", "page": {"index": 0}},
                    {"step": "render", "targets": [
                        {"id": "home", "page": {"index": 0}, "max_dimension": 200}
                    ]},
                    {"step": "changeset", "contents": true, "ops": true}
                ]
            })),
            directory.path(),
        )
        .expect("the run answers");
        assert!(
            response.results.iter().all(|result| result.ok),
            "{:?}",
            response.results
        );
        assert_eq!(response.source.project_id, Some(doc.id.to_string()));
        let result = |index: usize| {
            response.results[index]
                .result
                .clone()
                .expect("a successful step has a result")
        };
        assert_eq!(result(0)["pages"][0]["name"], "Home");
        assert_eq!(result(1)["comments"][0]["ok"], true);
        assert_eq!(result(2)["nodes"][0]["id"], card_id.to_string());
        let image = &result(3)["images"][0];
        assert_eq!(image["target_id"], "home");
        let png = base64::engine::general_purpose::STANDARD
            .decode(image["png_b64"].as_str().expect("png_b64"))
            .expect("base64");
        assert!(png.starts_with(b"\x89PNG"));
        let changeset = result(4);
        let page_file = changeset["files"]
            .as_array()
            .expect("files")
            .iter()
            .find(|file| file["path"] == "pages/home/page.fnx")
            .expect("the commented page changed");
        assert_eq!(page_file["status"], "modified");
        let source = base64::engine::general_purpose::STANDARD
            .decode(page_file["content_b64"].as_str().expect("content_b64"))
            .expect("base64");
        assert!(String::from_utf8_lossy(&source).contains("Too faint"));
        assert_eq!(changeset["ops"][0]["op"], "set_meta");
    }

    #[test]
    fn errors_serialize_with_their_contract_codes() {
        let error = EngineError::new(ErrorCode::SchemaTooNew, "saved by a newer Fanta");
        assert_eq!(
            serde_json::to_value(&error).expect("errors serialize"),
            json!({"error": {"code": "schema_too_new", "message": "saved by a newer Fanta"}})
        );
        assert_eq!(error.error.code.status(), 409);
    }
}
